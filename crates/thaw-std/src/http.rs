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
    // `handle_stream` backs the one-shot helpers (`serveOnce`,
    // `serveOnceWith`, and `run_server_many`'s blocking per-connection
    // loop) -- each handles exactly one request per accepted TCP
    // connection by construction, so keep-alive doesn't apply here.
    let response = render_response(response_for(&method, &target), false);
    stream
        .write_all(&response)
        .map_err(|error| format!("response write failed: {error}"))?;
    Ok(target)
}

fn status_reason(status: u16) -> &'static str {
    match status {
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

fn render_head(
    status: u16,
    headers: &[(String, String)],
    framing: &str,
    keep_alive: bool,
) -> String {
    let mut head = format!("HTTP/1.1 {} {}\r\n", status, status_reason(status));
    for (name, value) in headers {
        head.push_str(&name.replace(['\r', '\n'], ""));
        head.push_str(": ");
        head.push_str(&value.replace(['\r', '\n'], ""));
        head.push_str("\r\n");
    }
    head.push_str(framing);
    head.push_str(if keep_alive {
        "Connection: keep-alive\r\n\r\n"
    } else {
        "Connection: close\r\n\r\n"
    });
    head
}

fn render_response(response_spec: ResponseSpec, keep_alive: bool) -> Vec<u8> {
    let framing = format!("Content-Length: {}\r\n", response_spec.body.len());
    let mut response = render_head(
        response_spec.status,
        &response_spec.headers,
        &framing,
        keep_alive,
    )
    .into_bytes();
    response.extend(response_spec.body);
    response
}

/// The status line + headers for a response whose body is streamed as it
/// is produced: `Transfer-Encoding: chunked` since the total length
/// isn't known when the head goes out. Any caller-set `Content-Length`
/// is dropped (it would contradict the chunked framing).
fn render_streaming_head(status: u16, headers: &[(String, String)], keep_alive: bool) -> Vec<u8> {
    let headers: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("Content-Length"))
        .cloned()
        .collect();
    render_head(
        status,
        &headers,
        "Transfer-Encoding: chunked\r\n",
        keep_alive,
    )
    .into_bytes()
}

/// One `Transfer-Encoding: chunked` chunk: `<hex len>\r\n<bytes>\r\n`.
/// A zero-length payload would frame the terminating chunk, so callers
/// skip empty writes rather than routing them here.
fn frame_chunk(payload: &[u8]) -> Vec<u8> {
    let mut chunk = format!("{:x}\r\n", payload.len()).into_bytes();
    chunk.extend_from_slice(payload);
    chunk.extend_from_slice(b"\r\n");
    chunk
}

const CHUNKED_TERMINATOR: &[u8] = b"0\r\n\r\n";

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
    /// Set the first time `response.end(...)` (or `endEncoded`) runs.
    /// Until then the handler isn't finished -- a plain handler that
    /// simply hasn't returned yet, or an `async` one currently suspended
    /// on an `await` -- and there is no response to send.
    ended: bool,
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

/// Everything one in-flight request/response needs that must outlive a
/// single turn of the event loop. A synchronous handler runs start to
/// finish inside `run_server_callback` and this could all be stack
/// local -- but an `async` handler suspends on its first `await` and
/// returns control before it has called `response.end(...)`, so the
/// `ServerResponse`/`IncomingMessage` it keeps referencing (and the
/// buffers its `write`/`end` closures append to) are boxed and owned by
/// the `ConnectionState` until the response has been fully written.
struct RequestContext {
    /// The connection this request arrived on, or null for the one-shot
    /// helpers (`createServerOnce` etc.) that run no event loop and so
    /// can't resume a handler that suspends.
    connection: *mut ConnectionState,
    /// False until `run_server_callback` hands ownership of this context
    /// to the connection because the handler suspended. `finish_response`
    /// checks it *before* touching `connection`, so the synchronous path
    /// (where the caller still holds `&mut ConnectionState`) never
    /// aliases it. Also gates streaming: a `response.write(...)` only
    /// streams once the handler has suspended at least once (before that
    /// the whole response is still buffered and sent with a real
    /// `Content-Length`, which no observer can tell from streaming since
    /// the handler hasn't yielded).
    resumable: bool,
    /// Set once the chunked response head has been queued -- from then on
    /// every `write`/`end` frames a chunk instead of buffering.
    headers_sent: bool,
    state: ResponseState,
    response: ServerResponse,
    request: IncomingMessage,
    set_header: NativeClosure,
    write: NativeClosure,
    end: NativeClosure,
    end_encoded: NativeClosure,
    // Backing storage the `IncomingMessage` pointers borrow from; never
    // read through directly (hence the underscores), just kept alive.
    _method: CString,
    _url: CString,
}

impl RequestContext {
    fn new(method: &str, target: &str, connection: *mut ConnectionState) -> Box<RequestContext> {
        let mut context = Box::new(RequestContext {
            connection,
            resumable: false,
            headers_sent: false,
            state: ResponseState {
                headers: Vec::new(),
                body: Vec::new(),
                ended: false,
            },
            response: ServerResponse {
                status_code: 200.0,
                set_header: std::ptr::null(),
                end: std::ptr::null(),
                write: std::ptr::null(),
                end_encoded: std::ptr::null(),
            },
            request: IncomingMessage {
                method: std::ptr::null(),
                url: std::ptr::null(),
            },
            set_header: NativeClosure {
                code: response_set_header as *const c_void,
                context: std::ptr::null_mut(),
            },
            write: NativeClosure {
                code: response_write as *const c_void,
                context: std::ptr::null_mut(),
            },
            end: NativeClosure {
                code: response_end as *const c_void,
                context: std::ptr::null_mut(),
            },
            end_encoded: NativeClosure {
                code: response_end_encoded as *const c_void,
                context: std::ptr::null_mut(),
            },
            _method: CString::new(method).unwrap_or_default(),
            _url: CString::new(target).unwrap_or_default(),
        });
        // The box has a stable address now -- wire every self pointer.
        let context_ptr: *mut RequestContext = &mut *context;
        context.request.method = context._method.as_ptr();
        context.request.url = context._url.as_ptr();
        context.set_header.context = context_ptr.cast();
        context.write.context = context_ptr.cast();
        context.end.context = context_ptr.cast();
        context.end_encoded.context = context_ptr.cast();
        context.response.set_header = &context.set_header;
        context.response.end = &context.end;
        context.response.write = &context.write;
        context.response.end_encoded = &context.end_encoded;
        context
    }
}

unsafe fn request_context(environment: *const c_void) -> &'static mut RequestContext {
    let closure = &*(environment as *const NativeClosure);
    &mut *(closure.context as *mut RequestContext)
}

unsafe extern "C" fn response_set_header(
    environment: *const c_void,
    name: *const c_char,
    value: *const c_char,
) -> bool {
    request_context(environment)
        .state
        .headers
        .push((string_from_ptr(name), string_from_ptr(value)));
    true
}

unsafe extern "C" fn response_write(environment: *const c_void, chunk: *const c_char) -> bool {
    let context = request_context(environment);
    let bytes = string_from_ptr(chunk).into_bytes();
    if stream_should_engage(context) {
        stream_write(context, &bytes);
    } else {
        // Still in the handler's synchronous burst (or a one-shot helper
        // with no event loop): buffer, and let the `Content-Length` path
        // send the whole thing -- nothing has observed a partial
        // response, so this is indistinguishable from streaming.
        context.state.body.extend(bytes);
    }
    true
}

unsafe extern "C" fn response_end_encoded(
    environment: *const c_void,
    content: *const c_char,
    encoding: *const c_char,
) -> bool {
    let content = string_from_ptr(content);
    let context = request_context(environment);
    if string_from_ptr(encoding) != "hex" {
        stream_or_buffer_end(context, content.into_bytes());
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
    stream_or_buffer_end(context, decoded);
    true
}

unsafe extern "C" fn response_end(environment: *const c_void, chunk: *const c_char) -> bool {
    let context = request_context(environment);
    stream_or_buffer_end(context, string_from_ptr(chunk).into_bytes());
    true
}

/// Whether a `response.write(...)` on `context` should stream directly
/// onto the socket rather than buffer. Only once the handler has
/// suspended at least once (`resumable`) and this context owns a real
/// connection: before that the whole response is still gathered and sent
/// with a `Content-Length`, which no client can distinguish from a
/// stream since the handler hasn't yielded.
fn stream_should_engage(context: &RequestContext) -> bool {
    context.resumable && !context.connection.is_null()
}

/// Builds the bytes for one streamed `write`: the chunked response head
/// the first time (plus any bytes the handler buffered before it first
/// suspended, as the leading chunk), then `payload` as its own chunk.
fn stream_head_and_chunk(context: &mut RequestContext, payload: &[u8]) -> Vec<u8> {
    let mut outbound = Vec::new();
    if !context.headers_sent {
        context.headers_sent = true;
        let keep_alive = unsafe { (*context.connection).keep_alive };
        outbound.extend(render_streaming_head(
            normalize_status(context.response.status_code),
            &context.state.headers,
            keep_alive,
        ));
        let pending = std::mem::take(&mut context.state.body);
        if !pending.is_empty() {
            outbound.extend(frame_chunk(&pending));
        }
    }
    if !payload.is_empty() {
        outbound.extend(frame_chunk(payload));
    }
    outbound
}

fn stream_write(context: &mut RequestContext, payload: &[u8]) {
    let outbound = stream_head_and_chunk(context, payload);
    let connection = unsafe { &mut *context.connection };
    connection.streaming = true;
    connection.awaiting_handler = false;
    queue_and_flush(connection, outbound);
}

/// `response.end(tail)`. If the response is already streaming (a
/// `write` happened), or a `write` happened before the handler suspended
/// and it's now ending without another `write`, finish it as a stream:
/// final chunk (if any) then the terminating `0\r\n\r\n`. Otherwise the
/// whole body is in hand and goes out buffered with a `Content-Length`
/// (`finish_response` handles the synchronous vs parked-async split).
fn stream_or_buffer_end(context: &mut RequestContext, tail: Vec<u8>) {
    let force_stream = stream_should_engage(context) && !context.state.body.is_empty();
    if !context.headers_sent && !force_stream {
        context.state.body.extend(tail);
        finish_response(context);
        return;
    }
    let mut outbound = stream_head_and_chunk(context, &tail);
    outbound.extend_from_slice(CHUNKED_TERMINATOR);
    context.state.ended = true;
    let connection = unsafe { &mut *context.connection };
    connection.streaming = true;
    connection.awaiting_handler = false;
    connection.response_ended = true;
    queue_and_flush(connection, outbound);
}

fn queue_and_flush(connection: &mut ConnectionState, mut bytes: Vec<u8>) {
    connection.response.append(&mut bytes);
    write_response(connection);
}

/// Parks a suspended `async` handler's context on the connection. If the
/// handler already wrote bytes before suspending, the chunked stream
/// begins with them now: Node flushes an early `response.write(...)`
/// immediately, so holding those bytes until the handler resumes would
/// add its whole `await` to the client's wait for the first byte.
/// Returns whether there is now output for `write_response` to flush.
fn park_pending_context(
    connection: &mut ConnectionState,
    context: *mut RequestContext,
    keep_alive: bool,
) -> bool {
    connection.response_ctx = context;
    let context = unsafe { &mut *context };
    if context.state.body.is_empty() {
        connection.awaiting_handler = true;
        return false;
    }
    context.headers_sent = true;
    connection.streaming = true;
    let mut head = render_streaming_head(
        normalize_status(context.response.status_code),
        &context.state.headers,
        keep_alive,
    );
    head.extend(frame_chunk(&std::mem::take(&mut context.state.body)));
    connection.response = head;
    connection.written = 0;
    true
}

/// Marks the response complete. On the synchronous path this just flips
/// `ended` and returns -- `run_server_callback` is still on the stack
/// (and still holds the caller's `&mut ConnectionState`) and renders the
/// response itself once the handler returns. On the async path the
/// handler has already suspended and returned, the connection is parked
/// (`awaiting_handler`), and this call -- reached from inside the
/// handler's resumed continuation -- is the signal to render and hand
/// the response to the event loop.
fn finish_response(context: &mut RequestContext) {
    if context.state.ended {
        return;
    }
    context.state.ended = true;
    if !context.resumable || context.connection.is_null() {
        return;
    }
    let connection = unsafe { &mut *context.connection };
    connection.awaiting_handler = false;
    let spec = ResponseSpec {
        status: normalize_status(context.response.status_code),
        headers: std::mem::take(&mut context.state.headers),
        body: std::mem::take(&mut context.state.body),
    };
    connection.response = render_response(spec, connection.keep_alive);
    // Deliberately does not write here: this runs inside the async
    // handler's resume, and any handler code after `response.end(...)`
    // still expects a live `ServerResponse`. Re-arming for writability
    // instead defers `write_response` (which may free the whole
    // connection, `RequestContext` included) to the next event-loop
    // turn, once that continuation has fully unwound.
    rewatch_connection(connection, THAW_FD_WRITABLE);
}

fn normalize_status(status_code: f64) -> u16 {
    if status_code.is_finite() && (100.0..=999.0).contains(&status_code) {
        status_code as u16
    } else {
        500
    }
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

/// Result of running the user's `createServer` handler for one request.
enum CallbackOutcome {
    /// The handler called `response.end(...)` (or there is no event loop
    /// to wait on): the finished response.
    Ready(ResponseSpec),
    /// An `async` handler suspended before finishing. The boxed context's
    /// ownership transfers to the caller, which parks it on the
    /// connection; `finish_response` completes it on a later turn.
    Pending(*mut RequestContext),
}

fn run_server_callback(
    callback: *const c_void,
    method: &str,
    target: &str,
    connection: *mut ConnectionState,
) -> CallbackOutcome {
    let mut context = RequestContext::new(method, target, connection);
    unsafe {
        // The ambient `.d.ts` declares this callback `=> void`, so thaw
        // emits a void-returning native function for a plain handler --
        // transmuting the code pointer through a `-> bool` signature (as
        // this once did) is a real ABI mismatch that crashed on the
        // first real request. An `async` handler's value is accepted
        // against that same `=> void` slot (a void-returning callback
        // position admits any return type, matching TypeScript) and
        // genuinely returns a promise handle -- harmlessly ignored: a
        // lone pointer-sized return left unread in a register is fine
        // under the C ABI.
        type Callback =
            unsafe extern "C" fn(*const c_void, *const IncomingMessage, *mut ServerResponse);
        let request_ptr: *const IncomingMessage = &context.request;
        let response_ptr: *mut ServerResponse = &mut context.response;
        let code = *(callback as *const *const c_void);
        let callback_fn: Callback = std::mem::transmute(code);
        callback_fn(callback, request_ptr, response_ptr);
    }
    if context.state.ended || connection.is_null() {
        CallbackOutcome::Ready(ResponseSpec {
            status: normalize_status(context.response.status_code),
            headers: std::mem::take(&mut context.state.headers),
            body: std::mem::take(&mut context.state.body),
        })
    } else {
        context.resumable = true;
        CallbackOutcome::Pending(Box::into_raw(context))
    }
}

/// Synchronous entry point for the one-shot helpers, which have no event
/// loop and so can only ever see a `Ready` outcome.
fn invoke_server_callback(callback: *const c_void, method: &str, target: &str) -> ResponseSpec {
    match run_server_callback(callback, method, target, std::ptr::null_mut()) {
        CallbackOutcome::Ready(spec) => spec,
        CallbackOutcome::Pending(context) => {
            let context = unsafe { Box::from_raw(context) };
            ResponseSpec {
                status: normalize_status(context.response.status_code),
                headers: context.state.headers.clone(),
                body: context.state.body.clone(),
            }
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
    /// Decided once per request, right after the head is parsed
    /// (`negotiate_keep_alive`), and consumed by `write_response` once
    /// the response finishes: reuse the connection for another request
    /// instead of closing it.
    keep_alive: bool,
    /// True between an `async` handler suspending and its eventual
    /// `response.end(...)`. While set, `connection_ready` ignores socket
    /// events -- the handler's resumed continuation drives the response
    /// (`finish_response`), not a read here.
    awaiting_handler: bool,
    /// The boxed per-request context, owned here while an `async` handler
    /// is in flight (and until the response it produced is fully
    /// written). Null the rest of the time.
    response_ctx: *mut RequestContext,
    /// True once a `response.write(...)` has begun streaming a chunked
    /// response directly onto this socket. `response` is then an
    /// incremental outbound buffer that drains as the socket accepts it,
    /// rather than a whole response rendered at once.
    streaming: bool,
    /// True once the streamed response's terminating chunk has been
    /// queued: the next time `response` fully drains, the connection
    /// moves on to the keep-alive-or-close decision.
    response_ended: bool,
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
        keep_alive: false,
        awaiting_handler: false,
        response_ctx: std::ptr::null_mut(),
        streaming: false,
        response_ended: false,
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
    if connection.awaiting_handler {
        // An `async` handler is still running; its eventual
        // `response.end(...)` re-arms this connection for writing. Any
        // socket event now (including the client hanging up) is left for
        // that path to notice.
        return;
    }
    if connection.streaming && !connection.response_ended {
        // Mid-stream: flush whatever chunk bytes are queued. A readable
        // event here (pipelined bytes, half-close) is ignored until the
        // handler produces the next chunk or ends the response.
        if !connection.response.is_empty() {
            write_response(connection);
        }
        return;
    }
    if connection.response.is_empty() && !read_request(connection) {
        return;
    }
    write_response(connection);
}

/// Whether `request` (the buffered head, including its trailing blank
/// line) should keep the connection open for another request once this
/// one's response is fully sent. HTTP/1.1 defaults to keep-alive unless
/// the request says `Connection: close`; HTTP/1.0 defaults to close
/// unless it says `Connection: keep-alive`. A `Content-Length`/
/// `Transfer-Encoding` header forces `false` regardless of version --
/// this parser doesn't read a request body at all (a separate,
/// pre-existing limitation), and reusing the connection while unread
/// body bytes are still sitting in the stream would desync every
/// request after this one, which is worse than just not reusing it.
fn negotiate_keep_alive(request: &str) -> bool {
    let mut lines = request.lines();
    let Some(request_line) = lines.next() else {
        return false;
    };
    let http_1_1 = request_line
        .split_whitespace()
        .next_back()
        .is_some_and(|version| version.eq_ignore_ascii_case("HTTP/1.1"));
    let mut connection_close = false;
    let mut connection_keep_alive = false;
    let mut has_body_header = false;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("Connection") {
            for token in value.split(',') {
                let token = token.trim();
                if token.eq_ignore_ascii_case("close") {
                    connection_close = true;
                } else if token.eq_ignore_ascii_case("keep-alive") {
                    connection_keep_alive = true;
                }
            }
        } else if name.eq_ignore_ascii_case("Content-Length")
            || name.eq_ignore_ascii_case("Transfer-Encoding")
        {
            has_body_header = true;
        }
    }
    if has_body_header {
        return false;
    }
    if http_1_1 {
        !connection_close
    } else {
        connection_keep_alive
    }
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
                    let (method, target, keep_alive) = {
                        let request = String::from_utf8_lossy(&connection.request);
                        let mut request_line =
                            request.lines().next().unwrap_or("").split_whitespace();
                        let method = request_line.next().unwrap_or("GET").to_string();
                        let target = request_line.next().unwrap_or("/").to_string();
                        let keep_alive = negotiate_keep_alive(&request);
                        (method, target, keep_alive)
                    };
                    let callback = unsafe { &*connection.server }.callback as *const c_void;
                    // Set before running the handler: an `async` handler
                    // that suspends won't return through here, and
                    // `finish_response` needs the negotiated value when it
                    // renders the response later.
                    connection.keep_alive = keep_alive;
                    match run_server_callback(
                        callback,
                        &method,
                        &target,
                        connection as *mut ConnectionState,
                    ) {
                        CallbackOutcome::Ready(spec) => {
                            connection.response = render_response(spec, keep_alive);
                            return true;
                        }
                        CallbackOutcome::Pending(context) => {
                            return park_pending_context(connection, context, keep_alive);
                        }
                    }
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
    if connection.streaming && !connection.response_ended {
        // A streamed response with more chunks still to come: everything
        // queued so far is on the wire, so drop it and wait (idle, armed
        // only for readable so the socket isn't spuriously "writable")
        // for the handler's next `write`/`end`.
        connection.response.clear();
        connection.written = 0;
        rewatch_connection(connection, THAW_FD_READABLE);
        return;
    }
    if connection.keep_alive {
        // Reuse the connection instead of closing it: reset the
        // per-request fields and re-arm for another request's head,
        // exactly the same registration `register_connection` already
        // does for a brand-new connection. `connection_ready` decides
        // whether to read or write purely from `response.is_empty()`,
        // so clearing it here is what makes the next readiness event
        // re-enter `read_request` instead of trying to write again.
        //
        // Pipelining (a client sending a second request before reading
        // this response) isn't supported: `connection.request.clear()`
        // discards any bytes already read past this request's own
        // `\r\n\r\n`, and `read_request` only ever looks for new bytes
        // via a fresh readable event, not ones already sitting in a
        // just-cleared buffer -- a genuinely pipelining client's second
        // request would be silently lost. A client that waits for each
        // response before sending the next one (the common case, and
        // what Node's own default `http.Agent` does on a keep-alive
        // connection) is unaffected.
        free_response_context(connection);
        connection.request.clear();
        connection.response.clear();
        connection.written = 0;
        connection.awaiting_handler = false;
        connection.streaming = false;
        connection.response_ended = false;
        rewatch_connection(connection, THAW_FD_READABLE);
        return;
    }
    finish_connection(connection);
}

/// Drops the boxed `RequestContext` a still-in-flight (or just-completed)
/// `async` request left parked on the connection, if any.
fn free_response_context(connection: &mut ConnectionState) {
    if !connection.response_ctx.is_null() {
        unsafe { drop(Box::from_raw(connection.response_ctx)) };
        connection.response_ctx = std::ptr::null_mut();
    }
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
    free_response_context(connection);
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
    fn negotiates_keep_alive_by_version_header_and_body_presence() {
        assert!(negotiate_keep_alive(
            "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n"
        ));
        assert!(!negotiate_keep_alive(
            "GET / HTTP/1.1\r\nConnection: close\r\n\r\n"
        ));
        assert!(!negotiate_keep_alive(
            "GET / HTTP/1.0\r\nHost: localhost\r\n\r\n"
        ));
        assert!(negotiate_keep_alive(
            "GET / HTTP/1.0\r\nConnection: keep-alive\r\n\r\n"
        ));
        // No request-body parsing support (a separate, pre-existing
        // limitation) means reusing the connection while unread body
        // bytes are still in the stream would desync every request
        // after this one -- force close whenever a body might exist,
        // even on an otherwise keep-alive-eligible HTTP/1.1 request.
        assert!(!negotiate_keep_alive(
            "POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\n"
        ));
        assert!(!negotiate_keep_alive(
            "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"
        ));
    }

    #[test]
    fn encoded_response_decodes_binary_bytes() {
        let context = RequestContext::new("GET", "/", std::ptr::null_mut());
        let content = CString::new("89504e4700ff").unwrap();
        let encoding = CString::new("hex").unwrap();
        assert!(unsafe {
            response_end_encoded(
                (&context.end_encoded as *const NativeClosure).cast(),
                content.as_ptr(),
                encoding.as_ptr(),
            )
        });
        assert_eq!(context.state.body, [0x89, 0x50, 0x4e, 0x47, 0x00, 0xff]);
        assert!(context.state.ended);
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
            slow.write_all(b"GET /slow HTTP/1.1\r\nHost: localhost\r\nConnection: close")
                .unwrap();
            let mut fast = TcpStream::connect(("127.0.0.1", port)).unwrap();
            fast.write_all(b"GET /fast HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
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

    /// Reads exactly one HTTP response (status line + headers + a body
    /// exactly `Content-Length` bytes long) off `stream` without relying
    /// on the connection closing -- the point of this helper is to work
    /// correctly on a kept-alive connection, unlike `read_to_string`.
    fn read_one_response(stream: &mut TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 256];
        let header_end = loop {
            if let Some(index) = buffer.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                break index + 4;
            }
            let length = stream.read(&mut chunk).unwrap();
            assert!(length > 0, "connection closed before headers completed");
            buffer.extend_from_slice(&chunk[..length]);
        };
        let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
        let content_length: usize = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("Content-Length")
                    .then(|| value.trim().parse().ok())?
            })
            .expect("response had no Content-Length");
        while buffer.len() < header_end + content_length {
            let length = stream.read(&mut chunk).unwrap();
            assert!(length > 0, "connection closed before body completed");
            buffer.extend_from_slice(&chunk[..length]);
        }
        String::from_utf8_lossy(&buffer[..header_end + content_length]).into_owned()
    }

    #[test]
    fn keep_alive_reuses_the_connection_for_a_second_request() {
        static CALLBACKS: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "C" fn callback(
            environment: *const c_void,
            request: *const IncomingMessage,
            response: *mut ServerResponse,
        ) -> bool {
            let body = CString::new(string_from_ptr((*request).url)).unwrap();
            let ended = response_end((*response).end.cast(), body.as_ptr());
            // Both requests share one `ServerState`, closing the
            // *listening* socket only after the second response is
            // built -- closing a `Server` never touches an already
            // in-flight `ConnectionState`, so this doesn't interfere
            // with either response actually being written; it's just
            // what lets `thaw_http_run_servers()` return once both are
            // done, the same way the slow/fast test above does.
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
            let mut stream = loop {
                match TcpStream::connect(("127.0.0.1", port)) {
                    Ok(stream) => break stream,
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            };
            // Neither request says `Connection: close` -- HTTP/1.1
            // defaults to keep-alive, so both should arrive on this same
            // TCP connection without it ever closing in between.
            stream
                .write_all(b"GET /first HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let first = read_one_response(&mut stream);
            stream
                .write_all(b"GET /second HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            let second = read_one_response(&mut stream);
            // The second request asked to close -- the stream should now
            // hit EOF rather than staying open.
            let mut trailing = Vec::new();
            stream.read_to_end(&mut trailing).unwrap();
            assert!(trailing.is_empty());
            (first, second)
        });
        thaw_http_run_servers();
        let (first, second) = client.join().unwrap();
        assert!(first.contains("Connection: keep-alive"));
        assert!(first.ends_with("/first"));
        assert!(second.contains("Connection: close"));
        assert!(second.ends_with("/second"));
        assert_eq!(CALLBACKS.load(Ordering::Acquire), 2);
        assert!(unsafe { &*state }.closed.load(Ordering::Acquire));
    }
}
