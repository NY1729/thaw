use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::raw::{c_char, c_void};
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
use thaw_runtime as _;

unsafe extern "C" {
    fn thaw_runtime_watch_fd(
        fd: i32,
        interests: u8,
        callback: extern "C" fn(*mut u8, i16),
        context: *mut u8,
    ) -> u64;
    fn thaw_runtime_unwatch_fd(id: u64) -> bool;
    fn thaw_runtime_run_one_event() -> bool;
}

const THAW_FD_READABLE: u8 = 1;
const THAW_FD_WRITABLE: u8 = 2;
const MAX_REQUEST_HEAD: usize = 64 * 1024;

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
    body: Vec<u8>,
}

impl ResponseSpec {
    fn plain(body: String) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
            body: body.into_bytes(),
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
    let response = render_response(response_for(&method, &target));
    stream
        .write_all(&response)
        .map_err(|error| format!("response write failed: {error}"))?;
    Ok(target)
}

fn render_response(response_spec: ResponseSpec) -> Vec<u8> {
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
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        response_spec.body.len()
    ));
    let mut response = response.into_bytes();
    response.extend(response_spec.body);
    response
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
    body: Vec<u8>,
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
        .extend(string_from_ptr(chunk).as_bytes());
    true
}

unsafe extern "C" fn response_end_encoded(
    environment: *const c_void,
    content: *const c_char,
    encoding: *const c_char,
) -> bool {
    let content = string_from_ptr(content);
    if string_from_ptr(encoding) != "hex" {
        response_state(environment).body.extend(content.as_bytes());
        return true;
    }
    if !content.len().is_multiple_of(2) {
        return false;
    }
    let decoded = (0..content.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&content[index..index + 2], 16))
        .collect::<Result<Vec<_>, _>>();
    let Ok(decoded) = decoded else {
        return false;
    };
    response_state(environment).body.extend(decoded);
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
    end: *const NativeClosure,
    write: *const NativeClosure,
    end_encoded: *const NativeClosure,
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
            body: Vec::new(),
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
        let end_encoded = NativeClosure {
            code: response_end_encoded as *const c_void,
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
            end: &end,
            write: &write,
            end_encoded: &end_encoded,
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

fn run_server_many(port: f64, state: &ServerState, count: usize) -> String {
    if !port.is_finite() || port < 0.0 || port > u16::MAX as f64 {
        return String::new();
    }
    let Ok(listener) = TcpListener::bind(("127.0.0.1", port as u16)) else {
        return String::new();
    };
    if listener.set_nonblocking(true).is_err() {
        return String::new();
    }
    let mut last_target = String::new();
    for _ in 0..count {
        let stream = loop {
            if state.closed.load(Ordering::Acquire) {
                return last_target;
            }
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(_) => return String::new(),
            }
        };
        match handle_stream(stream, |method, target| {
            invoke_server_callback(state.callback as *const c_void, method, target)
        }) {
            Ok(target) => last_target = target,
            Err(_) => return String::new(),
        }
    }
    last_target
}

struct ServerState {
    callback: usize,
    closed: AtomicBool,
    listener: Mutex<Option<TcpListener>>,
    watcher: AtomicU64,
    connections: AtomicUsize,
    listening_listeners: Mutex<Vec<EventListener>>,
    close_listeners: Mutex<Vec<EventListener>>,
    error_listeners: Mutex<Vec<EventListener>>,
    pending_listening: AtomicBool,
    pending_errors: Mutex<Vec<ServerError>>,
    close_requested: AtomicBool,
}

#[derive(Clone, Copy)]
struct EventListener {
    callback: usize,
    once: bool,
}

fn active_servers() -> &'static Mutex<Vec<usize>> {
    static SERVERS: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
    SERVERS.get_or_init(|| Mutex::new(Vec::new()))
}

static ACTIVE_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);
static UNHANDLED_SERVER_ERROR: AtomicBool = AtomicBool::new(false);

struct ConnectionState {
    stream: TcpStream,
    server: *const ServerState,
    request: Vec<u8>,
    response: Vec<u8>,
    written: usize,
    watcher: u64,
}

fn register_server(state: *const ServerState) {
    let address = state as usize;
    let mut servers = active_servers().lock().unwrap();
    if !servers.contains(&address) {
        servers.push(address);
    }
}

/// Drives all listeners registered by `Server.listen`. Generated native entry
/// points call this after the user's main function has returned.
#[no_mangle]
pub extern "C" fn thaw_http_run_servers() {
    loop {
        {
            let mut servers = active_servers().lock().unwrap();
            servers.retain(|address| {
                let state = unsafe { &*(*address as *const ServerState) };
                if state.pending_listening.swap(false, Ordering::AcqRel) {
                    emit_event(&state.listening_listeners);
                }
                let errors = std::mem::take(&mut *state.pending_errors.lock().unwrap());
                for error in errors {
                    emit_error(&state.error_listeners, &error);
                }
                if state.closed.load(Ordering::Acquire) {
                    let watcher = state.watcher.swap(0, Ordering::AcqRel);
                    if watcher != 0 {
                        unsafe { thaw_runtime_unwatch_fd(watcher) };
                    }
                    *state.listener.lock().unwrap() = None;
                    if state.connections.load(Ordering::Acquire) == 0 {
                        if state.close_requested.swap(false, Ordering::AcqRel) {
                            emit_event(&state.close_listeners);
                        }
                        false
                    } else {
                        true
                    }
                } else {
                    true
                }
            });
            if servers.is_empty() && ACTIVE_CONNECTIONS.load(Ordering::Acquire) == 0 {
                return;
            }
        }
        if !unsafe { thaw_runtime_run_one_event() } {
            return;
        }
    }
}

fn emit_event(listeners: &Mutex<Vec<EventListener>>) {
    let callbacks = {
        let mut listeners = listeners.lock().unwrap();
        let callbacks = listeners.clone();
        listeners.retain(|listener| !listener.once);
        callbacks
    };
    for listener in callbacks {
        let callback = listener.callback as *const c_void;
        unsafe {
            type Callback = unsafe extern "C" fn(*const c_void);
            let code = *(callback as *const *const c_void);
            let callback_fn: Callback = std::mem::transmute(code);
            callback_fn(callback);
        }
    }
}

fn add_event_listener(listeners: &Mutex<Vec<EventListener>>, callback: *const c_void, once: bool) {
    if !callback.is_null() {
        listeners.lock().unwrap().push(EventListener {
            callback: callback as usize,
            once,
        });
    }
}

#[derive(Clone)]
struct ServerError {
    message: String,
    code: String,
    port: f64,
}

#[repr(C)]
struct NativeServerError {
    message: *const c_char,
    code: *const c_char,
    syscall: *const c_char,
    address: *const c_char,
    port: f64,
}

fn emit_error(listeners: &Mutex<Vec<EventListener>>, error: &ServerError) {
    let callbacks = listeners.lock().unwrap().clone();
    if callbacks.is_empty() {
        eprintln!(
            "Unhandled 'error' event: {} ({})",
            error.message, error.code
        );
        UNHANDLED_SERVER_ERROR.store(true, Ordering::Release);
        return;
    }
    let message = CString::new(error.message.as_str()).unwrap_or_default();
    let code = CString::new(error.code.as_str()).unwrap_or_default();
    let syscall = CString::new("listen").unwrap();
    let address = CString::new("127.0.0.1").unwrap();
    let native = NativeServerError {
        message: message.as_ptr(),
        code: code.as_ptr(),
        syscall: syscall.as_ptr(),
        address: address.as_ptr(),
        port: error.port,
    };
    for listener in callbacks {
        let callback = listener.callback as *const c_void;
        unsafe {
            type Callback = unsafe extern "C" fn(*const c_void, *const NativeServerError);
            let code = *(callback as *const *const c_void);
            let callback_fn: Callback = std::mem::transmute(code);
            callback_fn(callback, &native);
        }
    }
}

#[no_mangle]
pub extern "C" fn thaw_http_take_unhandled_error() -> u8 {
    u8::from(UNHANDLED_SERVER_ERROR.swap(false, Ordering::AcqRel))
}

extern "C" fn server_listener_ready(context: *mut u8, _events: i16) {
    let state = unsafe { &*(context as *const ServerState) };
    if state.closed.load(Ordering::Acquire) {
        return;
    }
    let accepted = {
        let listener = state.listener.lock().unwrap();
        listener.as_ref().map(TcpListener::accept)
    };
    match accepted {
        Some(Ok((stream, _))) => register_connection(stream, state),
        Some(Err(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        Some(Err(_)) | None => state.closed.store(true, Ordering::Release),
    }
}

fn register_connection(stream: TcpStream, server: &ServerState) {
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    let connection = Box::into_raw(Box::new(ConnectionState {
        stream,
        server,
        request: Vec::new(),
        response: Vec::new(),
        written: 0,
        watcher: 0,
    }));
    let watcher = unsafe {
        thaw_runtime_watch_fd(
            (*connection).stream.as_raw_fd(),
            THAW_FD_READABLE,
            connection_ready,
            connection.cast(),
        )
    };
    if watcher == 0 {
        unsafe { drop(Box::from_raw(connection)) };
        return;
    }
    unsafe { (*connection).watcher = watcher };
    ACTIVE_CONNECTIONS.fetch_add(1, Ordering::AcqRel);
    server.connections.fetch_add(1, Ordering::AcqRel);
}

extern "C" fn connection_ready(context: *mut u8, _events: i16) {
    let connection = unsafe { &mut *(context as *mut ConnectionState) };
    if connection.response.is_empty() && !read_request(connection) {
        return;
    }
    write_response(connection);
}

fn read_request(connection: &mut ConnectionState) -> bool {
    let mut chunk = [0_u8; 4096];
    loop {
        match connection.stream.read(&mut chunk) {
            Ok(0) => {
                finish_connection(connection);
                return false;
            }
            Ok(length) => {
                connection.request.extend_from_slice(&chunk[..length]);
                if connection.request.len() > MAX_REQUEST_HEAD {
                    finish_connection(connection);
                    return false;
                }
                if connection
                    .request
                    .windows(4)
                    .any(|bytes| bytes == b"\r\n\r\n")
                {
                    let request = String::from_utf8_lossy(&connection.request);
                    let mut request_line = request.lines().next().unwrap_or("").split_whitespace();
                    let method = request_line.next().unwrap_or("GET");
                    let target = request_line.next().unwrap_or("/");
                    let server = unsafe { &*connection.server };
                    connection.response = render_response(invoke_server_callback(
                        server.callback as *const c_void,
                        method,
                        target,
                    ));
                    return true;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return false,
            Err(_) => {
                finish_connection(connection);
                return false;
            }
        }
    }
}

fn write_response(connection: &mut ConnectionState) {
    while connection.written < connection.response.len() {
        match connection
            .stream
            .write(&connection.response[connection.written..])
        {
            Ok(0) => {
                finish_connection(connection);
                return;
            }
            Ok(length) => connection.written += length,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                rewatch_connection(connection, THAW_FD_WRITABLE);
                return;
            }
            Err(_) => {
                finish_connection(connection);
                return;
            }
        }
    }
    finish_connection(connection);
}

fn rewatch_connection(connection: &mut ConnectionState, interests: u8) {
    unsafe { thaw_runtime_unwatch_fd(connection.watcher) };
    connection.watcher = unsafe {
        thaw_runtime_watch_fd(
            connection.stream.as_raw_fd(),
            interests,
            connection_ready,
            (connection as *mut ConnectionState).cast(),
        )
    };
    if connection.watcher == 0 {
        finish_connection(connection);
    }
}

fn finish_connection(connection: &mut ConnectionState) {
    if connection.watcher != 0 {
        unsafe { thaw_runtime_unwatch_fd(connection.watcher) };
        connection.watcher = 0;
    }
    ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::AcqRel);
    unsafe { &*connection.server }
        .connections
        .fetch_sub(1, Ordering::AcqRel);
    unsafe { drop(Box::from_raw(connection as *mut ConnectionState)) };
}

#[repr(C)]
struct Server {
    listen: *const NativeClosure,
    listen_with_callback: *const NativeClosure,
    listen_many: *const NativeClosure,
    close: *const NativeClosure,
    close_with_callback: *const NativeClosure,
    on: *const NativeClosure,
    on_error: *const NativeClosure,
}

unsafe extern "C" fn server_listen(environment: *const c_void, port: f64) -> *const c_char {
    let closure = &*(environment as *const NativeClosure);
    let state = &*(closure.context as *const ServerState);
    if !port.is_finite() || port < 0.0 || port > u16::MAX as f64 {
        fail_server_listen(
            state,
            "Port should be >= 0 and < 65536".into(),
            "ERR_SOCKET_BAD_PORT".into(),
            port,
        );
        return CString::new("").unwrap().into_raw();
    }
    let listener = match TcpListener::bind(("127.0.0.1", port as u16)) {
        Ok(listener) => listener,
        Err(error) => {
            let message = if error.kind() == std::io::ErrorKind::AddrInUse {
                "EADDRINUSE".into()
            } else {
                format!("listen failed: {error}")
            };
            fail_server_listen(state, message.clone(), message, port);
            return CString::new("").unwrap().into_raw();
        }
    };
    if listener.set_nonblocking(true).is_err() {
        fail_server_listen(
            state,
            "failed to configure non-blocking listener".into(),
            "ERR_SERVER_LISTEN".into(),
            port,
        );
        return CString::new("").unwrap().into_raw();
    }
    state.closed.store(false, Ordering::Release);
    *state.listener.lock().unwrap() = Some(listener);
    let old_watcher = state.watcher.swap(0, Ordering::AcqRel);
    if old_watcher != 0 {
        thaw_runtime_unwatch_fd(old_watcher);
    }
    let fd = state.listener.lock().unwrap().as_ref().unwrap().as_raw_fd();
    let watcher = thaw_runtime_watch_fd(
        fd,
        THAW_FD_READABLE,
        server_listener_ready,
        (state as *const ServerState).cast_mut().cast(),
    );
    if watcher == 0 {
        fail_server_listen(
            state,
            "failed to register listener with event loop".into(),
            "ERR_SERVER_LISTEN".into(),
            port,
        );
        return CString::new("").unwrap().into_raw();
    }
    state.watcher.store(watcher, Ordering::Release);
    state.pending_listening.store(true, Ordering::Release);
    register_server(state);
    CString::new("").unwrap().into_raw()
}

unsafe extern "C" fn server_listen_with_callback(
    environment: *const c_void,
    port: f64,
    callback: *const c_void,
) -> *const c_char {
    let closure = &*(environment as *const NativeClosure);
    let state = &*(closure.context as *const ServerState);
    add_event_listener(&state.listening_listeners, callback, true);
    server_listen(environment, port)
}

fn fail_server_listen(state: &ServerState, message: String, code: String, port: f64) {
    state.closed.store(true, Ordering::Release);
    state.pending_errors.lock().unwrap().push(ServerError {
        message,
        code,
        port,
    });
    *state.listener.lock().unwrap() = None;
    register_server(state);
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
    state.closed.store(false, Ordering::Release);
    CString::new(run_server_many(port, state, count))
        .unwrap_or_default()
        .into_raw()
}

unsafe extern "C" fn server_close(environment: *const c_void) -> bool {
    let closure = &*(environment as *const NativeClosure);
    let state = &*(closure.context as *const ServerState);
    close_server_state(state)
}

unsafe extern "C" fn server_close_with_callback(
    environment: *const c_void,
    callback: *const c_void,
) -> bool {
    let closure = &*(environment as *const NativeClosure);
    let state = &*(closure.context as *const ServerState);
    add_event_listener(&state.close_listeners, callback, true);
    close_server_state(state)
}

unsafe extern "C" fn server_on(
    environment: *const c_void,
    event: *const c_char,
    callback: *const c_void,
) -> bool {
    let closure = &*(environment as *const NativeClosure);
    let state = &*(closure.context as *const ServerState);
    match string_from_ptr(event).as_str() {
        "listening" => add_event_listener(&state.listening_listeners, callback, false),
        "close" => add_event_listener(&state.close_listeners, callback, false),
        _ => return false,
    }
    !callback.is_null()
}

unsafe extern "C" fn server_on_error(
    environment: *const c_void,
    event: *const c_char,
    callback: *const c_void,
) -> bool {
    if string_from_ptr(event) != "error" {
        return false;
    }
    let closure = &*(environment as *const NativeClosure);
    let state = &*(closure.context as *const ServerState);
    add_event_listener(&state.error_listeners, callback, false);
    !callback.is_null()
}

fn close_server_state(state: &ServerState) -> bool {
    let was_open = !state.closed.swap(true, Ordering::AcqRel);
    if was_open {
        state.close_requested.store(true, Ordering::Release);
    }
    let watcher = state.watcher.swap(0, Ordering::AcqRel);
    if watcher != 0 {
        unsafe { thaw_runtime_unwatch_fd(watcher) };
    }
    was_open
}

/// Node-shaped constructor slice: returns an object with `listen(port)`.
#[no_mangle]
pub extern "C" fn createServer(callback: *const c_void) -> *const c_void {
    if callback.is_null() {
        return std::ptr::null();
    }
    let state = Box::into_raw(Box::new(ServerState {
        callback: callback as usize,
        closed: AtomicBool::new(false),
        listener: Mutex::new(None),
        watcher: AtomicU64::new(0),
        connections: AtomicUsize::new(0),
        listening_listeners: Mutex::new(Vec::new()),
        close_listeners: Mutex::new(Vec::new()),
        error_listeners: Mutex::new(Vec::new()),
        pending_listening: AtomicBool::new(false),
        pending_errors: Mutex::new(Vec::new()),
        close_requested: AtomicBool::new(false),
    }));
    let listen = Box::into_raw(Box::new(NativeClosure {
        code: server_listen as *const c_void,
        context: state.cast(),
    }));
    let listen_many = Box::into_raw(Box::new(NativeClosure {
        code: server_listen_many as *const c_void,
        context: state.cast(),
    }));
    let listen_with_callback = Box::into_raw(Box::new(NativeClosure {
        code: server_listen_with_callback as *const c_void,
        context: state.cast(),
    }));
    let close = Box::into_raw(Box::new(NativeClosure {
        code: server_close as *const c_void,
        context: state.cast(),
    }));
    let close_with_callback = Box::into_raw(Box::new(NativeClosure {
        code: server_close_with_callback as *const c_void,
        context: state.cast(),
    }));
    let on = Box::into_raw(Box::new(NativeClosure {
        code: server_on as *const c_void,
        context: state.cast(),
    }));
    let on_error = Box::into_raw(Box::new(NativeClosure {
        code: server_on_error as *const c_void,
        context: state.cast(),
    }));
    Box::into_raw(Box::new(Server {
        listen,
        listen_with_callback,
        listen_many,
        close,
        close_with_callback,
        on,
        on_error,
    }))
    .cast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unhandled_server_error_sets_process_failure_once() {
        let listeners = Mutex::new(Vec::new());
        emit_error(
            &listeners,
            &ServerError {
                message: "listen failed".into(),
                code: "EADDRINUSE".into(),
                port: 3000.0,
            },
        );
        assert_eq!(thaw_http_take_unhandled_error(), 1);
        assert_eq!(thaw_http_take_unhandled_error(), 0);
    }

    #[test]
    fn encoded_response_decodes_binary_bytes() {
        let mut state = ResponseState {
            headers: Vec::new(),
            body: Vec::new(),
        };
        let closure = NativeClosure {
            code: response_end_encoded as *const c_void,
            context: (&mut state as *mut ResponseState).cast(),
        };
        let content = CString::new("89504e4700ff").unwrap();
        let encoding = CString::new("hex").unwrap();
        assert!(unsafe {
            response_end_encoded(
                (&closure as *const NativeClosure).cast(),
                content.as_ptr(),
                encoding.as_ptr(),
            )
        });
        assert_eq!(state.body, [0x89, 0x50, 0x4e, 0x47, 0x00, 0xff]);
    }

    use std::net::TcpStream;
    use std::sync::Arc;
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

    #[test]
    fn close_interrupts_a_continuous_listener_between_requests() {
        unsafe extern "C" fn callback(
            _environment: *const c_void,
            _request: *const IncomingMessage,
            _response: *mut ServerResponse,
        ) -> bool {
            true
        }

        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let callback = Box::into_raw(Box::new(NativeClosure {
            code: callback as *const c_void,
            context: std::ptr::null_mut(),
        }));
        let state = Arc::new(ServerState {
            callback: callback as usize,
            closed: AtomicBool::new(false),
            listener: Mutex::new(None),
            watcher: AtomicU64::new(0),
            connections: AtomicUsize::new(0),
            listening_listeners: Mutex::new(Vec::new()),
            close_listeners: Mutex::new(Vec::new()),
            error_listeners: Mutex::new(Vec::new()),
            pending_listening: AtomicBool::new(false),
            pending_errors: Mutex::new(Vec::new()),
            close_requested: AtomicBool::new(false),
        });
        let server_state = Arc::clone(&state);
        let server = thread::spawn(move || run_server_many(port as f64, &server_state, usize::MAX));
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => thread::sleep(Duration::from_millis(5)),
            }
        };
        stream
            .write_all(b"GET /first HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        state.closed.store(true, Ordering::Release);
        assert_eq!(server.join().unwrap(), "/first");
    }

    #[test]
    fn lifecycle_loop_advances_a_complete_request_past_a_slow_connection() {
        static CALLBACKS: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "C" fn callback(
            environment: *const c_void,
            request: *const IncomingMessage,
            response: *mut ServerResponse,
        ) -> bool {
            let body = CString::new(string_from_ptr((*request).url)).unwrap();
            let ended = response_end((*response).end.cast(), body.as_ptr());
            if CALLBACKS.fetch_add(1, Ordering::AcqRel) == 1 {
                let closure = &*(environment as *const NativeClosure);
                close_server_state(&*(closure.context as *const ServerState));
            }
            ended
        }

        CALLBACKS.store(0, Ordering::Release);
        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let callback = Box::into_raw(Box::new(NativeClosure {
            code: callback as *const c_void,
            context: std::ptr::null_mut(),
        }));
        let state = Box::into_raw(Box::new(ServerState {
            callback: callback as usize,
            closed: AtomicBool::new(false),
            listener: Mutex::new(None),
            watcher: AtomicU64::new(0),
            connections: AtomicUsize::new(0),
            listening_listeners: Mutex::new(Vec::new()),
            close_listeners: Mutex::new(Vec::new()),
            error_listeners: Mutex::new(Vec::new()),
            pending_listening: AtomicBool::new(false),
            pending_errors: Mutex::new(Vec::new()),
            close_requested: AtomicBool::new(false),
        }));
        unsafe { (*callback).context = state.cast() };
        let listen = NativeClosure {
            code: server_listen as *const c_void,
            context: state.cast(),
        };
        unsafe {
            let result = server_listen((&listen as *const NativeClosure).cast(), port as f64);
            assert_eq!(CStr::from_ptr(result).to_bytes(), b"");
        }

        let client = thread::spawn(move || {
            let mut slow = loop {
                match TcpStream::connect(("127.0.0.1", port)) {
                    Ok(stream) => break stream,
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            };
            slow.write_all(b"GET /slow HTTP/1.1\r\nHost: localhost")
                .unwrap();
            let mut fast = TcpStream::connect(("127.0.0.1", port)).unwrap();
            fast.write_all(b"GET /fast HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut fast_response = String::new();
            fast.read_to_string(&mut fast_response).unwrap();

            slow.write_all(b"\r\n\r\n").unwrap();
            let mut slow_response = String::new();
            slow.read_to_string(&mut slow_response).unwrap();
            (fast_response, slow_response)
        });
        thaw_http_run_servers();
        let (fast_response, slow_response) = client.join().unwrap();
        assert!(fast_response.ends_with("/fast"));
        assert!(slow_response.ends_with("/slow"));
        assert_eq!(CALLBACKS.load(Ordering::Acquire), 2);
        assert!(unsafe { &*state }.closed.load(Ordering::Acquire));
    }
}
