use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::raw::c_char;

fn string_from_ptr(value: *const c_char) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

#[no_mangle]
pub extern "C" fn serveOnce(port: f64, body: *const c_char) -> *const c_char {
    let body = string_from_ptr(body);
    let result = (|| -> Result<String, String> {
        if !port.is_finite() || port < 0.0 || port > u16::MAX as f64 {
            return Err("invalid port".into());
        }
        let listener = TcpListener::bind(("127.0.0.1", port as u16))
            .map_err(|error| format!("bind failed: {error}"))?;
        let (mut stream, _) = listener
            .accept()
            .map_err(|error| format!("accept failed: {error}"))?;
        let mut request = [0_u8; 8192];
        let length = stream
            .read(&mut request)
            .map_err(|error| format!("request read failed: {error}"))?;
        let request = String::from_utf8_lossy(&request[..length]);
        let target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/")
            .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        );
        stream
            .write_all(response.as_bytes())
            .map_err(|error| format!("response write failed: {error}"))?;
        Ok(target)
    })()
    .unwrap_or_default();
    CString::new(result).unwrap_or_default().into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn serves_one_http_request_and_returns_its_target() {
        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let server = thread::spawn(move || {
            let body = CString::new("hello thaw").unwrap();
            let target = serveOnce(port as f64, body.as_ptr());
            unsafe { CStr::from_ptr(target) }
                .to_string_lossy()
                .into_owned()
        });
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => thread::sleep(Duration::from_millis(5)),
            }
        };
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.ends_with("hello thaw"));
        assert_eq!(server.join().unwrap(), "/health");
    }
}
