use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::raw::{c_char, c_void};
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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
    /// Registers a timer promise that settles after at least `milliseconds`.
    /// Used here only to bound how long the event loop blocks in `poll`, so
    /// the connection-deadline sweep runs on time.
    fn thaw_sleep_ms(milliseconds: u64) -> *mut c_void;
    fn thaw_promise_destroy(promise: *mut c_void);
}

const THAW_FD_READABLE: u8 = 1;
const THAW_FD_WRITABLE: u8 = 2;
const MAX_REQUEST_HEAD: usize = 64 * 1024;
const MAX_REQUEST_BODY: usize = 8 * 1024 * 1024;

/// How the request body's length is framed, parsed from the head once
/// `\r\n\r\n` is seen and then consulted on every subsequent read until
/// the body is fully in hand.
#[derive(Clone, Copy, PartialEq)]
enum BodyPlan {
    /// No body (no `Content-Length`, no `Transfer-Encoding: chunked`).
    None,
    /// Exactly this many bytes follow the head.
    Fixed(usize),
    /// `Transfer-Encoding: chunked`; ends at the `0\r\n\r\n` terminator.
    Chunked,
}

/// Parses the body framing from an already-complete request head.
/// `Transfer-Encoding: chunked` wins over `Content-Length` (per RFC
/// 9112); a `Content-Length` that isn't a plain number is treated as no
/// body.
fn parse_body_plan(head: &str) -> BodyPlan {
    let mut content_length = None;
    for line in head.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("Transfer-Encoding") {
            if value
                .to_ascii_lowercase()
                .split(',')
                .any(|token| token.trim() == "chunked")
            {
                return BodyPlan::Chunked;
            }
        } else if name.eq_ignore_ascii_case("Content-Length") {
            content_length = value.parse::<usize>().ok();
        }
    }
    match content_length {
        Some(length) if length > 0 => BodyPlan::Fixed(length),
        _ => BodyPlan::None,
    }
}

/// Decodes a `Transfer-Encoding: chunked` body (`<hex len>\r\n<bytes>\r\n`
/// ..., ending at a zero-length chunk). Returns the decoded body and how
/// many bytes of `bytes` it consumed (so a pipelined next request left
/// after the terminator isn't swallowed). `None` if `bytes` doesn't yet
/// contain a complete body, or is malformed.
fn decode_chunked_body(bytes: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut offset = 0;
    let mut body = Vec::new();
    loop {
        let rest = &bytes[offset..];
        let line_end = rest.windows(2).position(|pair| pair == b"\r\n")?;
        let size =
            usize::from_str_radix(std::str::from_utf8(&rest[..line_end]).ok()?.trim(), 16).ok()?;
        let after_size = &rest[line_end + 2..];
        if after_size.len() < size + 2 {
            return None;
        }
        if &after_size[size..size + 2] != b"\r\n" {
            return None;
        }
        offset += line_end + 2 + size + 2;
        if size == 0 {
            return Some((body, offset));
        }
        body.extend_from_slice(&after_size[..size]);
    }
}

/// How long a connection may take to send a complete request head (and,
/// between requests on a kept-alive connection, how long it may sit idle
/// before the next one) before the server closes it. Mirrors the purpose
/// of Node's `headersTimeout` / `keepAliveTimeout`. Overridable for
/// tests via `THAW_HTTP_HEADER_TIMEOUT_MS`.
fn header_timeout() -> Duration {
    static MS: OnceLock<u64> = OnceLock::new();
    Duration::from_millis(*MS.get_or_init(|| {
        std::env::var("THAW_HTTP_HEADER_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(60_000)
    }))
}

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
    /// Meaningless on a server request (Node only has it on a *client*
    /// response), and always 0; kept only because it was already in the
    /// declared `.d.ts` interface and dropping it would churn every
    /// handler's structural type annotation.
    status_code: f64,
    /// The fully-read request body as a UTF-8 string. `read_request`
    /// consumes `Content-Length` / `Transfer-Encoding: chunked` bytes
    /// before the handler runs, so this is complete by the time the
    /// handler sees it. Empty for a bodyless request.
    body: *const c_char,
    /// `request.on("data", cb)` / `request.on("end", cb)`. Registers a
    /// listener; the already-buffered body is replayed to it once the
    /// synchronous part of the handler returns (see
    /// `deliver_request_body_events`).
    on: *const NativeClosure,
    /// `request.bodyHex()` -- the raw request body as a lowercase hex
    /// string, computed on call. thaw strings are NUL-terminated, so
    /// `body` (and the `on("data")` chunk) can't carry an arbitrary byte
    /// sequence; this is the escape hatch for a binary body, mirroring
    /// `response.endEncoded(content, "hex")` on the way out.
    body_hex: *const NativeClosure,
    /// `request.bodyBytes()` -- the raw request body as a native byte
    /// array (`HirType::Bytes`, physically an `Array(F64)` handle). The
    /// first-class counterpart to `bodyHex()`: the handler gets the bytes
    /// directly, indexes / iterates / slices them, and hands them back
    /// through `response.endBytes(...)` without an encode/decode detour.
    body_bytes: *const NativeClosure,
}

#[repr(C)]
struct ServerResponse {
    status_code: f64,
    set_header: *const NativeClosure,
    end: *const NativeClosure,
    write: *const NativeClosure,
    end_encoded: *const NativeClosure,
    /// `response.write(bytes)` / `response.end(bytes)` for a native byte
    /// array (`HirType::Bytes`). Same streaming / buffering path as the
    /// string closures -- the only difference is the argument is decoded
    /// from an `Array(F64)` handle instead of a NUL-terminated C string,
    /// so an arbitrary byte sequence survives.
    write_bytes: *const NativeClosure,
    end_bytes: *const NativeClosure,
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
    /// Set by `finish_connection` when it tore the connection down while
    /// this context's handler was still running: `connection` is now
    /// null *and* there's nothing left to do, so any further `res.*`
    /// no-ops. (A plain null `connection` -- the one-shot helpers --
    /// still buffers normally; this flag is what tells the two apart.)
    abandoned: bool,
    state: ResponseState,
    response: ServerResponse,
    request: IncomingMessage,
    set_header: NativeClosure,
    write: NativeClosure,
    end: NativeClosure,
    end_encoded: NativeClosure,
    request_on: NativeClosure,
    request_body_hex: NativeClosure,
    request_body_bytes: NativeClosure,
    write_bytes: NativeClosure,
    end_bytes: NativeClosure,
    /// The raw request body, kept so `bodyHex()` can hex-encode it. Owns
    /// what `_body` decoded from.
    _raw_body: Vec<u8>,
    /// `bodyHex()`'s result, computed and stored on the first call so the
    /// returned pointer stays valid for the rest of the request without
    /// paying the 2x-body memory for a handler that never asks.
    _body_hex: Option<CString>,
    /// `request.on("data", cb)` callbacks, in registration order. Each is
    /// a raw closure pointer (`*const c_void` as `usize`), replayed once
    /// with the whole body by `deliver_request_body_events`.
    request_data_listeners: Vec<usize>,
    /// `request.on("end", cb)` callbacks, fired after the `data` ones.
    request_end_listeners: Vec<usize>,
    /// Set once `deliver_request_body_events` has run, so a listener
    /// registered late (after an `await`) doesn't get a second replay and
    /// a re-entrant `on(...)` during delivery is a no-op.
    body_events_delivered: bool,
    // Backing storage the `IncomingMessage` pointers borrow from; never
    // read through directly (hence the underscores), just kept alive.
    _method: CString,
    _url: CString,
    _body: CString,
}

impl RequestContext {
    fn new(
        method: &str,
        target: &str,
        body: &[u8],
        connection: *mut ConnectionState,
    ) -> Box<RequestContext> {
        let mut context = Box::new(RequestContext {
            connection,
            resumable: false,
            headers_sent: false,
            abandoned: false,
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
                write_bytes: std::ptr::null(),
                end_bytes: std::ptr::null(),
            },
            request: IncomingMessage {
                method: std::ptr::null(),
                url: std::ptr::null(),
                status_code: 0.0,
                body: std::ptr::null(),
                on: std::ptr::null(),
                body_hex: std::ptr::null(),
                body_bytes: std::ptr::null(),
            },
            request_on: NativeClosure {
                code: request_add_listener as *const c_void,
                context: std::ptr::null_mut(),
            },
            request_body_hex: NativeClosure {
                code: request_body_hex as *const c_void,
                context: std::ptr::null_mut(),
            },
            request_body_bytes: NativeClosure {
                code: request_body_bytes as *const c_void,
                context: std::ptr::null_mut(),
            },
            write_bytes: NativeClosure {
                code: response_write_bytes as *const c_void,
                context: std::ptr::null_mut(),
            },
            end_bytes: NativeClosure {
                code: response_end_bytes as *const c_void,
                context: std::ptr::null_mut(),
            },
            _raw_body: body.to_vec(),
            _body_hex: None,
            request_data_listeners: Vec::new(),
            request_end_listeners: Vec::new(),
            body_events_delivered: false,
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
            // Exposed as a string, so decode lossily and drop interior
            // NULs (which would otherwise truncate it). JSON / form /
            // text bodies -- the target use case -- are unaffected;
            // binary bodies are not supported through this field.
            _body: CString::new(String::from_utf8_lossy(body).replace('\0', "\u{fffd}"))
                .unwrap_or_default(),
        });
        // The box has a stable address now -- wire every self pointer.
        let context_ptr: *mut RequestContext = &mut *context;
        context.request.method = context._method.as_ptr();
        context.request.url = context._url.as_ptr();
        context.request.body = context._body.as_ptr();
        context.set_header.context = context_ptr.cast();
        context.write.context = context_ptr.cast();
        context.end.context = context_ptr.cast();
        context.end_encoded.context = context_ptr.cast();
        context.request_on.context = context_ptr.cast();
        context.request_body_hex.context = context_ptr.cast();
        context.request_body_bytes.context = context_ptr.cast();
        context.write_bytes.context = context_ptr.cast();
        context.end_bytes.context = context_ptr.cast();
        context.response.set_header = &context.set_header;
        context.response.end = &context.end;
        context.response.write = &context.write;
        context.response.end_encoded = &context.end_encoded;
        context.response.write_bytes = &context.write_bytes;
        context.response.end_bytes = &context.end_bytes;
        context.request.on = &context.request_on;
        context.request.body_hex = &context.request_body_hex;
        context.request.body_bytes = &context.request_body_bytes;
        context
    }
}

/// `request.bodyHex()` -- the raw request body as a lowercase hex string,
/// the lossless counterpart to the lossy `request.body` / `on("data")`
/// string. Empty for a bodyless request. Computed once and cached on the
/// context so the pointer outlives the call.
unsafe extern "C" fn request_body_hex(environment: *const c_void) -> *const c_char {
    let context = request_context(environment);
    if context._body_hex.is_none() {
        let mut hex = String::with_capacity(context._raw_body.len() * 2);
        for byte in &context._raw_body {
            use std::fmt::Write;
            let _ = write!(hex, "{byte:02x}");
        }
        context._body_hex = Some(CString::new(hex).unwrap_or_default());
    }
    context
        ._body_hex
        .as_ref()
        .map_or(std::ptr::null(), |value| value.as_ptr())
}

/// Number of bytes each element occupies in the native `Array(F64)`
/// payload, and the size of the leading `i64` length header -- the layout
/// `thaw-llvm`'s `compile_array_wrap` / element access and `thaw-std`'s
/// `json.rs` number-array bridge both assume.
const NATIVE_ARRAY_HEADER: usize = 8;
const NATIVE_ARRAY_ELEMENT: usize = 8;

/// Reads a `HirType::Bytes` argument (a one-word handle onto a native
/// `[len: i64][f64 * len]` buffer, exactly what `Buffer.from` / a byte
/// literal produce) back into raw bytes. Each element is truncated toward
/// zero and taken mod 256, matching `Buffer`'s own `ToUint8` coercion.
/// A null handle or null buffer (a failed allocation upstream) reads as
/// empty rather than faulting across the FFI boundary.
unsafe fn native_bytes_to_vec(handle: *const u8) -> Vec<u8> {
    if handle.is_null() {
        return Vec::new();
    }
    let buffer = (handle as *const *const u8).read();
    if buffer.is_null() {
        return Vec::new();
    }
    let length = (buffer as *const i64).read().max(0) as usize;
    (0..length)
        .map(|index| {
            let value = (buffer.add(NATIVE_ARRAY_HEADER + index * NATIVE_ARRAY_ELEMENT)
                as *const f64)
                .read();
            (value as i64 & 0xff) as u8
        })
        .collect()
}

/// Builds a native `HirType::Bytes` value from raw bytes: the
/// `[len: i64][f64 * len]` payload, then the one-word handle onto it that
/// every `Array`/`Bytes` value is (see `compile_array_wrap` /
/// `json.rs::wrap_array_handle`). Arena-allocated, so it lives as long as
/// every other heap value the handler sees. Returns null only if an
/// allocation fails.
fn native_bytes_from_slice(bytes: &[u8]) -> *mut u8 {
    let payload = thaw_arena::thaw_arena_alloc(
        NATIVE_ARRAY_HEADER + bytes.len() * NATIVE_ARRAY_ELEMENT,
        NATIVE_ARRAY_ELEMENT,
    );
    if payload.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        (payload as *mut i64).write(bytes.len() as i64);
        for (index, &byte) in bytes.iter().enumerate() {
            (payload.add(NATIVE_ARRAY_HEADER + index * NATIVE_ARRAY_ELEMENT) as *mut f64)
                .write(f64::from(byte));
        }
    }
    let handle = thaw_arena::thaw_arena_alloc(NATIVE_ARRAY_HEADER, NATIVE_ARRAY_ELEMENT);
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { (handle as *mut *mut u8).write(payload) };
    handle
}

/// `request.bodyBytes()` -- the raw request body as a first-class native
/// byte array (`HirType::Bytes`). The lossless, directly-usable
/// counterpart to `bodyHex()`: no hex round-trip, the handler indexes and
/// iterates the bytes as-is. A fresh handle per call (cheap: bodies here
/// are bounded by `MAX_REQUEST_BODY`), so a handler is free to mutate one
/// without disturbing another read.
unsafe extern "C" fn request_body_bytes(environment: *const c_void) -> *mut u8 {
    let context = request_context(environment);
    native_bytes_from_slice(&context._raw_body)
}

/// `response.write(bytes)` for a `HirType::Bytes` argument -- identical to
/// the string `response.write`, only the payload is decoded from a native
/// byte array so an arbitrary byte sequence goes out verbatim.
unsafe extern "C" fn response_write_bytes(environment: *const c_void, chunk: *const u8) -> bool {
    let context = request_context(environment);
    let bytes = native_bytes_to_vec(chunk);
    if stream_should_engage(context) {
        stream_write(context, &bytes);
    } else {
        context.state.body.extend(bytes);
    }
    true
}

/// `response.end(bytes)` for a `HirType::Bytes` argument -- the byte-exact
/// counterpart to `response.end(string)` / `response.endEncoded(hex,
/// "hex")`.
unsafe extern "C" fn response_end_bytes(environment: *const c_void, chunk: *const u8) -> bool {
    let context = request_context(environment);
    let bytes = native_bytes_to_vec(chunk);
    stream_or_buffer_end(context, bytes);
    true
}

/// `request.on("data" | "end", callback)`. Registers the callback; the
/// already-buffered body is replayed to every listener by
/// `deliver_request_body_events` once the synchronous part of the handler
/// returns. Any other event name is accepted and ignored (Node's
/// `IncomingMessage` emits `aborted`/`close`/`error` too, none of which
/// this buffered model produces).
unsafe extern "C" fn request_add_listener(
    environment: *const c_void,
    event: *const c_char,
    callback: *const c_void,
) -> bool {
    if callback.is_null() {
        return false;
    }
    let context = request_context(environment);
    match string_from_ptr(event).as_str() {
        "data" => context.request_data_listeners.push(callback as usize),
        "end" => context.request_end_listeners.push(callback as usize),
        _ => {}
    }
    true
}

/// Replays the fully-read request body to every `request.on("data", ...)`
/// listener as a single string chunk (skipped when the body is empty,
/// matching Node -- which never emits `data` for a bodyless request),
/// then fires every `request.on("end", ...)` listener. Idempotent: runs
/// at most once per request, right after the synchronous part of the
/// handler returns, so a handler that registers listeners at the top
/// (before any `await`) sees them fire regardless of registration order.
fn deliver_request_body_events(context: &mut RequestContext) {
    if context.body_events_delivered {
        return;
    }
    context.body_events_delivered = true;
    if context.request_data_listeners.is_empty() && context.request_end_listeners.is_empty() {
        return;
    }
    let data_listeners = std::mem::take(&mut context.request_data_listeners);
    let end_listeners = std::mem::take(&mut context.request_end_listeners);
    let body = context._body.clone();
    if !context._body.as_bytes().is_empty() {
        for callback in data_listeners {
            let callback = callback as *const c_void;
            unsafe {
                type Callback = unsafe extern "C" fn(*const c_void, *const c_char);
                let code = *(callback as *const *const c_void);
                let callback_fn: Callback = std::mem::transmute(code);
                callback_fn(callback, body.as_ptr());
            }
        }
    }
    for callback in end_listeners {
        let callback = callback as *const c_void;
        unsafe {
            type Callback = unsafe extern "C" fn(*const c_void);
            let code = *(callback as *const *const c_void);
            let callback_fn: Callback = std::mem::transmute(code);
            callback_fn(callback);
        }
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
    if bail_if_client_gone(context) {
        return;
    }
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
    if bail_if_client_gone(context) {
        return;
    }
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
    if bail_if_client_gone(context) {
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

/// Non-blocking check for the client having closed the read side. `peek`
/// doesn't consume, so it can't lose bytes a client did send.
fn client_hung_up(connection: &ConnectionState) -> bool {
    let mut probe = [0_u8; 1];
    match &connection.stream {
        Some(stream) => matches!(stream.peek(&mut probe), Ok(0)),
        None => true,
    }
}

/// The client vanished mid-response: close the socket (releasing its
/// descriptor) and stop watching it now. If an in-flight handler is
/// parked (a suspended `async` handler, or a streaming response between
/// chunks) it won't return to `res.*` for who knows how long, so tear
/// the connection down straight away too -- `finish_connection` frees
/// the `ConnectionState` (and any large buffered request body it holds)
/// and husk-defers the `RequestContext` the parked handler still
/// references; that handler's eventual `res.*` calls then no-op via
/// `abandoned`. A synchronous handler is still on the stack and about to
/// hit `bail_if_client_gone` itself, so its box is left for that.
fn mark_client_gone(connection: &mut ConnectionState) {
    connection.client_gone = true;
    if let Some(stream) = connection.stream.take() {
        let _ = stream.shutdown(Shutdown::Both);
        // `stream` drops here -- the fd is closed straight away rather
        // than lingering until the handler completes.
    }
    if connection.watcher != 0 {
        unsafe { thaw_runtime_unwatch_fd(connection.watcher) };
        connection.watcher = 0;
    }
    if connection.awaiting_handler || (connection.streaming && !connection.response_ended) {
        finish_connection(connection);
    }
}

/// Called from the response-side closures. Returns `true` -- meaning the
/// caller should stop and not touch `context` further -- when the
/// response can't be delivered: either the connection was already torn
/// down (this is a husk, `context.connection == null`), or the client
/// has hung up, in which case the connection is finished now (which also
/// defers this `RequestContext`'s free) rather than rendering / streaming
/// a response nobody will read.
fn bail_if_client_gone(context: &mut RequestContext) -> bool {
    if context.abandoned {
        return true;
    }
    if context.connection.is_null() {
        // A one-shot helper with no event loop: not abandoned, just no
        // connection to stream onto -- let the buffered path handle it.
        return false;
    }
    let connection = unsafe { &mut *context.connection };
    if connection.client_gone {
        finish_connection(connection);
        return true;
    }
    false
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
    body: &[u8],
    connection: *mut ConnectionState,
) -> CallbackOutcome {
    let mut context = RequestContext::new(method, target, body, connection);
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
    // Replay the buffered body to any `request.on("data"/"end", ...)`
    // listener the handler registered synchronously. An `end` listener
    // that finishes the response (`res.end(...)` from inside it) is the
    // whole point, so this runs before the `ended` check below.
    deliver_request_body_events(&mut context);
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
/// loop and so can only ever see a `Ready` outcome. They don't parse a
/// request body, so it's always empty here.
fn invoke_server_callback(callback: *const c_void, method: &str, target: &str) -> ResponseSpec {
    match run_server_callback(callback, method, target, &[], std::ptr::null_mut()) {
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
    /// `None` only after `mark_client_gone` took the socket to close its
    /// descriptor early. Every request/response-path use is past a
    /// `client_gone` check by then, so `socket()` is `Some` for them.
    stream: Option<TcpStream>,
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
    /// When set, the point by which the client must have sent a complete
    /// request head (or, on a kept-alive connection, its next request):
    /// `thaw_http_run_servers` closes the connection once it passes.
    /// Cleared while a handler is actually running or a response is
    /// streaming -- those phases are bounded by the handler, not by this
    /// read-side timeout.
    head_deadline: Option<Instant>,
    /// Byte offset in `request` where the body starts (just past the
    /// head's `\r\n\r\n`). 0 until the head is complete.
    head_end: usize,
    /// How the body length is framed, decided when the head completes.
    body_plan: BodyPlan,
    /// Total bytes of `request` the current request occupies (head +
    /// body). On keep-alive these are drained and anything past them --
    /// a pipelined next request -- is kept and processed in turn.
    consumed: usize,
    /// Set when the client is seen to have hung up while an `async`
    /// handler (or a streaming response) is still in flight: the socket
    /// is shut down and unwatched right away, and the handler's next
    /// `response.write`/`end` tears the connection down instead of
    /// rendering / streaming into a dead socket.
    client_gone: bool,
}

impl ConnectionState {
    /// The live socket. Only `None` after `mark_client_gone`.
    fn socket(&mut self) -> Option<&mut TcpStream> {
        self.stream.as_mut()
    }
}

/// Live `ConnectionState` box addresses, so `thaw_http_run_servers` can
/// sweep for ones whose `head_deadline` has passed. Entries are added by
/// `register_connection` and removed by `finish_connection`.
fn tracked_connections() -> &'static Mutex<Vec<usize>> {
    static CONNECTIONS: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
    CONNECTIONS.get_or_init(|| Mutex::new(Vec::new()))
}

fn track_connection(connection: *const ConnectionState) {
    tracked_connections()
        .lock()
        .unwrap()
        .push(connection as usize);
}

fn untrack_connection(connection: *const ConnectionState) {
    let address = connection as usize;
    tracked_connections()
        .lock()
        .unwrap()
        .retain(|tracked| *tracked != address);
}

/// Closes every tracked connection whose `head_deadline` has passed, and
/// returns the earliest still-pending deadline (so the caller can bound
/// how long it blocks before sweeping again).
fn sweep_idle_connections() -> Option<Instant> {
    let now = Instant::now();
    let tracked: Vec<usize> = tracked_connections().lock().unwrap().clone();
    let mut next = None::<Instant>;
    for address in tracked {
        let connection = unsafe { &mut *(address as *mut ConnectionState) };
        match connection.head_deadline {
            Some(deadline) if deadline <= now => finish_connection(connection),
            Some(deadline) => next = Some(next.map_or(deadline, |current| current.min(deadline))),
            None => {}
        }
    }
    next
}

fn register_server(state: *const ServerState) {
    let address = state as usize;
    let mut servers = active_servers().lock().unwrap();
    if !servers.contains(&address) {
        servers.push(address);
    }
}

/// Keeps a short timer promise alive so the event loop's `poll` never
/// blocks longer than `cap` while a connection deadline is pending --
/// without it, an idle watched socket would block `poll` indefinitely
/// and `sweep_idle_connections` would never run. Recreated (and the old
/// one destroyed) each call; passing `None` just clears it.
fn arm_idle_wakeup(next_deadline: Option<Instant>) {
    thread_local! {
        static IDLE_TIMER: std::cell::Cell<*mut c_void> = const { std::cell::Cell::new(std::ptr::null_mut()) };
    }
    IDLE_TIMER.with(|timer| {
        let existing = timer.replace(std::ptr::null_mut());
        if !existing.is_null() {
            unsafe { thaw_promise_destroy(existing) };
        }
        let Some(deadline) = next_deadline else {
            return;
        };
        let cap = Duration::from_millis(250);
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(cap)
            .max(Duration::from_millis(1));
        timer.set(unsafe { thaw_sleep_ms(wait.as_millis() as u64) });
    });
}

/// Drives all listeners registered by `Server.listen`. Generated native entry
/// points call this after the user's main function has returned.
#[no_mangle]
pub extern "C" fn thaw_http_run_servers() {
    loop {
        // Any handler continuation whose connection was torn down mid-flight
        // has fully unwound by now: free the husks it left behind.
        drain_pending_context_frees();
        arm_idle_wakeup(sweep_idle_connections());
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
                arm_idle_wakeup(None);
                return;
            }
        }
        if !unsafe { thaw_runtime_run_one_event() } {
            arm_idle_wakeup(None);
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
        stream: Some(stream),
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
        head_deadline: Some(Instant::now() + header_timeout()),
        head_end: 0,
        body_plan: BodyPlan::None,
        consumed: 0,
        client_gone: false,
    }));
    let watcher = unsafe {
        thaw_runtime_watch_fd(
            (*connection).stream.as_ref().unwrap().as_raw_fd(),
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
    track_connection(connection);
    ACTIVE_CONNECTIONS.fetch_add(1, Ordering::AcqRel);
    server.connections.fetch_add(1, Ordering::AcqRel);
}

extern "C" fn connection_ready(context: *mut u8, _events: i16) {
    let connection = unsafe { &mut *(context as *mut ConnectionState) };
    loop {
        if connection.awaiting_handler || (connection.streaming && !connection.response_ended) {
            // An `async` handler / streaming response is still in flight.
            // Its `response.write`/`end` drives things from here -- but a
            // readable event now may be the client hanging up. If so,
            // shut the socket and stop watching immediately (freeing the
            // descriptor); the handler's next `res.*` call then tears the
            // rest down via `bail_if_client_gone` rather than rendering /
            // streaming into a dead socket.
            if !connection.client_gone && client_hung_up(connection) {
                mark_client_gone(connection);
            } else if connection.streaming
                && !connection.response_ended
                && !connection.response.is_empty()
            {
                write_response(connection);
            }
            return;
        }
        if connection.response.is_empty() && !read_request(connection) {
            return;
        }
        // `write_response`, on a fully-sent keep-alive response, drains
        // the request it just answered and leaves any pipelined bytes in
        // `connection.request`. If a whole next request is already
        // buffered, loop and answer it now -- the readable event that
        // would otherwise trigger it may never come. Any other outcome
        // (closed, or still writing) ends this call; note `connection`
        // must not be touched after a `Closed`.
        match write_response(connection) {
            FlushResult::KeptAlive
                if connection
                    .request
                    .windows(4)
                    .any(|bytes| bytes == b"\r\n\r\n") => {}
            _ => return,
        }
    }
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
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("Connection") {
            for token in value.trim().split(',') {
                let token = token.trim();
                if token.eq_ignore_ascii_case("close") {
                    connection_close = true;
                } else if token.eq_ignore_ascii_case("keep-alive") {
                    connection_keep_alive = true;
                }
            }
        }
    }
    if http_1_1 {
        !connection_close
    } else {
        connection_keep_alive
    }
}

/// Parses a complete request head (bytes up to and including `\r\n\r\n`).
fn parse_head(head: &[u8]) -> (String, String, bool, BodyPlan) {
    let head = String::from_utf8_lossy(head);
    let mut request_line = head.lines().next().unwrap_or("").split_whitespace();
    let method = request_line.next().unwrap_or("GET").to_string();
    let target = request_line.next().unwrap_or("/").to_string();
    (
        method,
        target,
        negotiate_keep_alive(&head),
        parse_body_plan(&head),
    )
}

/// If `connection.request` (from `head_end` on) already holds a full
/// body per `connection.body_plan`, returns it decoded plus how many
/// bytes of the body region it spans (so pipelined bytes past it are
/// left in place). `None` means keep reading.
fn take_complete_body(connection: &ConnectionState) -> Option<(Vec<u8>, usize)> {
    let tail = &connection.request[connection.head_end..];
    match connection.body_plan {
        BodyPlan::None => Some((Vec::new(), 0)),
        BodyPlan::Fixed(length) => {
            (tail.len() >= length).then(|| (tail[..length].to_vec(), length))
        }
        BodyPlan::Chunked => decode_chunked_body(tail),
    }
}

/// Tries to turn what's currently buffered in `connection.request` into a
/// dispatched request, without reading the socket. Returns `Some(true)`
/// if a request was dispatched, `Some(false)` if the connection was torn
/// down (oversized / malformed), `None` if more bytes are needed.
fn try_buffered_request(connection: &mut ConnectionState) -> Option<bool> {
    if connection.head_end == 0 {
        let index = connection
            .request
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")?;
        connection.head_end = index + 4;
        let (_, _, _, body_plan) = parse_head(&connection.request[..connection.head_end]);
        connection.body_plan = body_plan;
    }
    // A `Content-Length` past the cap, or a chunked body that grows past
    // it, is refused -- and the connection can't be reused (unread body
    // bytes would desync it), so it's closed outright.
    let body_seen = connection.request.len() - connection.head_end;
    let over_cap = match connection.body_plan {
        BodyPlan::Fixed(length) => length > MAX_REQUEST_BODY,
        BodyPlan::Chunked => body_seen > MAX_REQUEST_BODY,
        BodyPlan::None => false,
    };
    if over_cap {
        finish_connection(connection);
        return Some(false);
    }
    let (body, body_len) = take_complete_body(connection)?;
    connection.consumed = connection.head_end + body_len;
    Some(dispatch_request(connection, body))
}

fn read_request(connection: &mut ConnectionState) -> bool {
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(dispatched) = try_buffered_request(connection) {
            return dispatched;
        }
        if connection.head_end == 0 && connection.request.len() > MAX_REQUEST_HEAD {
            finish_connection(connection);
            return false;
        }
        let Some(socket) = connection.socket() else {
            finish_connection(connection);
            return false;
        };
        match socket.read(&mut chunk) {
            Ok(0) => {
                finish_connection(connection);
                return false;
            }
            Ok(length) => connection.request.extend_from_slice(&chunk[..length]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return false,
            Err(_) => {
                finish_connection(connection);
                return false;
            }
        }
    }
}

/// The head and body are both fully in hand: run the handler.
fn dispatch_request(connection: &mut ConnectionState, body: Vec<u8>) -> bool {
    let (method, target, keep_alive, _) = parse_head(&connection.request[..connection.head_end]);
    let callback = unsafe { &*connection.server }.callback as *const c_void;
    // The whole request is in: the read-side timeout no longer applies --
    // the handler (and any streaming response) bounds its own lifetime.
    connection.head_deadline = None;
    // Set before running the handler: an `async` handler that suspends
    // won't return through here, and `finish_response` needs the
    // negotiated value when it renders the response later.
    connection.keep_alive = keep_alive;
    match run_server_callback(
        callback,
        &method,
        &target,
        &body,
        connection as *mut ConnectionState,
    ) {
        CallbackOutcome::Ready(spec) => {
            connection.response = render_response(spec, keep_alive);
            true
        }
        CallbackOutcome::Pending(context) => park_pending_context(connection, context, keep_alive),
    }
}

/// What `write_response` left the connection in.
enum FlushResult {
    /// The connection was torn down (write error, or a non-keep-alive
    /// response finished) -- `connection` is now freed.
    Closed,
    /// The response was fully sent and the connection reset for the next
    /// request; safe to look for a pipelined follow-up.
    KeptAlive,
    /// Still in progress: a slow client (would-block, re-armed for
    /// writability) or a streamed response waiting on its next chunk.
    Pending,
}

fn write_response(connection: &mut ConnectionState) -> FlushResult {
    while connection.written < connection.response.len() {
        let Some(stream) = connection.stream.as_mut() else {
            finish_connection(connection);
            return FlushResult::Closed;
        };
        match stream.write(&connection.response[connection.written..]) {
            Ok(0) => {
                finish_connection(connection);
                return FlushResult::Closed;
            }
            Ok(length) => connection.written += length,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return if rewatch_connection(connection, THAW_FD_WRITABLE) {
                    FlushResult::Pending
                } else {
                    FlushResult::Closed
                };
            }
            Err(_) => {
                finish_connection(connection);
                return FlushResult::Closed;
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
        return FlushResult::Pending;
    }
    if connection.keep_alive {
        // Reuse the connection instead of closing it: drain the request
        // just answered and reset the per-request fields for the next
        // one. `connection_ready` decides whether to read or write purely
        // from `response.is_empty()`, so clearing it here is what makes
        // the next readiness event re-enter `read_request`.
        //
        // Pipelining is supported: bytes past this request (a client's
        // already-sent next request) stay in `connection.request` and
        // `connection_ready` loops straight into them; responses still go
        // out in request order because each is fully sent before the
        // next request is read.
        free_response_context(connection);
        connection
            .request
            .drain(..connection.consumed.min(connection.request.len()));
        connection.consumed = 0;
        connection.response.clear();
        connection.written = 0;
        connection.awaiting_handler = false;
        connection.streaming = false;
        connection.response_ended = false;
        connection.head_end = 0;
        connection.body_plan = BodyPlan::None;
        // Waiting for the next request now: the read-side timeout applies
        // again (doubling as a keep-alive idle timeout).
        connection.head_deadline = Some(Instant::now() + header_timeout());
        return if rewatch_connection(connection, THAW_FD_READABLE) {
            FlushResult::KeptAlive
        } else {
            FlushResult::Closed
        };
    }
    finish_connection(connection);
    FlushResult::Closed
}

/// Drops the boxed `RequestContext` a still-in-flight (or just-completed)
/// `async` request left parked on the connection, if any.
fn free_response_context(connection: &mut ConnectionState) {
    if !connection.response_ctx.is_null() {
        unsafe { drop(Box::from_raw(connection.response_ctx)) };
        connection.response_ctx = std::ptr::null_mut();
    }
}

thread_local! {
    /// `RequestContext` boxes whose connection was torn down while their
    /// handler was still in flight. The handler may still call back into
    /// `res.*` synchronously as its continuation unwinds; those calls
    /// see `context.connection == null` and no-op. `thaw_http_run_servers`
    /// frees these on its next turn, once that continuation is done.
    static PENDING_CONTEXT_FREE: std::cell::RefCell<Vec<*mut RequestContext>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn drain_pending_context_frees() {
    let stale = PENDING_CONTEXT_FREE.with(|list| std::mem::take(&mut *list.borrow_mut()));
    for context in stale {
        unsafe { drop(Box::from_raw(context)) };
    }
}

/// Returns `false` if re-registering the fd watch failed and the
/// connection was torn down (so `connection` is now freed).
fn rewatch_connection(connection: &mut ConnectionState, interests: u8) -> bool {
    unsafe { thaw_runtime_unwatch_fd(connection.watcher) };
    connection.watcher = 0;
    let Some(fd) = connection.stream.as_ref().map(TcpStream::as_raw_fd) else {
        finish_connection(connection);
        return false;
    };
    connection.watcher = unsafe {
        thaw_runtime_watch_fd(
            fd,
            interests,
            connection_ready,
            (connection as *mut ConnectionState).cast(),
        )
    };
    if connection.watcher == 0 {
        finish_connection(connection);
        return false;
    }
    true
}

fn finish_connection(connection: &mut ConnectionState) {
    untrack_connection(connection);
    // An `async` handler that hasn't finished still holds a
    // `*mut ConnectionState` and may call `res.*` again as its
    // continuation unwinds -- don't free its `RequestContext` out from
    // under it. Null the back-pointer (so those calls no-op) and defer
    // the free to the next event-loop turn.
    if !connection.response_ctx.is_null() {
        unsafe {
            (*connection.response_ctx).connection = std::ptr::null_mut();
            (*connection.response_ctx).abandoned = true;
        }
        PENDING_CONTEXT_FREE.with(|list| list.borrow_mut().push(connection.response_ctx));
        connection.response_ctx = std::ptr::null_mut();
    }
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
    fn negotiates_keep_alive_by_version_and_connection_header() {
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
        // The request body is consumed now (`read_request` /
        // `take_complete_body`), so a `Content-Length` / chunked request
        // no longer forces the connection closed.
        assert!(negotiate_keep_alive(
            "POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\n"
        ));
        assert!(negotiate_keep_alive(
            "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"
        ));
        assert!(!negotiate_keep_alive(
            "POST / HTTP/1.1\r\nContent-Length: 4\r\nConnection: close\r\n\r\n"
        ));
    }

    #[test]
    fn parses_body_framing_from_the_head() {
        assert!(matches!(
            parse_body_plan("GET / HTTP/1.1\r\nHost: x\r\n\r\n"),
            BodyPlan::None
        ));
        assert!(matches!(
            parse_body_plan("POST / HTTP/1.1\r\nContent-Length: 11\r\n\r\n"),
            BodyPlan::Fixed(11)
        ));
        assert!(matches!(
            parse_body_plan("POST / HTTP/1.1\r\nContent-Length: 0\r\n\r\n"),
            BodyPlan::None
        ));
        assert!(matches!(
            parse_body_plan("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"),
            BodyPlan::Chunked
        ));
        // Transfer-Encoding wins over a (spec-forbidden) Content-Length.
        assert!(matches!(
            parse_body_plan(
                "POST / HTTP/1.1\r\nContent-Length: 9\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            BodyPlan::Chunked
        ));
    }

    #[test]
    fn decodes_a_chunked_request_body() {
        let input = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let (body, consumed) = decode_chunked_body(input).unwrap();
        assert_eq!(body, b"hello world");
        assert_eq!(consumed, input.len());
        // Bytes past the terminator (a pipelined request) aren't consumed.
        let (body, consumed) = decode_chunked_body(b"3\r\nabc\r\n0\r\n\r\nGET /next").unwrap();
        assert_eq!(body, b"abc");
        assert_eq!(consumed, b"3\r\nabc\r\n0\r\n\r\n".len());
        assert_eq!(decode_chunked_body(b"0\r\n\r\n"), Some((Vec::new(), 5)));
        // Incomplete: terminator not yet received.
        assert_eq!(decode_chunked_body(b"5\r\nhel"), None);
        assert_eq!(decode_chunked_body(b"5\r\nhello\r\n"), None);
    }

    #[test]
    fn encoded_response_decodes_binary_bytes() {
        let context = RequestContext::new("GET", "/", b"", std::ptr::null_mut());
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
