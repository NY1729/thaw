//! A minimal AWS Lambda custom runtime client. This is what turns a
//! compiled Thaw program with a `function handler(event: string): string`
//! into an actual deployable Lambda function: it polls the Runtime API for
//! the next invocation, calls the handler with the raw event JSON, and
//! posts the handler's raw string result back.
//!
//! Deliberately a hand-rolled blocking HTTP/1.1 client over `TcpStream`
//! rather than hyper/tokio: the Runtime API is a fixed, local-only, plain
//! HTTP endpoint with no concurrency to speak of (one invocation at a time
//! per execution environment), and pulling in a full async runtime here
//! would fight the entire point of Thaw -- small binaries, fast cold
//! start. `fetch()` for user code is a separate, much bigger problem
//! (arbitrary hosts, TLS, redirects) and isn't this crate's job; it's
//! deferred along with async/await.
//!
//! Known limitations, acceptable for what this talks to: no TLS, no
//! chunked transfer-encoding, one request per TCP connection.

use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::raw::c_char;

pub type HandlerFn = extern "C" fn(*const c_char) -> *const c_char;

/// Runs the event loop forever. Compiled by `thaw-llvm::hir_codegen` as the
/// process entry point whenever a program defines `handler` instead of
/// `main` (see that module for the `int main(void)` wrapper that calls
/// this).
#[no_mangle]
pub extern "C" fn thaw_runtime_run(handler: HandlerFn) -> ! {
    let runtime_api = std::env::var("AWS_LAMBDA_RUNTIME_API").expect(
        "AWS_LAMBDA_RUNTIME_API is not set -- is this running inside a Lambda execution environment?",
    );

    loop {
        if let Err(err) = handle_one_invocation(&runtime_api, handler) {
            eprintln!("thaw-runtime: {err}");
        }
    }
}

fn handle_one_invocation(runtime_api: &str, handler: HandlerFn) -> Result<(), String> {
    let next = http_request(runtime_api, "GET", "/2018-06-01/runtime/invocation/next", None)?;

    let request_id = next
        .header("lambda-runtime-aws-request-id")
        .ok_or("response from .../invocation/next is missing the request id header")?
        .to_string();

    let event_cstring = CString::new(next.body).map_err(|e| e.to_string())?;
    let result_ptr = handler(event_cstring.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();

    let response_path = format!("/2018-06-01/runtime/invocation/{request_id}/response");
    http_request(runtime_api, "POST", &response_path, Some(&result))?;
    Ok(())
}

struct HttpResponse {
    status: u32,
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn http_request(
    host: &str,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<HttpResponse, String> {
    let mut stream = TcpStream::connect(host).map_err(|e| format!("connecting to {host}: {e}"))?;

    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(body) = body {
        request.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    request.push_str("\r\n");
    if let Some(body) = body {
        request.push_str(body);
    }

    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("writing request: {e}"))?;
    // Half-close the write side so the peer's `read_to_end` (used for
    // request bodies on our mock server, and in principle by any HTTP/1.1
    // peer reading an unbounded body) sees EOF instead of blocking forever
    // waiting for more of *our* request -- otherwise both sides can end up
    // blocked in `read_to_end` at once.
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("shutting down write half: {e}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("reading response: {e}"))?;
    let raw = String::from_utf8_lossy(&raw);

    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or("malformed HTTP response (no header/body separator)")?;

    let mut lines = head.lines();
    let status_line = lines.next().ok_or("empty HTTP response")?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| format!("malformed status line: {status_line}"))?;

    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();

    let response = HttpResponse {
        status,
        headers,
        body: body.to_string(),
    };

    if response.status >= 400 {
        return Err(format!(
            "{method} {path} -> HTTP {}: {}",
            response.status, response.body
        ));
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;

    extern "C" fn echo_handler(event: *const c_char) -> *const c_char {
        let event = unsafe { CStr::from_ptr(event) }.to_string_lossy().into_owned();
        // Leaked on purpose: matches the arena/global-lifetime string model
        // compiled Thaw code uses (nothing frees heap strings yet).
        CString::new(format!("echo:{event}")).unwrap().into_raw() as *const c_char
    }

    #[test]
    fn polls_an_invocation_and_posts_the_handler_result() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = conn.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.starts_with("GET /2018-06-01/runtime/invocation/next"));

            let body = "\"hello\"";
            let response = format!(
                "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: req-123\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            conn.write_all(response.as_bytes()).unwrap();
            drop(conn);

            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            conn.read_to_end(&mut buf).unwrap();
            tx.send(String::from_utf8_lossy(&buf).into_owned()).unwrap();

            let response = "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            conn.write_all(response.as_bytes()).unwrap();
        });

        handle_one_invocation(&addr, echo_handler).unwrap();
        server.join().unwrap();

        let post_request = rx.recv().unwrap();
        assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/req-123/response"));
        assert!(post_request.ends_with("echo:\"hello\""));
    }

    #[test]
    fn surfaces_http_error_status_as_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = conn.read(&mut buf).unwrap();
            let body = "boom";
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            conn.write_all(response.as_bytes()).unwrap();
        });

        let err = handle_one_invocation(&addr, echo_handler).unwrap_err();
        assert!(err.contains("500"));
        server.join().unwrap();
    }
}
