//! `fetch(url: string): string` (V1, see docs/design/async-await.md: this
//! is a plain blocking call under the hood, same as everything else in V1
//! async/await). Returns the response body as a string; no headers,
//! methods other than GET, or request bodies yet -- deferred until there's
//! a real use case driving the design.
//!
//! Uses `ureq` (blocking, no async runtime) rather than the design doc's
//! `hyper`: hyper 1.x needs an async executor to drive it, and V1's
//! async/await has no real executor (see thaw-runtime's own rationale for
//! not using hyper/tokio in the Lambda poll loop -- same reasoning here).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

#[no_mangle]
pub extern "C" fn thaw_fetch_get(url: *const c_char) -> *const c_char {
    let url = unsafe { CStr::from_ptr(url) }.to_string_lossy().into_owned();

    let body = match ureq::get(&url).call() {
        Ok(mut response) => response
            .body_mut()
            .read_to_string()
            .unwrap_or_else(|e| format!("fetch error reading body: {e}")),
        Err(e) => format!("fetch error: {e}"),
    };

    CString::new(body).unwrap_or_default().into_raw() as *const c_char
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn fetches_a_body_over_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = conn.read(&mut buf).unwrap();
            let body = "hello from mock server";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            conn.write_all(response.as_bytes()).unwrap();
        });

        let url = CString::new(format!("http://{addr}/")).unwrap();
        let result_ptr = thaw_fetch_get(url.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned();

        server.join().unwrap();
        assert_eq!(result, "hello from mock server");
    }
}
