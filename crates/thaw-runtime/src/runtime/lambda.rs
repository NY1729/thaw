/// Reclaims request-owned generated values on every exit path. The handler
/// result is copied into a Rust `String` before this guard is dropped, so the
/// response never borrows reclaimed arena memory.
struct InvocationArenaReset;

impl Drop for InvocationArenaReset {
    fn drop(&mut self) {
        thaw_runtime_drain_detached();
        // Anything still pending here belongs to work this invocation is
        // abandoning (a deadline timeout, or a genuine deadlock with no
        // possible progress) rather than work `drain_detached` already
        // finished. Its frames live in the arena reset below, so leaving it
        // registered would let a timer or fd event fire during a later
        // invocation and resume a pointer into since-reused memory.
        purge_pending_async_state();
        thaw_arena::thaw_arena_reset();
    }
}

/// Runs the event loop forever. Compiled by `thaw-llvm::hir_codegen` as the
/// process entry point whenever a program defines `handler` instead of
/// `main` (see that module for the `int main(void)` wrapper that calls
/// this).
#[no_mangle]
pub extern "C" fn thaw_runtime_run(handler: HandlerFn, error_slot: HandlerErrorSlot) -> ! {
    let runtime_api = std::env::var("AWS_LAMBDA_RUNTIME_API").expect(
        "AWS_LAMBDA_RUNTIME_API is not set -- is this running inside a Lambda execution environment?",
    );

    let mut consecutive_failures = 0u32;
    loop {
        match handle_one_invocation(&runtime_api, handler, error_slot) {
            Ok(()) => consecutive_failures = 0,
            Err(err) => {
                eprintln!("thaw-runtime: {err}");
                consecutive_failures += 1;
                match failure_backoff(consecutive_failures) {
                    Some(delay) => std::thread::sleep(delay),
                    None => {
                        // An unreachable Runtime API will not recover by
                        // retrying forever; like the managed runtimes, exit
                        // non-zero so the environment is replaced.
                        eprintln!(
                            "thaw-runtime: giving up after {consecutive_failures} consecutive failures"
                        );
                        std::process::exit(1);
                    }
                }
            }
        }
    }
}

/// Maximum number of consecutive invocation-loop failures before the runtime exits.
const MAX_CONSECUTIVE_FAILURES: u32 = 20;

/// Delay before retrying after the `consecutive`-th failure in a row (25 ms doubling to
/// 500 ms, about 8 s in total), or `None` once the runtime should give up. A short bounded
/// retry still covers a Runtime API that is not listening yet at process start.
fn failure_backoff(consecutive: u32) -> Option<std::time::Duration> {
    if consecutive >= MAX_CONSECUTIVE_FAILURES {
        return None;
    }
    let millis = (25u64 << (consecutive - 1).min(5)).min(500);
    Some(std::time::Duration::from_millis(millis))
}

#[cfg(test)]
mod failure_backoff_tests {
    use super::{failure_backoff, MAX_CONSECUTIVE_FAILURES};
    use std::time::Duration;

    #[test]
    fn retries_back_off_then_give_up() {
        assert_eq!(failure_backoff(1), Some(Duration::from_millis(25)));
        assert_eq!(failure_backoff(2), Some(Duration::from_millis(50)));
        assert_eq!(failure_backoff(5), Some(Duration::from_millis(400)));
        assert_eq!(failure_backoff(6), Some(Duration::from_millis(500)));
        assert_eq!(failure_backoff(MAX_CONSECUTIVE_FAILURES - 1), Some(Duration::from_millis(500)));
        assert_eq!(failure_backoff(MAX_CONSECUTIVE_FAILURES), None);
        let total: Duration = (1..MAX_CONSECUTIVE_FAILURES).filter_map(failure_backoff).sum();
        assert!(total < Duration::from_secs(10), "bounded total delay: {total:?}");
    }
}

fn handle_one_invocation(
    runtime_api: &str,
    handler: HandlerFn,
    error_slot: HandlerErrorSlot,
) -> Result<(), String> {
    let _arena_reset = InvocationArenaReset;
    let next = http_request(
        runtime_api,
        "GET",
        "/2018-06-01/runtime/invocation/next",
        None,
    )?;

    let request_id = next
        .header("lambda-runtime-aws-request-id")
        .ok_or("response from .../invocation/next is missing the request id header")?
        .to_string();

    // The Lambda Runtime API supplies the X-Ray parent and sampling decision
    // per invocation. A warm environment must not retain the previous trace
    // when the next response has no trace header.
    match next.header("lambda-runtime-trace-id") {
        Some(trace_id) if trace_id.contains('\0') => {
            return Err("response from .../invocation/next has an invalid trace id header".into());
        }
        Some(trace_id) => std::env::set_var("_X_AMZN_TRACE_ID", trace_id),
        None => std::env::remove_var("_X_AMZN_TRACE_ID"),
    }

    // `Lambda-Runtime-Deadline-Ms` is an absolute epoch timestamp; convert it
    // to a countdown so the event-loop drivers in lib.rs only need a
    // monotonic clock. A missing/unparsable header (e.g. a local test
    // harness that never sent one) leaves no deadline active, matching
    // today's behavior of running the handler to completion.
    let deadline_remaining_ms = next
        .header("lambda-runtime-deadline-ms")
        .and_then(|value| value.parse::<u64>().ok())
        .map(|deadline_epoch_ms| {
            let now_epoch_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as u64)
                .unwrap_or(0);
            deadline_epoch_ms.saturating_sub(now_epoch_ms)
        });
    set_invocation_deadline(deadline_remaining_ms);

    let event_cstring = CString::new(next.body).map_err(|e| e.to_string())?;
    if !error_slot.is_null() {
        unsafe { *error_slot = std::ptr::null() };
    }
    let result_ptr = handler(event_cstring.as_ptr());
    if result_ptr.is_null() {
        let (error_type, message) = if error_slot.is_null() || unsafe { (*error_slot).is_null() }
        {
            (
                "ThawError".to_string(),
                "handler failed with an uncaught Thaw exception".to_string(),
            )
        } else {
            let raw = unsafe { CStr::from_ptr(*error_slot) }.to_string_lossy();
            let (_, message) = split_error_tag(&raw);
            (resolved_error_name(&raw), message.to_string())
        };
        let error_path = format!("/2018-06-01/runtime/invocation/{request_id}/error");
        let body = format!(
            "{{\"errorMessage\":{},\"errorType\":{}}}",
            json_string(&message),
            json_string(&error_type)
        );
        http_request(runtime_api, "POST", &error_path, Some(&body))?;
        return Ok(());
    }
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();

    let response_path = format!("/2018-06-01/runtime/invocation/{request_id}/response");
    http_request(runtime_api, "POST", &response_path, Some(&result))?;
    Ok(())
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
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
    let header_end = find_bytes(&raw, b"\r\n\r\n")
        .ok_or("malformed HTTP response (no header/body separator)")?;
    let head = String::from_utf8_lossy(&raw[..header_end]);
    let body = &raw[header_end + 4..];

    let mut lines = head.lines();
    let status_line = lines.next().ok_or("empty HTTP response")?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| format!("malformed status line: {status_line}"))?;

    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();

    let chunked = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
    });
    let body = if chunked {
        decode_chunked(body)?.ok_or("incomplete chunked HTTP response")?
    } else {
        body.to_vec()
    };

    let response = HttpResponse {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    };

    if response.status >= 400 {
        return Err(format!(
            "{method} {path} -> HTTP {}: {}",
            response.status, response.body
        ));
    }

    Ok(response)
}
