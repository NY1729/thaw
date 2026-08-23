use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::raw::{c_char, c_void};

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
    CString::new(serve_once(port, |_, _| ResponseSpec::plain(body)).unwrap_or_default())
        .unwrap_or_default()
        .into_raw()
}

struct ResponseSpec {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl ResponseSpec {
    fn plain(body: String) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
            body,
        }
    }
}

fn serve_once(
    port: f64,
    response_for: impl FnOnce(&str, &str) -> ResponseSpec,
) -> Result<String, String> {
    if !port.is_finite() || port < 0.0 || port > u16::MAX as f64 {
        return Err("invalid port".into());
    }
    let listener = TcpListener::bind(("127.0.0.1", port as u16))
        .map_err(|error| format!("bind failed: {error}"))?;
    let (stream, _) = listener
        .accept()
        .map_err(|error| format!("accept failed: {error}"))?;
    handle_stream(stream, response_for)
}

fn handle_stream(
    mut stream: TcpStream,
    response_for: impl FnOnce(&str, &str) -> ResponseSpec,
) -> Result<String, String> {
    let mut request = [0_u8; 8192];
    let length = stream
        .read(&mut request)
        .map_err(|error| format!("request read failed: {error}"))?;
    let request = String::from_utf8_lossy(&request[..length]);
    let mut request_line = request.lines().next().unwrap_or("").split_whitespace();
    let method = request_line.next().unwrap_or("GET").to_string();
    let target = request_line.next().unwrap_or("/").to_string();
    let response_spec = response_for(&method, &target);
    let reason = match response_spec.status {
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let mut response = format!("HTTP/1.1 {} {}\r\n", response_spec.status, reason);
    for (name, value) in response_spec.headers {
        response.push_str(&name.replace(['\r', '\n'], ""));
        response.push_str(": ");
        response.push_str(&value.replace(['\r', '\n'], ""));
        response.push_str("\r\n");
    }
    response.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_spec.body.len(),
        response_spec.body
    ));
    stream
        .write_all(response.as_bytes())
        .map_err(|error| format!("response write failed: {error}"))?;
    Ok(target)
}

/// Invokes a Thaw closure with the request target and uses its returned string
/// as the response body. A closure is `[code pointer][captures...]`; generated
/// code accepts the closure address as its hidden first argument.
#[no_mangle]
pub extern "C" fn serveOnceWith(port: f64, callback: *const c_void) -> *const c_char {
    if callback.is_null() {
        return CString::default().into_raw();
    }
    let result = serve_once(port, |_, target| unsafe {
        type Callback = unsafe extern "C" fn(*const c_void, *const c_char) -> *const c_char;
        let code = *(callback as *const *const c_void);
        let callback_fn: Callback = std::mem::transmute(code);
        let target = CString::new(target).unwrap_or_default();
        ResponseSpec::plain(string_from_ptr(callback_fn(callback, target.as_ptr())))
    })
    .unwrap_or_default();
    CString::new(result).unwrap_or_default().into_raw()
}

#[repr(C)]
struct NativeClosure {
    code: *const c_void,
    context: *mut c_void,
}

struct ResponseState {
    headers: Vec<(String, String)>,
    body: String,
}

unsafe fn response_state(environment: *const c_void) -> &'static mut ResponseState {
    let closure = &*(environment as *const NativeClosure);
    &mut *(closure.context as *mut ResponseState)
}

unsafe extern "C" fn response_set_header(
    environment: *const c_void,
    name: *const c_char,
    value: *const c_char,
) -> bool {
    response_state(environment)
        .headers
        .push((string_from_ptr(name), string_from_ptr(value)));
    true
}

unsafe extern "C" fn response_write(environment: *const c_void, chunk: *const c_char) -> bool {
    response_state(environment)
        .body
        .push_str(&string_from_ptr(chunk));
    true
}

unsafe extern "C" fn response_end(environment: *const c_void, chunk: *const c_char) -> bool {
    response_write(environment, chunk)
}

#[repr(C)]
struct IncomingMessage {
    method: *const c_char,
    url: *const c_char,
}

#[repr(C)]
struct ServerResponse {
    status_code: f64,
    set_header: *const NativeClosure,
    write: *const NativeClosure,
    end: *const NativeClosure,
}

/// One-request native slice of Node's `createServer` callback shape.
#[no_mangle]
pub extern "C" fn createServerOnce(port: f64, callback: *const c_void) -> *const c_char {
    if callback.is_null() {
        return CString::default().into_raw();
    }
    CString::new(run_server(port, callback))
        .unwrap_or_default()
        .into_raw()
}

fn run_server(port: f64, callback: *const c_void) -> String {
    serve_once(port, |method, target| {
        invoke_server_callback(callback, method, target)
    })
    .unwrap_or_default()
}

fn invoke_server_callback(callback: *const c_void, method: &str, target: &str) -> ResponseSpec {
    unsafe {
        type Callback = unsafe extern "C" fn(
            *const c_void,
            *const IncomingMessage,
            *mut ServerResponse,
        ) -> bool;
        let mut state = ResponseState {
            headers: Vec::new(),
            body: String::new(),
        };
        let state_ptr = &mut state as *mut ResponseState;
        let set_header = NativeClosure {
            code: response_set_header as *const c_void,
            context: state_ptr.cast(),
        };
        let write = NativeClosure {
            code: response_write as *const c_void,
            context: state_ptr.cast(),
        };
        let end = NativeClosure {
            code: response_end as *const c_void,
            context: state_ptr.cast(),
        };
        let method = CString::new(method).unwrap_or_default();
        let target_string = CString::new(target).unwrap_or_default();
        let request = IncomingMessage {
            method: method.as_ptr(),
            url: target_string.as_ptr(),
        };
        let mut response = ServerResponse {
            status_code: 200.0,
            set_header: &set_header,
            write: &write,
            end: &end,
        };
        let code = *(callback as *const *const c_void);
        let callback_fn: Callback = std::mem::transmute(code);
        callback_fn(callback, &request, &mut response);
        ResponseSpec {
            status: if response.status_code.is_finite()
                && response.status_code >= 100.0
                && response.status_code <= 999.0
            {
                response.status_code as u16
            } else {
                500
            },
            headers: state.headers,
            body: state.body,
        }
    }
}

fn run_server_many(port: f64, callback: *const c_void, count: usize) -> String {
    if !port.is_finite() || port < 0.0 || port > u16::MAX as f64 {
        return String::new();
    }
    let Ok(listener) = TcpListener::bind(("127.0.0.1", port as u16)) else {
        return String::new();
    };
    let mut last_target = String::new();
    for _ in 0..count {
        let Ok((stream, _)) = listener.accept() else {
            return String::new();
        };
        match handle_stream(stream, |method, target| {
            invoke_server_callback(callback, method, target)
        }) {
            Ok(target) => last_target = target,
            Err(_) => return String::new(),
        }
    }
    last_target
}

struct ServerState {
    callback: *const c_void,
}

#[repr(C)]
struct Server {
    listen: *const NativeClosure,
    listen_many: *const NativeClosure,
}

unsafe extern "C" fn server_listen(environment: *const c_void, port: f64) -> *const c_char {
    let closure = &*(environment as *const NativeClosure);
    let state = &*(closure.context as *const ServerState);
    CString::new(run_server_many(port, state.callback, usize::MAX))
        .unwrap_or_default()
        .into_raw()
}

unsafe extern "C" fn server_listen_many(
    environment: *const c_void,
    port: f64,
    count: f64,
) -> *const c_char {
    let closure = &*(environment as *const NativeClosure);
    let state = &*(closure.context as *const ServerState);
    let count = if count.is_finite() && count >= 0.0 {
        count as usize
    } else {
        0
    };
    CString::new(run_server_many(port, state.callback, count))
        .unwrap_or_default()
        .into_raw()
}

/// Node-shaped constructor slice: returns an object with `listen(port)`.
#[no_mangle]
pub extern "C" fn createServer(callback: *const c_void) -> *const c_void {
    if callback.is_null() {
        return std::ptr::null();
    }
    let state = Box::into_raw(Box::new(ServerState { callback }));
    let listen = Box::into_raw(Box::new(NativeClosure {
        code: server_listen as *const c_void,
        context: state.cast(),
    }));
    let listen_many = Box::into_raw(Box::new(NativeClosure {
        code: server_listen_many as *const c_void,
        context: state.cast(),
    }));
    Box::into_raw(Box::new(Server {
        listen,
        listen_many,
    }))
    .cast()
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
