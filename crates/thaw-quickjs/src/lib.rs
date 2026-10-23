//! QuickJS-NG integration for the Fallback path (docs/design/bridge.md
//! section 7): runs arbitrary JS (e.g. an npm package's actual source, once
//! thaw-registry can fetch one) in an embedded engine, exchanging values
//! with compiled Thaw code as JSON text.
//!
//! `rquickjs` (which bundles QuickJS-NG -- the bnoordhuis/saghul fork the
//! design doc names, not Bellard's original) does the real work; this
//! crate is a thin C ABI shim around it, in the same spirit as
//! thaw-std/thaw-runtime.
//!
//! Argument/result marshaling goes through JSON both at the Rust/QuickJS
//! boundary (`thaw_js_call`'s `args_json`/return) and, one level up, at the
//! HIR/codegen boundary (`callDynamic`, see hir_codegen.rs), which
//! composes this crate's `thaw_js_call` with thaw-std's
//! `thaw_json_stringify`/`thaw_json_parse` so a `Json` value flows in and
//! out without this crate needing to know thaw-std's internal
//! representation. The host event loop drives QuickJS jobs and platform work,
//! and integrates native Promise callbacks through exported runtime hooks.
//!
//! Generated code uses `thaw_js_call_result` to route unknown functions,
//! thrown JS exceptions, malformed arguments, and Promise rejections through
//! Thaw's `try`/`catch`. The original `thaw_js_call` JSON-error-object API is
//! retained for C ABI compatibility with older callers.
#![allow(clippy::missing_safety_doc, clippy::not_unsafe_ptr_arg_deref)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::CString;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{Shutdown, TcpListener, TcpStream, UdpSocket};
use std::os::raw::{c_char, c_void};
#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::rc::Rc;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use thaw_arena::NativeStr as CStr;

// `thaw_js_dynamic_object_query` (api.rs) calls thaw-std's `thaw_json_parse`
// as an `extern "C"` declaration resolved at link time -- this crate has no
// real Rust-level dependency on thaw-std (see thaw-runtime's own
// `Cargo.toml` comment for why), so without this marker import `cargo test`
// never actually links thaw-std's rlib in and that symbol stays unresolved.
#[cfg(test)]
use thaw_std as _;
// thaw-std's own code calls a couple of thaw-runtime helpers the same
// "resolved at link time" way.
#[cfg(test)]
use thaw_runtime as _;

use base64::Engine as _;
use cbc::cipher::{
    block_padding::{NoPadding, Pkcs7},
    BlockDecryptMut, BlockEncryptMut, KeyIvInit,
};
use rquickjs::function::Args;
use rquickjs::Persistent; // also used by api.rs (N-API Symbol ops), not only by wasm
use rquickjs::{Array, ArrayBuffer, Context, Ctx, FromJs, Function, Object, Runtime, Value};
#[cfg(feature = "tls")]
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime,
};
#[cfg(feature = "tls")]
use rustls::server::WebPkiClientVerifier;
#[cfg(feature = "tls")]
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, Error as RustlsError, RootCertStore,
    ServerConfig, ServerConnection, SignatureScheme, StreamOwned,
};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};
#[cfg(feature = "wasm")]
use wasmi::Linker as WasmLinker;
#[cfg(feature = "wasm")]
use wasmi::{Caller as WasmCaller, Engine as WasmEngine, Extern as WasmExtern};
#[cfg(feature = "wasm")]
use wasmi::{Memory as WasmMemory, MemoryType as WasmMemoryType, Module as WasmModule};
#[cfg(feature = "wasm")]
use wasmi::{Store as WasmStore, Val as WasmVal, ValType as WasmValType};
#[cfg(feature = "wasm")]
use wasmi_wasi::sync::{ambient_authority, Dir as WasiDir, WasiCtxBuilder};
#[cfg(feature = "wasm")]
use wasmi_wasi::WasiCtx;

type NapiBridgeCallback = unsafe extern "C" fn(*const c_char, *const c_char) -> *const c_char;
type NapiBridgeExports = unsafe extern "C" fn() -> *const c_char;
type NapiBridgePoll = extern "C" fn() -> usize;
type NapiBridgePending = extern "C" fn() -> u8;
type NapiBridgeOwner = unsafe extern "C" fn(u64) -> *mut c_char;
type NapiBridgeLeaseRelease = extern "C" fn(u64) -> u8;
type NapiBridgeHandle = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
) -> *const c_char;
type NapiBridge = (
    NapiBridgeExports,
    NapiBridgeCallback,
    NapiBridgeHandle,
    NapiBridgePoll,
    NapiBridgePending,
    NapiBridgeOwner,
    NapiBridgeLeaseRelease,
);
static NAPI_BRIDGE: Mutex<Option<NapiBridge>> = Mutex::new(None);
// The worker's thread-local native Host must retire before its QuickJS Ctx.
// Keep shutdown separate from the existing JS call bridge ABI.
type NapiShutdownBridge = (
    extern "C" fn() -> u8,
    extern "C" fn() -> u8,
    extern "C" fn() -> *mut c_char,
    extern "C" fn() -> u8,
);
static NAPI_SHUTDOWN_BRIDGE: Mutex<Option<NapiShutdownBridge>> = Mutex::new(None);

pub fn register_napi_shutdown_bridge(
    begin: extern "C" fn() -> u8,
    poll: extern "C" fn() -> u8,
    take_error: extern "C" fn() -> *mut c_char,
    finish: extern "C" fn() -> u8,
) {
    *NAPI_SHUTDOWN_BRIDGE.lock().unwrap() = Some((begin, poll, take_error, finish));
}
/// Original terminal settings of every descriptor currently in raw mode (restored at exit).
#[cfg(unix)]
static ORIGINAL_STDIN_TERMIOS: Mutex<Vec<(libc::c_int, libc::termios)>> = Mutex::new(Vec::new());
#[cfg(unix)]
static STDIN_RESTORE_REGISTERED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

thread_local! {
    static ACTIVE_NAPI_CONTEXT: Cell<*const ()> = const { Cell::new(std::ptr::null()) };
}

struct ActiveNapiContext(*const ());

impl ActiveNapiContext {
    fn enter(ctx: &Ctx<'_>) -> Self {
        let previous =
            ACTIVE_NAPI_CONTEXT.with(|active| active.replace(ctx as *const Ctx<'_> as *const ()));
        Self(previous)
    }
}

impl Drop for ActiveNapiContext {
    fn drop(&mut self) {
        ACTIVE_NAPI_CONTEXT.with(|active| active.set(self.0));
    }
}

pub fn register_napi_bridge(
    exports: NapiBridgeExports,
    call: NapiBridgeCallback,
    handle: NapiBridgeHandle,
    poll: NapiBridgePoll,
    pending: NapiBridgePending,
    owner: NapiBridgeOwner,
    release: NapiBridgeLeaseRelease,
) {
    *NAPI_BRIDGE.lock().unwrap() = Some((exports, call, handle, poll, pending, owner, release));
}

fn install_napi_bridge(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    // PLATFORM_GLOBALS captures these functions before the first addon may load.
    // Resolve the registered bridge when called, so an earlier Ctx stays usable.
    let owner = Function::new(ctx.clone(), move |handle: String| {
        let Ok(handle) = handle.parse::<u64>() else {
            return String::new();
        };
        let owner = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.5);
        let Some(owner) = owner else {
            return String::new();
        };
        let result = unsafe { owner(handle) };
        if result.is_null() {
            return String::new();
        }
        let text = unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned();
        unsafe { thaw_arena::destroy_string(result) };
        text
    })?;
    ctx.globals().set("__thaw_napi_graph_owner", owner)?;
    let release = Function::new(ctx.clone(), move |ctx: Ctx<'_>, token: String| {
        let Ok(token) = token.parse::<u64>() else {
            return false;
        };
        let release = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.6);
        let _active = ActiveNapiContext::enter(&ctx);
        release.is_some_and(|release| release(token) != 0)
    })?;
    ctx.globals().set("__thaw_napi_graph_release", release)?;
    let available = Function::new(ctx.clone(), || NAPI_BRIDGE.lock().unwrap().is_some())?;
    ctx.globals()
        .set("__thaw_napi_bridge_available", available)?;
    let exports = Function::new(ctx.clone(), move || {
        let exports = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.0);
        exports.map_or_else(String::new, |exports| unsafe { to_str(exports()) })
    })?;
    let call = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, name: String, args: String| unsafe {
            let _active = ActiveNapiContext::enter(&ctx);
            let call = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.1);
            let Some(call) = call else {
                return String::new();
            };
            let name = CString::new(name).unwrap_or_default();
            let args = CString::new(args).unwrap_or_default();
            to_str(call(name.as_ptr(), args.as_ptr()))
        },
    )?;
    ctx.globals().set("__thaw_napi_bridge_exports", exports)?;
    ctx.globals().set("__thaw_napi_bridge_call", call)?;
    let handle = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, operation: String, target: String, name: String, args: String| unsafe {
            let _active = ActiveNapiContext::enter(&ctx);
            let handle = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.2);
            let Some(handle) = handle else {
                return String::new();
            };
            let operation = CString::new(operation).unwrap_or_default();
            let target = CString::new(target).unwrap_or_default();
            let name = CString::new(name).unwrap_or_default();
            let args = CString::new(args).unwrap_or_default();
            to_str(handle(
                operation.as_ptr(),
                target.as_ptr(),
                name.as_ptr(),
                args.as_ptr(),
            ))
        },
    )?;
    ctx.globals().set("__thaw_napi_bridge_handle", handle)?;
    Ok(())
}

fn napi_bridge_pending() -> bool {
    NAPI_BRIDGE
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|bridge| (bridge.4)() != 0)
}

fn poll_napi_bridge(ctx: &Ctx<'_>) {
    let poll = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.3);
    if let Some(poll) = poll {
        let _active = ActiveNapiContext::enter(ctx);
        poll();
    }
}

/// `process.nextTick` (`platform_globals/runtime.js`) queues into a
/// plain JS array rather than through `queueMicrotask`/`Promise.then`
/// -- real Node fully drains its own separate nextTick queue before
/// running *any* pending Promise microtask, at every checkpoint, and
/// nothing routed through the engine's real (opaque, native) Promise
/// job queue could ever jump ahead of a job already sitting in that
/// queue: whichever `.then()` calls first wins, always, no matter how
/// process.nextTick's own callback got there. Called right before
/// every `ctx.execute_pending_job()` check across this crate so a
/// nextTick queued while handling one microtask still runs before the
/// *next* one, matching Node's per-tick interleaving, not just once
/// per whole batch.
///
/// Returns whether it actually ran a callback -- a caller polling "is
/// some condition met yet, else give up" (`finish_with_platform_events`,
/// `thaw_js_run_until_native_resolved`) must `continue` its loop
/// immediately when this is `true` to re-check that condition, since a
/// nextTick callback may be the very thing that just resolved the
/// promise it's waiting on; falling through to an exhaustion check in
/// the same iteration first would misreport it as deadlocked.
fn drain_next_tick_queue(ctx: &Ctx<'_>) -> Result<bool, rquickjs::Error> {
    if let Ok(drain) = ctx
        .globals()
        .get::<_, Function>("__thaw_drain_next_tick_queue")
    {
        return drain.call::<_, bool>(());
    }
    Ok(false)
}
#[cfg(feature = "tls")]
type TlsStream = StreamOwned<ClientConnection, TcpStream>;
#[cfg(feature = "tls")]
type TlsStreamTable = (u32, HashMap<u32, TlsStream>);
#[cfg(feature = "tls")]
type TlsServerStream = StreamOwned<ServerConnection, TcpStream>;
#[cfg(feature = "tls")]
type TlsServerStreamTable = (u32, HashMap<u32, TlsServerStream>);

#[cfg(feature = "tls")]
struct TlsListener {
    socket: TcpListener,
    config: Arc<ServerConfig>,
    local_certificate: Vec<u8>,
    request_cert: bool,
}

#[cfg(feature = "tls")]
#[derive(Default)]
struct TlsCertificates {
    peer: Option<Vec<u8>>,
    /// The whole certificate chain the peer sent (leaf first), for `getPeerCertificate(true)`.
    peer_chain: Vec<Vec<u8>>,
    local: Option<Vec<u8>>,
    authorized: bool,
    authorization_error: Option<String>,
    chain_verified: bool,
}

#[cfg(feature = "tls")]
struct RecordingServerVerifier {
    roots: RootCertStore,
    defer_name: bool,
    allow_invalid: bool,
    result: Arc<Mutex<Option<String>>>,
    algorithms: rustls::crypto::WebPkiSupportedAlgorithms,
}

/// Node reports `socket.authorizationError` as the OpenSSL X509 verify code, not rustls's message.
#[cfg(feature = "tls")]
fn node_authorization_error(error: &RustlsError) -> String {
    use rustls::CertificateError as C;
    let RustlsError::InvalidCertificate(reason) = error else {
        return error.to_string();
    };
    match reason {
        C::UnknownIssuer => "UNABLE_TO_VERIFY_LEAF_SIGNATURE",
        C::Expired | C::ExpiredContext { .. } => "CERT_HAS_EXPIRED",
        C::NotValidYet | C::NotValidYetContext { .. } => "CERT_NOT_YET_VALID",
        C::Revoked => "CERT_REVOKED",
        C::BadSignature => "CERT_SIGNATURE_FAILURE",
        C::NotValidForName | C::NotValidForNameContext { .. } => "ERR_TLS_CERT_ALTNAME_INVALID",
        _ => return error.to_string(),
    }
    .to_string()
}

#[cfg(feature = "tls")]
impl std::fmt::Debug for RecordingServerVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RecordingServerVerifier")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "tls")]
impl rustls::client::danger::ServerCertVerifier for RecordingServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, RustlsError> {
        let validation = (|| {
            let parsed = rustls::server::ParsedCertificate::try_from(end_entity)?;
            rustls::client::verify_server_cert_signed_by_trust_anchor(
                &parsed,
                &self.roots,
                intermediates,
                now,
                self.algorithms.all,
            )?;
            if !self.defer_name {
                rustls::client::verify_server_name(&parsed, server_name)?;
            }
            Ok(())
        })();
        let error = validation.as_ref().err().map(node_authorization_error);
        if let Ok(mut result) = self.result.lock() {
            *result = error.clone();
        }
        match validation {
            Ok(()) => Ok(rustls::client::danger::ServerCertVerified::assertion()),
            Err(_error) if self.allow_invalid => {
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            }
            Err(error) => Err(error),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

include!("quickjs/compression.rs");

include!("quickjs/filesystem.rs");

#[cfg(feature = "wasm")]
include!("quickjs/wasm.rs");

fn run_quickjs_gc(ctx: Ctx<'_>) {
    ctx.run_gc();
}

enum HostChildCommand {
    Stdin(Vec<u8>),
    StdinEnd,
    /// Signal number plus a reply channel carrying whether the signal was delivered.
    Kill(i32, Sender<bool>),
}

enum HostChildEvent {
    Spawn(u32),
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Error {
        message: String,
        code: String,
    },
    /// The process itself ended; its output pipes may still hold data (`exit` fires now, `close` after EOF).
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    Exit {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

struct HostChild {
    commands: Sender<HostChildCommand>,
    events: Receiver<HostChildEvent>,
    thread: Option<JoinHandle<()>>,
    /// `ChildProcess.unref()` clears this: an unreferenced child no longer keeps the event loop alive.
    refed: bool,
}

struct HostChildTable {
    next_handle: u32,
    children: HashMap<u32, HostChild>,
}

enum HostStdinEvent {
    Data(Vec<u8>),
    End,
}

struct HostStdin {
    events: Receiver<HostStdinEvent>,
    thread: Option<JoinHandle<()>>,
}

enum HostWorkerCommand {
    Message(String),
    Stdin(String),
    StdinEnd,
    DirectMessage {
        payload: String,
        source: u32,
        request: u64,
        reply: Option<Sender<HostWorkerCommand>>,
    },
    DirectResult {
        request: u64,
        error: Option<String>,
    },
    PortMessage {
        port: String,
        payload: String,
    },
    Terminate,
}

enum HostWorkerEvent {
    Online,
    Message {
        payload: String,
        ports: Vec<String>,
    },
    Stdout(String),
    Stderr(String),
    DirectRequest {
        target: u32,
        source: u32,
        request: u64,
        payload: String,
        ports: Vec<String>,
    },
    ParentDirectResult {
        request: u64,
        error: Option<String>,
    },
    PortMessage {
        port: String,
        payload: String,
        ports: Vec<String>,
    },
    Error(String),
    Exit(i32),
}

struct HostWorker {
    commands: Sender<HostWorkerCommand>,
    events: Receiver<HostWorkerEvent>,
    thread: Option<JoinHandle<()>>,
    refed: bool,
    /// Set by `terminate()`; the Worker thread's engine interrupt handler watches it so a busy script stops too.
    terminate: Arc<std::sync::atomic::AtomicBool>,
}

struct HostWorkerStart {
    bundle_source: String,
    source: String,
    worker_data_json: String,
    config_json: String,
    thread_id: u32,
    shared_env: Arc<Mutex<HashMap<String, String>>>,
    terminate: Arc<std::sync::atomic::AtomicBool>,
}

struct HostWorkerTable {
    next_handle: u32,
    workers: HashMap<u32, HostWorker>,
    shared_env: Arc<Mutex<HashMap<String, String>>>,
}

include!("quickjs/networking.rs");

fn to_str(ptr: *const c_char) -> String {
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn hex_decode(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let text = std::str::from_utf8(pair).unwrap_or("00");
            u8::from_str_radix(text, 16).unwrap_or(0)
        })
        .collect()
}

fn hex_encode(value: &[u8]) -> String {
    value.iter().fold(String::new(), |mut output, byte| {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
        output
    })
}

#[cfg(unix)]
extern "C" fn restore_stdin_termios() {
    let saved = std::mem::take(&mut *ORIGINAL_STDIN_TERMIOS.lock().unwrap());
    for (fd, termios) in saved {
        unsafe {
            libc::tcsetattr(fd, libc::TCSANOW, &termios);
        }
    }
}

/// Switches a terminal descriptor into (or out of) raw mode; the original settings are restored at exit.
#[cfg(unix)]
fn set_fd_raw_mode(fd: libc::c_int, enabled: bool) -> bool {
    if !enabled {
        let saved = {
            let mut originals = ORIGINAL_STDIN_TERMIOS.lock().unwrap();
            originals
                .iter()
                .position(|(saved, _)| *saved == fd)
                .map(|index| originals.remove(index).1)
        };
        return match saved {
            Some(termios) => (unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) }) == 0,
            None => true,
        };
    }
    let mut termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
        return false;
    }
    {
        let mut originals = ORIGINAL_STDIN_TERMIOS.lock().unwrap();
        if !originals.iter().any(|(saved, _)| *saved == fd) {
            originals.push((fd, termios));
        }
    }
    unsafe {
        libc::cfmakeraw(&mut termios);
    }
    let changed = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } == 0;
    if changed && !STDIN_RESTORE_REGISTERED.swap(true, std::sync::atomic::Ordering::AcqRel) {
        unsafe {
            libc::atexit(restore_stdin_termios);
        }
    }
    changed
}

#[cfg(unix)]
fn set_stdin_raw_mode(enabled: bool) -> bool {
    set_fd_raw_mode(libc::STDIN_FILENO, enabled)
}

/// `tty.ReadStream.setRawMode` for the descriptor the stream was created on.
#[cfg(unix)]
fn set_tty_raw_mode(fd: i32, enabled: bool) -> bool {
    fd >= 0 && set_fd_raw_mode(fd, enabled)
}

/// Writes all `bytes` to `fd` (tty.WriteStream on a descriptor other than stdout/stderr).
#[cfg(unix)]
fn write_fd_bytes(fd: i32, bytes: &[u8]) -> std::io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        let count =
            unsafe { libc::write(fd, bytes[written..].as_ptr().cast(), bytes.len() - written) };
        if count < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        written += count as usize;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_tty_raw_mode(_fd: i32, _enabled: bool) -> bool {
    false
}

#[cfg(not(unix))]
fn write_fd_bytes(_fd: i32, _bytes: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "raw descriptor writes are unsupported on this platform",
    ))
}

#[cfg(not(unix))]
fn set_stdin_raw_mode(_enabled: bool) -> bool {
    false
}

/// Keccak-f[1600] (FIPS 202); `sha3-*` digests only, no extendable output.
fn keccak_f1600(state: &mut [u64; 25]) {
    const ROUND_CONSTANTS: [u64; 24] = [
        0x0000000000000001,
        0x0000000000008082,
        0x800000000000808a,
        0x8000000080008000,
        0x000000000000808b,
        0x0000000080000001,
        0x8000000080008081,
        0x8000000000008009,
        0x000000000000008a,
        0x0000000000000088,
        0x0000000080008009,
        0x000000008000000a,
        0x000000008000808b,
        0x800000000000008b,
        0x8000000000008089,
        0x8000000000008003,
        0x8000000000008002,
        0x8000000000000080,
        0x000000000000800a,
        0x800000008000000a,
        0x8000000080008081,
        0x8000000000008080,
        0x0000000080000001,
        0x8000000080008008,
    ];
    const ROTATIONS: [u32; 24] = [
        1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44,
    ];
    const LANES: [usize; 24] = [
        10, 7, 11, 17, 18, 3, 5, 16, 8, 21, 24, 4, 15, 23, 19, 13, 12, 2, 20, 14, 22, 9, 6, 1,
    ];
    for constant in ROUND_CONSTANTS {
        let mut columns = [0u64; 5];
        for (x, column) in columns.iter_mut().enumerate() {
            *column = state[x] ^ state[x + 5] ^ state[x + 10] ^ state[x + 15] ^ state[x + 20];
        }
        for x in 0..5 {
            let delta = columns[(x + 4) % 5] ^ columns[(x + 1) % 5].rotate_left(1);
            for y in 0..5 {
                state[x + 5 * y] ^= delta;
            }
        }
        let mut carried = state[1];
        for (lane, rotation) in LANES.into_iter().zip(ROTATIONS) {
            let next = state[lane];
            state[lane] = carried.rotate_left(rotation);
            carried = next;
        }
        for y in 0..5 {
            let row = [
                state[5 * y],
                state[5 * y + 1],
                state[5 * y + 2],
                state[5 * y + 3],
                state[5 * y + 4],
            ];
            for x in 0..5 {
                state[5 * y + x] = row[x] ^ (!row[(x + 1) % 5] & row[(x + 2) % 5]);
            }
        }
        state[0] ^= constant;
    }
}

fn sha3_digest(value: &[u8], output_bytes: usize) -> Vec<u8> {
    let rate = 200 - 2 * output_bytes;
    let mut state = [0u64; 25];
    let mut padded = value.to_vec();
    padded.push(0x06);
    padded.resize(padded.len().div_ceil(rate) * rate, 0);
    *padded.last_mut().expect("padded block") |= 0x80;
    for block in padded.chunks(rate) {
        for (lane, bytes) in state.iter_mut().zip(block.as_chunks::<8>().0) {
            *lane ^= u64::from_le_bytes(*bytes);
        }
        keccak_f1600(&mut state);
    }
    state
        .iter()
        .flat_map(|lane| lane.to_le_bytes())
        .take(output_bytes)
        .collect()
}

fn digest_bytes(algorithm: &str, value: &[u8]) -> Vec<u8> {
    match algorithm {
        "md5" => md5::Md5::digest(value).to_vec(),
        "sha1" => Sha1::digest(value).to_vec(),
        "sha224" => sha2::Sha224::digest(value).to_vec(),
        "sha256" => Sha256::digest(value).to_vec(),
        "sha384" => Sha384::digest(value).to_vec(),
        "sha512" => Sha512::digest(value).to_vec(),
        "sha3224" => sha3_digest(value, 28),
        "sha3256" => sha3_digest(value, 32),
        "sha3384" => sha3_digest(value, 48),
        "sha3512" => sha3_digest(value, 64),
        _ => Vec::new(),
    }
}

/// HMAC block size: the digest's input block (the SHA-3 sponge rate for `sha3*`).
fn hmac_block_size(algorithm: &str) -> usize {
    match algorithm {
        "sha384" | "sha512" => 128,
        "sha3224" => 144,
        "sha3256" => 136,
        "sha3384" => 104,
        "sha3512" => 72,
        _ => 64,
    }
}

fn hmac_bytes(algorithm: &str, key: &[u8], value: &[u8]) -> Vec<u8> {
    let block_size = hmac_block_size(algorithm);
    let mut normalized = if key.len() > block_size {
        digest_bytes(algorithm, key)
    } else {
        key.to_vec()
    };
    normalized.resize(block_size, 0);
    let inner_key = normalized
        .iter()
        .map(|byte| byte ^ 0x36)
        .collect::<Vec<_>>();
    let outer_key = normalized
        .iter()
        .map(|byte| byte ^ 0x5c)
        .collect::<Vec<_>>();
    let mut inner = inner_key;
    inner.extend_from_slice(value);
    let inner_digest = digest_bytes(algorithm, &inner);
    let mut outer = outer_key;
    outer.extend_from_slice(&inner_digest);
    digest_bytes(algorithm, &outer)
}

fn pbkdf2_bytes(
    algorithm: &str,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    length: usize,
) -> Vec<u8> {
    let digest_length = digest_bytes(algorithm, &[]).len();
    if digest_length == 0 || iterations == 0 {
        return Vec::new();
    }
    let mut output = Vec::with_capacity(length);
    for block in 1..=length.div_ceil(digest_length) {
        let mut input = salt.to_vec();
        input.extend_from_slice(&(block as u32).to_be_bytes());
        let mut value = hmac_bytes(algorithm, password, &input);
        let mut accumulated = value.clone();
        for _ in 1..iterations {
            value = hmac_bytes(algorithm, password, &value);
            for (byte, next) in accumulated.iter_mut().zip(&value) {
                *byte ^= next;
            }
        }
        output.extend_from_slice(&accumulated);
    }
    output.truncate(length);
    output
}

#[cfg(target_os = "linux")]
fn linux_cpu_times(stat: &str, ticks_per_second: u64) -> Vec<(usize, [u64; 5])> {
    if ticks_per_second == 0 {
        return Vec::new();
    }
    stat.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let index = fields.next()?.strip_prefix("cpu")?.parse::<usize>().ok()?;
            // Linux publishes user, nice, system, idle, iowait, irq in this
            // order. The guest columns are already included in user/nice.
            let raw = fields
                .take(6)
                .map(str::parse::<u64>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            if raw.len() != 6 {
                return None;
            }
            let milliseconds = |ticks: u64| {
                ((ticks as u128 * 1000) / ticks_per_second as u128).min(u64::MAX as u128) as u64
            };
            Some((
                index,
                [raw[0], raw[1], raw[2], raw[3], raw[5]].map(milliseconds),
            ))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_cpu_metadata(cpu_info: &str) -> std::collections::HashMap<usize, (String, u64)> {
    cpu_info
        .split("\n\n")
        .filter_map(|block| {
            let mut index = None;
            let mut model = None;
            let mut speed = None;
            for line in block.lines() {
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                match key.trim() {
                    "processor" => index = value.trim().parse::<usize>().ok(),
                    "model name" => model = Some(value.trim().to_string()),
                    "cpu MHz" => {
                        speed = value
                            .trim()
                            .parse::<f64>()
                            .ok()
                            .map(|mhz| mhz.round() as u64)
                    }
                    _ => {}
                }
            }
            Some((index?, (model.unwrap_or_default(), speed.unwrap_or(0))))
        })
        .collect()
}

#[cfg(all(test, target_os = "linux"))]
mod linux_cpu_info_tests {
    use super::{linux_cpu_metadata, linux_cpu_times};

    #[test]
    fn parses_numbered_cpus_in_milliseconds_without_affinity_or_guest_double_counting() {
        let stat = "cpu 999 999 999 999 999 999 999 999 999 999\n\
                    cpu1 25 50 75 100 125 150 175 200 225 250\n\
                    cpu3 250 500 750 1000 1250 1500 1750 2000 2250 2500\n";
        assert_eq!(
            linux_cpu_times(stat, 250),
            vec![
                (1, [100, 200, 300, 400, 600]),
                (3, [1000, 2000, 3000, 4000, 6000]),
            ]
        );
        assert_eq!(
            linux_cpu_times("cpu7 18446744073709551615 1 2 3 4 5 6 7 8 9\n", 100),
            vec![(7, [u64::MAX, 10, 20, 30, 50])]
        );
        assert!(linux_cpu_times(stat, 0).is_empty());
    }

    #[test]
    fn matches_core_metadata_by_processor_number() {
        let info = "processor : 3\nmodel name : fast\ncpu MHz : 3200.5\n\n\
                    processor : 1\nmodel name : efficient\ncpu MHz : 1700.25\n";
        let metadata = linux_cpu_metadata(info);
        assert_eq!(metadata.get(&1), Some(&("efficient".to_string(), 1700)));
        assert_eq!(metadata.get(&3), Some(&("fast".to_string(), 3201)));
    }
}

#[cfg(target_os = "macos")]
fn macos_cpu_times(ticks: [u32; 4], hz: u64) -> [u64; 5] {
    let milliseconds = |state: usize| ((ticks[state] as u128 * 1000) / hz as u128) as u64;
    [
        milliseconds(libc::CPU_STATE_USER as usize),
        milliseconds(libc::CPU_STATE_NICE as usize),
        milliseconds(libc::CPU_STATE_SYSTEM as usize),
        milliseconds(libc::CPU_STATE_IDLE as usize),
        0,
    ]
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn macos_processor_ticks_map_to_node_milliseconds() {
    assert_eq!(
        macos_cpu_times([25, 50, 75, 100], 250),
        [100, 400, 200, 300, 0]
    );
}

#[cfg(target_os = "macos")]
extern "C" {
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
}

#[cfg(target_os = "macos")]
fn macos_sysctl_bytes(name: &std::ffi::CStr) -> Option<Vec<u8>> {
    let mut size = 0usize;
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    if size == 0 || size > 4096 {
        return None;
    }
    let mut value = vec![0u8; size];
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    if size > value.len() {
        return None;
    }
    value.truncate(size);
    Some(value)
}

#[cfg(target_os = "macos")]
fn macos_cpus() -> Vec<serde_json::Value> {
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let Ok(hz) = u64::try_from(hz) else {
        return Vec::new();
    };
    if hz == 0 {
        return Vec::new();
    }
    let model = {
        macos_sysctl_bytes(c"machdep.cpu.brand_string").or_else(|| macos_sysctl_bytes(c"hw.model"))
    }
    .map(|bytes| {
        String::from_utf8_lossy(bytes.split(|byte| *byte == 0).next().unwrap_or(&[])).into_owned()
    })
    .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    let speed = macos_sysctl_bytes(c"hw.cpufrequency")
        .and_then(|bytes| bytes.try_into().ok().map(u64::from_ne_bytes))
        .unwrap_or(0)
        / 1_000_000;
    let mut count: libc::natural_t = 0;
    let mut raw: libc::processor_info_array_t = std::ptr::null_mut();
    let mut raw_count: libc::mach_msg_type_number_t = 0;
    let host = unsafe { libc::mach_host_self() };
    let result = unsafe {
        libc::host_processor_info(
            host,
            libc::PROCESSOR_CPU_LOAD_INFO,
            &mut count,
            &mut raw,
            &mut raw_count,
        )
    };
    unsafe { mach_port_deallocate(libc::mach_task_self(), host) };
    if result != 0 || raw.is_null() {
        return Vec::new();
    }
    let expected =
        (count as usize).checked_mul(std::mem::size_of::<libc::processor_cpu_load_info>());
    let actual = (raw_count as usize).checked_mul(std::mem::size_of::<libc::integer_t>());
    let mut cpus = Vec::new();
    if matches!((actual, expected), (Some(actual), Some(expected)) if actual >= expected) {
        let records = unsafe {
            std::slice::from_raw_parts(raw.cast::<libc::processor_cpu_load_info>(), count as usize)
        };
        for record in records {
            let [user, nice, sys, idle, irq] = macos_cpu_times(record.cpu_ticks, hz);
            cpus.push(serde_json::json!({ "model": model, "speed": speed,
                "times": { "user": user, "nice": nice, "sys": sys, "idle": idle, "irq": irq } }));
        }
    }
    if let Some(actual) = actual {
        unsafe {
            libc::vm_deallocate(
                libc::mach_task_self(),
                raw as libc::vm_address_t,
                actual as libc::vm_size_t,
            )
        };
    }
    cpus
}

#[cfg(target_os = "windows")]
fn windows_cpu_times(user: i64, kernel: i64, idle: i64, interrupt: i64) -> [u64; 5] {
    let ms = |ticks: i64| ticks.max(0) as u64 / 10_000;
    [
        ms(user),
        0,
        ms(kernel.saturating_sub(idle)),
        ms(idle),
        ms(interrupt),
    ]
}

#[cfg(all(test, target_os = "windows"))]
#[test]
fn windows_processor_ticks_exclude_idle_from_system_time() {
    assert_eq!(
        windows_cpu_times(50_000, 90_000, 40_000, 20_000),
        [5, 0, 5, 4, 2]
    );
}

#[cfg(target_os = "windows")]
fn windows_cpus() -> Vec<serde_json::Value> {
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct ProcessorTimes {
        idle: i64,
        kernel: i64,
        user: i64,
        dpc: i64,
        interrupt: i64,
        interrupt_count: u32,
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQuerySystemInformation(
            class: u32,
            data: *mut std::ffi::c_void,
            length: u32,
            returned: *mut u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetActiveProcessorCount(group: u16) -> u32;
    }
    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            parent: *mut std::ffi::c_void,
            path: *const u16,
            options: u32,
            access: u32,
            key: *mut *mut std::ffi::c_void,
        ) -> i32;
        fn RegQueryValueExW(
            key: *mut std::ffi::c_void,
            name: *const u16,
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            bytes: *mut u32,
        ) -> i32;
        fn RegCloseKey(key: *mut std::ffi::c_void) -> i32;
    }
    let identity = |index: usize| {
        let path = format!("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\{index}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let name = "ProcessorNameString\0".encode_utf16().collect::<Vec<_>>();
        let frequency = "~MHz\0".encode_utf16().collect::<Vec<_>>();
        let mut key = std::ptr::null_mut();
        let root = (0x8000_0002u32 as i32 as isize) as *mut std::ffi::c_void;
        if unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, 1, &mut key) } != 0 {
            return (std::env::consts::ARCH.to_string(), 0u64);
        }
        let mut brand = [0u16; 256];
        let mut brand_bytes = std::mem::size_of_val(&brand) as u32;
        let mut brand_kind = 0u32;
        let brand_ok = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut brand_kind,
                brand.as_mut_ptr().cast(),
                &mut brand_bytes,
            )
        } == 0
            && brand_kind == 1
            && brand_bytes as usize <= std::mem::size_of_val(&brand)
            && brand_bytes % 2 == 0;
        let mut mhz = 0u32;
        let mut mhz_bytes = std::mem::size_of_val(&mhz) as u32;
        let mut mhz_kind = 0u32;
        let mhz_ok = unsafe {
            RegQueryValueExW(
                key,
                frequency.as_ptr(),
                std::ptr::null_mut(),
                &mut mhz_kind,
                (&mut mhz as *mut u32).cast(),
                &mut mhz_bytes,
            )
        } == 0
            && mhz_kind == 4
            && mhz_bytes as usize == std::mem::size_of_val(&mhz);
        unsafe {
            RegCloseKey(key);
        }
        let model = if brand_ok {
            let length = (brand_bytes as usize / 2).min(brand.len());
            let end = brand[..length]
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(length);
            String::from_utf16_lossy(&brand[..end])
        } else {
            std::env::consts::ARCH.to_string()
        };
        (model, if mhz_ok { mhz as u64 } else { 0 })
    };
    let mut capacity = unsafe { GetActiveProcessorCount(0xffff) }.max(1) as usize;
    let records = loop {
        let mut values = vec![ProcessorTimes::default(); capacity];
        let Some(bytes) = capacity
            .checked_mul(std::mem::size_of::<ProcessorTimes>())
            .and_then(|value| u32::try_from(value).ok())
        else {
            return Vec::new();
        };
        let mut returned = 0u32;
        let status = unsafe {
            NtQuerySystemInformation(8, values.as_mut_ptr().cast(), bytes, &mut returned)
        };
        if status >= 0 {
            if returned == 0
                || returned > bytes
                || returned as usize % std::mem::size_of::<ProcessorTimes>() != 0
            {
                return Vec::new();
            }
            values.truncate(returned as usize / std::mem::size_of::<ProcessorTimes>());
            break values;
        }
        if status != 0xc000_0004u32 as i32 && status != 0xc000_0023u32 as i32 {
            return Vec::new();
        }
        // Some Windows versions do not populate ReturnLength on a short buffer.
        let required = (returned as usize).div_ceil(std::mem::size_of::<ProcessorTimes>());
        let next = required.max(capacity.saturating_mul(2));
        if next <= capacity || next > 65_536 {
            return Vec::new();
        }
        capacity = next;
    };
    records
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            let [user, nice, sys, idle, irq] =
                windows_cpu_times(record.user, record.kernel, record.idle, record.interrupt);
            let (model, speed) = identity(index);
            serde_json::json!({ "model": model, "speed": speed,
            "times": { "user": user, "nice": nice, "sys": sys, "idle": idle, "irq": irq } })
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn macos_memory_bytes(free_pages: u64, page_size: u64) -> u64 {
    free_pages.saturating_mul(page_size)
}

#[cfg(target_os = "macos")]
fn macos_uptime_seconds(now: f64, boot: f64) -> f64 {
    (now - boot).max(0.0)
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn macos_dynamic_units_are_bytes_and_seconds() {
    assert_eq!(macos_memory_bytes(25, 16_384), 409_600);
    assert_eq!(macos_uptime_seconds(1_234.5, 1_000.0), 234.5);
    assert_eq!(macos_uptime_seconds(900.0, 1_000.0), 0.0);
}

#[cfg(target_os = "macos")]
fn macos_dynamic_info() -> std::io::Result<(u64, u64, f64, Vec<f64>)> {
    let total = macos_sysctl_bytes(c"hw.memsize")
        .and_then(|bytes| bytes.try_into().ok().map(u64::from_ne_bytes))
        .ok_or_else(|| std::io::Error::other("cannot read hw.memsize"))?;
    let pagesize = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let pagesize = u64::try_from(pagesize)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| std::io::Error::other("cannot read page size"))?;
    let mut info: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let host = unsafe { libc::mach_host_self() };
    let status = unsafe {
        libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            (&mut info as *mut libc::vm_statistics64).cast(),
            &mut count,
        )
    };
    unsafe { mach_port_deallocate(libc::mach_task_self(), host) };
    if status != 0 || count < libc::HOST_VM_INFO64_COUNT {
        return Err(std::io::Error::other("cannot read host VM statistics"));
    }
    let free = macos_memory_bytes(info.free_count as u64, pagesize);
    let boot = macos_sysctl_bytes(c"kern.boottime")
        .filter(|bytes| bytes.len() == std::mem::size_of::<libc::timeval>())
        .ok_or_else(|| std::io::Error::other("cannot read kern.boottime"))?;
    let boot = unsafe { std::ptr::read_unaligned(boot.as_ptr().cast::<libc::timeval>()) };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| std::io::Error::other(error.to_string()))?
        .as_secs_f64();
    let uptime = macos_uptime_seconds(now, boot.tv_sec as f64 + boot.tv_usec as f64 / 1_000_000.0);
    let mut averages = [0.0f64; 3];
    if unsafe { libc::getloadavg(averages.as_mut_ptr(), 3) } != 3 {
        return Err(std::io::Error::other("cannot read load averages"));
    }
    Ok((total, free, uptime, averages.to_vec()))
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn macos_dynamic_info_reads_native_current_values() {
    let (total, free, uptime, load) = macos_dynamic_info().unwrap();
    assert!(total > 0 && free <= total);
    assert!(uptime > 0.0);
    assert_eq!(load.len(), 3);
    assert!(load.iter().all(|value| value.is_finite() && *value >= 0.0));
}

#[cfg(target_os = "windows")]
fn windows_uptime_seconds(milliseconds: u64) -> f64 {
    milliseconds as f64 / 1000.0
}

#[cfg(all(test, target_os = "windows"))]
#[test]
fn windows_uptime_uses_milliseconds_and_load_is_unavailable() {
    assert_eq!(windows_uptime_seconds(12_345), 12.345);
}

#[cfg(target_os = "windows")]
fn windows_dynamic_info() -> std::io::Result<(u64, u64, f64, Vec<f64>)> {
    #[repr(C)]
    #[derive(Default)]
    struct MemoryStatus {
        length: u32,
        memory_load: u32,
        total_physical: u64,
        available_physical: u64,
        total_page_file: u64,
        available_page_file: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
        fn GetTickCount64() -> u64;
    }
    let mut status = MemoryStatus::default();
    status.length = std::mem::size_of::<MemoryStatus>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // libuv reports no load averages on Windows.
    Ok((
        status.total_physical,
        status.available_physical,
        windows_uptime_seconds(unsafe { GetTickCount64() }),
        vec![0.0; 3],
    ))
}

#[cfg(all(test, target_os = "windows"))]
#[test]
fn windows_dynamic_info_reads_memory_and_uptime_with_no_load_average() {
    let (total, free, uptime, load) = windows_dynamic_info().unwrap();
    assert!(total > 0 && free <= total);
    assert!(uptime > 0.0);
    assert_eq!(load, vec![0.0; 3]);
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn os_info_json() -> rquickjs::Result<String> {
    Err(rquickjs::Error::new_from_js_message(
        "OS info",
        "supported host statistics",
        "unsupported operating system",
    ))
}

fn os_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        value => value,
    }
}

fn os_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        value => value,
    }
}

/// `os.tmpdir()`: the first non-empty of TMPDIR, TMP, TEMP, else `/tmp`,
/// without a trailing slash (unless the path is just `/`).
fn os_tmpdir_from(variable: impl Fn(&str) -> Option<String>) -> String {
    let mut path = ["TMPDIR", "TMP", "TEMP"]
        .iter()
        .find_map(|name| variable(name).filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "/tmp".to_string());
    if path.len() > 1 && path.ends_with('/') {
        path.pop();
    }
    path
}

/// `os.userInfo()` describes the effective user, as `getpwuid_r(geteuid())`
/// sees it; environment variables are only a fallback when the password
/// database has no entry.
#[cfg(unix)]
fn os_user_info(home_fallback: &str) -> serde_json::Value {
    use std::ffi::CStr;
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut passwd,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    let text = |pointer: *const libc::c_char| {
        if pointer.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(pointer) }
                .to_string_lossy()
                .into_owned()
        }
    };
    if status == 0 && !found.is_null() {
        return serde_json::json!({
            "username": text(passwd.pw_name), "homedir": text(passwd.pw_dir),
            "shell": text(passwd.pw_shell), "uid": uid, "gid": gid,
        });
    }
    serde_json::json!({
        "username": std::env::var("USER").unwrap_or_default(), "homedir": home_fallback,
        "shell": std::env::var("SHELL").unwrap_or_default(), "uid": uid, "gid": gid,
    })
}

#[cfg(not(unix))]
fn os_user_info(home_fallback: &str) -> serde_json::Value {
    serde_json::json!({
        "username": std::env::var("USER").unwrap_or_default(), "homedir": home_fallback,
        "shell": std::env::var("SHELL").unwrap_or_default(), "uid": 0, "gid": 0,
    })
}

fn os_static_info() -> serde_json::Value {
    let platform = os_platform();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let tmp = os_tmpdir_from(|name| std::env::var(name).ok());
    let mut hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "localhost".to_string())
        .trim()
        .to_string();
    if hostname.is_empty() {
        hostname = "localhost".to_string();
    }
    serde_json::json!({
        "platform": platform,
        "arch": os_arch(),
        "type": if platform == "linux" { "Linux" } else { platform },
        "hostname": hostname, "homedir": home, "tmpdir": tmp,
        "release": std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim(),
        "version": std::fs::read_to_string("/proc/sys/kernel/version").unwrap_or_default().trim(),
        "machine": std::env::consts::ARCH,
        "endianness": if cfg!(target_endian = "little") { "LE" } else { "BE" },
        "userInfo": os_user_info(&home),
        "glibcVersionRuntime": glibc_version_runtime(),
    })
}

fn os_identity_json() -> String {
    os_static_info().to_string()
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn os_info_json() -> rquickjs::Result<String> {
    let arch = os_arch();
    #[cfg(target_os = "linux")]
    let cpu_info = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    #[cfg(target_os = "linux")]
    let model = cpu_info
        .lines()
        .find_map(|line| line.strip_prefix("model name\t: "))
        .unwrap_or(arch)
        .to_string();
    #[cfg(target_os = "linux")]
    let speed = cpu_info
        .lines()
        .find_map(|line| line.strip_prefix("cpu MHz\t\t: "))
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0)
        .round() as u64;
    #[cfg(target_os = "linux")]
    let cpus = {
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();
        let metadata = linux_cpu_metadata(&cpu_info);
        linux_cpu_times(&stat, u64::try_from(hz).unwrap_or(0))
            .into_iter()
            .map(|(index, [user, nice, sys, idle, irq])| {
                let (core_model, core_speed) = metadata
                    .get(&index)
                    .map(|(name, mhz)| {
                        (
                            if name.is_empty() {
                                model.as_str()
                            } else {
                                name.as_str()
                            },
                            *mhz,
                        )
                    })
                    .unwrap_or((model.as_str(), speed));
                let frequency = std::fs::read_to_string(format!(
                    "/sys/devices/system/cpu/cpu{index}/cpufreq/scaling_max_freq"
                ))
                .ok()
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(|khz| khz / 1000)
                .unwrap_or(core_speed);
                serde_json::json!({ "model": core_model, "speed": frequency,
                    "times": { "user": user, "nice": nice, "sys": sys, "idle": idle, "irq": irq } })
            })
            .collect::<Vec<_>>()
    };
    #[cfg(target_os = "macos")]
    let cpus = macos_cpus();
    #[cfg(target_os = "windows")]
    let cpus = windows_cpus();
    #[cfg(target_os = "linux")]
    let (totalmem, freemem, uptime, loadavg) = {
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let memory_value = |name: &str| {
            meminfo
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0)
                * 1024
        };
        let uptime = std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|value| value.split_whitespace().next()?.parse::<f64>().ok())
            .unwrap_or(0.0);
        let loadavg = std::fs::read_to_string("/proc/loadavg")
            .unwrap_or_default()
            .split_whitespace()
            .take(3)
            .filter_map(|value| value.parse::<f64>().ok())
            .collect::<Vec<_>>();
        (
            memory_value("MemTotal:"),
            memory_value("MemAvailable:"),
            uptime,
            loadavg,
        )
    };
    #[cfg(target_os = "macos")]
    let (totalmem, freemem, uptime, loadavg) = macos_dynamic_info().map_err(|error| {
        rquickjs::Error::new_from_js_message("OS info", "macOS host statistics", error.to_string())
    })?;
    #[cfg(target_os = "windows")]
    let (totalmem, freemem, uptime, loadavg) = windows_dynamic_info().map_err(|error| {
        rquickjs::Error::new_from_js_message(
            "OS info",
            "Windows host statistics",
            error.to_string(),
        )
    })?;
    let mut info = os_static_info();
    let fields = info
        .as_object_mut()
        .expect("static OS identity is an object");
    fields.insert("cpus".to_string(), serde_json::json!(cpus));
    fields.insert(
        "availableParallelism".to_string(),
        serde_json::json!(std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)),
    );
    fields.insert("totalmem".to_string(), serde_json::json!(totalmem));
    fields.insert("freemem".to_string(), serde_json::json!(freemem));
    fields.insert("uptime".to_string(), serde_json::json!(uptime));
    fields.insert("loadavg".to_string(), serde_json::json!(loadavg));
    Ok(info.to_string())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn os_priority_errno() -> *mut libc::c_int {
    #[cfg(target_os = "linux")]
    {
        unsafe { libc::__errno_location() }
    }
    #[cfg(target_os = "macos")]
    {
        unsafe { libc::__error() }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn posix_priority_result(priority: i32, errno: i32) -> std::io::Result<i32> {
    if priority == -1 && errno != 0 {
        Err(std::io::Error::from_raw_os_error(errno))
    } else {
        Ok(priority)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn native_get_priority(pid: i32) -> std::io::Result<i32> {
    // -1 is a valid nice value, so errno must be cleared before the call.
    let errno = os_priority_errno();
    unsafe { *errno = 0 };
    let priority = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t) };
    posix_priority_result(priority, unsafe { *errno })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn native_set_priority(pid: i32, priority: i32) -> std::io::Result<()> {
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, priority) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "windows")]
fn windows_priority_class(priority: i32) -> u32 {
    if priority < -14 {
        0x100
    }
    // REALTIME_PRIORITY_CLASS
    else if priority < -7 {
        0x80
    }
    // HIGH_PRIORITY_CLASS
    else if priority < 0 {
        0x8000
    }
    // ABOVE_NORMAL_PRIORITY_CLASS
    else if priority < 10 {
        0x20
    }
    // NORMAL_PRIORITY_CLASS
    else if priority < 19 {
        0x4000
    }
    // BELOW_NORMAL_PRIORITY_CLASS
    else {
        0x40
    } // IDLE_PRIORITY_CLASS
}

#[cfg(target_os = "windows")]
fn windows_nice_value(priority_class: u32) -> std::io::Result<i32> {
    match priority_class {
        0x100 => Ok(-20),
        0x80 => Ok(-14),
        0x8000 => Ok(-7),
        0x20 => Ok(0),
        0x4000 => Ok(10),
        0x40 => Ok(19),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unrecognized Windows priority class",
        )),
    }
}

#[cfg(target_os = "windows")]
fn windows_priority_handle(pid: i32, access: u32) -> std::io::Result<*mut std::ffi::c_void> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
    }
    let handle = if pid == 0 {
        unsafe { GetCurrentProcess() }
    } else {
        unsafe { OpenProcess(access, 0, pid as u32) }
    };
    if handle.is_null() {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

#[cfg(target_os = "windows")]
fn windows_close_priority_handle(pid: i32, handle: *mut std::ffi::c_void) {
    if pid != 0 {
        #[link(name = "kernel32")]
        extern "system" {
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        unsafe { CloseHandle(handle) };
    }
}

#[cfg(target_os = "windows")]
fn native_get_priority(pid: i32) -> std::io::Result<i32> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetPriorityClass(handle: *mut std::ffi::c_void) -> u32;
    }
    let handle = windows_priority_handle(pid, 0x1000)?; // PROCESS_QUERY_LIMITED_INFORMATION
    let priority_class = unsafe { GetPriorityClass(handle) };
    let result = if priority_class == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        windows_nice_value(priority_class)
    };
    windows_close_priority_handle(pid, handle);
    result
}

#[cfg(target_os = "windows")]
fn native_set_priority(pid: i32, priority: i32) -> std::io::Result<()> {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetPriorityClass(handle: *mut std::ffi::c_void, priority_class: u32) -> i32;
    }
    let handle = windows_priority_handle(pid, 0x0200)?; // PROCESS_SET_INFORMATION
    let result = if unsafe { SetPriorityClass(handle, windows_priority_class(priority)) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    };
    windows_close_priority_handle(pid, handle);
    result
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn native_get_priority(_pid: i32) -> std::io::Result<i32> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "OS priority is unsupported",
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn native_set_priority(_pid: i32, _priority: i32) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "OS priority is unsupported",
    ))
}

fn os_priority_result(result: std::io::Result<i32>) -> String {
    match result {
        Ok(value) => serde_json::json!({ "ok": true, "value": value }).to_string(),
        Err(error) => {
            let errno = error.raw_os_error();
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            let code = match errno {
                Some(value) if value == libc::ESRCH => "ESRCH",
                Some(value) if value == libc::EACCES => "EACCES",
                Some(value) if value == libc::EPERM => "EPERM",
                _ => "UNKNOWN",
            };
            #[cfg(target_os = "windows")]
            let code = match errno {
                Some(87) => "ESRCH",
                Some(5) => "EACCES",
                _ => "UNKNOWN",
            };
            #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
            let code = "ENOSYS";
            let code = if errno.is_none() {
                match error.kind() {
                    std::io::ErrorKind::Unsupported => "ENOSYS",
                    std::io::ErrorKind::InvalidInput => "EINVAL",
                    _ => code,
                }
            } else {
                code
            };
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            let uv_errno = errno.map(|value| -value).unwrap_or(match code {
                "EINVAL" => -libc::EINVAL,
                "ENOSYS" => -libc::ENOSYS,
                _ => -4094,
            });
            #[cfg(target_os = "windows")]
            let uv_errno = match code {
                "ESRCH" => -4040,
                "EACCES" => -4092,
                "EINVAL" => -4071,
                "ENOSYS" => -4054,
                _ => -4094,
            };
            #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
            let uv_errno = -4054;
            serde_json::json!({ "ok": false, "code": code, "errno": uv_errno,
                "message": error.to_string() })
            .to_string()
        }
    }
}

fn os_get_priority_json(pid: i32) -> String {
    os_priority_result(native_get_priority(pid))
}

fn os_set_priority_json(pid: i32, priority: i32) -> String {
    if !(-20..=19).contains(&priority) {
        return os_priority_result(Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "priority must be between -20 and 19",
        )));
    }
    os_priority_result(native_set_priority(pid, priority).map(|()| 0))
}

#[cfg(all(test, target_os = "windows"))]
#[test]
fn windows_priority_classes_match_libuv_thresholds() {
    for (input, class, output) in [
        (-20, 0x100, -20),
        (-15, 0x100, -20),
        (-14, 0x80, -14),
        (-8, 0x80, -14),
        (-7, 0x8000, -7),
        (-1, 0x8000, -7),
        (0, 0x20, 0),
        (9, 0x20, 0),
        (10, 0x4000, 10),
        (18, 0x4000, 10),
        (19, 0x40, 19),
    ] {
        assert_eq!(windows_priority_class(input), class);
        assert_eq!(windows_nice_value(class).unwrap(), output);
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[test]
fn unix_getpriority_accepts_valid_minus_one() {
    assert_eq!(posix_priority_result(-1, 0).unwrap(), -1);
    assert_eq!(
        posix_priority_result(-1, libc::ESRCH)
            .unwrap_err()
            .raw_os_error(),
        Some(libc::ESRCH)
    );
    let current = native_get_priority(0).unwrap();
    assert!((-20..=19).contains(&current));
    let absent = native_get_priority(i32::MAX).unwrap_err();
    assert_eq!(absent.raw_os_error(), Some(libc::ESRCH));
}

/// Backs `process.report.getReport().header.glibcVersionRuntime` -- real
/// Node's own way of telling a glibc build apart from a musl one at
/// runtime (`null`/absent on musl and non-Linux targets), which some native-addon loaders check
/// directly instead of trusting `process.platform`/`arch` alone (real
/// trigger: `better-sqlite3`'s own prebuild-selection fallback,
/// `!process.report.getReport().header.glibcVersionRuntime`). Thaw's own
/// binary is itself statically one or the other (a build-time choice, the
/// same `cfg!(target_env = "musl")` distinction `thaw-registry`'s own
/// prebuild target-matching already uses) -- `libc::gnu_get_libc_version`
/// is the real glibc runtime version string on a glibc build; musl has no
/// such symbol at all.
fn glibc_version_runtime() -> Option<String> {
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        None
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let version = unsafe { std::ffi::CStr::from_ptr(libc::gnu_get_libc_version()) };
        Some(version.to_string_lossy().into_owned())
    }
}

/// Real per-interface addresses, grouped by interface name the way
/// `os.networkInterfaces()` shapes its result -- via POSIX
/// `getifaddrs` (already available through the existing `libc`
/// dependency, no new crate needed), the same "walk one C struct list"
/// approach `/proc`/`/sys` reads elsewhere in this file use in spirit.
/// The MAC address isn't in `getifaddrs`' own `AF_INET`/`AF_INET6`
/// entries (Linux only exposes it via a separate `AF_PACKET` entry for
/// the same interface name); reading `/sys/class/net/<name>/address`
/// instead is the same value without needing to also parse those
/// `AF_PACKET` entries, confirmed against real Node's own output for
/// every interface on a real multi-NIC machine, loopback (all zero)
/// included.
fn network_interfaces_json() -> rquickjs::Result<String> {
    use std::ffi::CStr;
    use std::net::{Ipv4Addr, Ipv6Addr};

    let mut groups: std::collections::BTreeMap<String, Vec<serde_json::Value>> =
        std::collections::BTreeMap::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        // Node throws a system error here; an empty result would read as "no interfaces".
        return Err(rquickjs::Error::new_from_js_message(
            "os",
            "network interfaces",
            format!("getifaddrs failed: {}", std::io::Error::last_os_error()),
        ));
    }
    let mut entry = head;
    while !entry.is_null() {
        let ifa = unsafe { &*entry };
        entry = ifa.ifa_next;
        if ifa.ifa_addr.is_null() {
            continue;
        }
        let family = unsafe { (*ifa.ifa_addr).sa_family } as i32;
        if family != libc::AF_INET && family != libc::AF_INET6 {
            continue;
        }
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let internal = ifa.ifa_flags & (libc::IFF_LOOPBACK as libc::c_uint) != 0;
        let mac = std::fs::read_to_string(format!("/sys/class/net/{name}/address"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "00:00:00:00:00:00".to_string());
        let entries = groups.entry(name).or_default();
        if family == libc::AF_INET {
            let address = unsafe { *(ifa.ifa_addr as *const libc::sockaddr_in) };
            let ip = Ipv4Addr::from(u32::from_be(address.sin_addr.s_addr));
            let mask_bits = if ifa.ifa_netmask.is_null() {
                0
            } else {
                let mask = unsafe { *(ifa.ifa_netmask as *const libc::sockaddr_in) };
                u32::from_be(mask.sin_addr.s_addr)
            };
            let netmask = Ipv4Addr::from(mask_bits);
            entries.push(serde_json::json!({
                "address": ip.to_string(), "netmask": netmask.to_string(), "family": "IPv4",
                "mac": mac, "internal": internal, "cidr": format!("{ip}/{}", mask_bits.count_ones()),
            }));
        } else {
            let address = unsafe { *(ifa.ifa_addr as *const libc::sockaddr_in6) };
            let ip = Ipv6Addr::from(address.sin6_addr.s6_addr);
            let mask_bytes = if ifa.ifa_netmask.is_null() {
                [0u8; 16]
            } else {
                let mask = unsafe { *(ifa.ifa_netmask as *const libc::sockaddr_in6) };
                mask.sin6_addr.s6_addr
            };
            let prefix: u32 = mask_bytes.iter().map(|byte| byte.count_ones()).sum();
            entries.push(serde_json::json!({
                "address": ip.to_string(), "netmask": Ipv6Addr::from(mask_bytes).to_string(), "family": "IPv6",
                "mac": mac, "internal": internal, "cidr": format!("{ip}/{prefix}"), "scopeid": address.sin6_scope_id,
            }));
        }
    }
    unsafe { libc::freeifaddrs(head) };
    Ok(serde_json::to_string(&groups).unwrap_or_else(|_| "{}".to_string()))
}

/// Real DNS resolution for `node:dns`'s `lookup`/`resolve`/`resolve4`/
/// `resolve6`, which otherwise only ever recognized an IP literal or
/// `"localhost"` and returned `ENOTFOUND` for every real hostname --
/// unlike `net.connect`/`http.request`, which already resolve real
/// hostnames correctly because the native `TcpStream::connect` call
/// they go through does its own OS-level resolution independently of
/// this module. `std::net::ToSocketAddrs` (stdlib, no new dependency)
/// already wraps the same OS resolver (`getaddrinfo`); a `port` of `0`
/// is irrelevant here since only the address half of the result is
/// used. Deliberately synchronous/blocking, like every other native
/// primitive this crate exposes to QuickJS (`TcpStream::connect`
/// itself already blocks the whole runtime during a real connection,
/// so this isn't a new class of tradeoff) -- real Node offloads DNS to
/// libuv's threadpool, which this runtime has no equivalent of.
fn dns_lookup_json(hostname: String) -> String {
    use std::net::ToSocketAddrs;

    match (hostname.as_str(), 0u16).to_socket_addrs() {
        Ok(addrs) => {
            let mut addresses: Vec<serde_json::Value> = addrs
                .map(|addr| {
                    let ip = addr.ip();
                    serde_json::json!({ "address": ip.to_string(), "family": if ip.is_ipv4() { 4 } else { 6 } })
                })
                .collect();
            addresses.dedup();
            serde_json::json!({ "ok": true, "addresses": addresses }).to_string()
        }
        Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
    }
}

include!("quickjs/processes.rs");

include!("quickjs/dns_client.rs");

include!("quickjs/intl.rs");

include!("quickjs/intl_time_zone_names.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_locale.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_datetime_skeleton.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_datetime.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_number.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_number_style.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_list.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_plurals.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_collator.rs");

#[cfg(feature = "icu4c")]
include!("quickjs/intl_icu4c.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_segmenter.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_relative_time.rs");

include!("quickjs/asymmetric_crypto.rs");

#[cfg(unix)]
static PENDING_PROCESS_SIGNALS: AtomicU64 = AtomicU64::new(0);

/// Signals a script can subscribe to with `process.on(name, ..)`.
#[cfg(unix)]
const PROCESS_SIGNALS: [(&str, libc::c_int); 6] = [
    ("SIGINT", libc::SIGINT),
    ("SIGTERM", libc::SIGTERM),
    ("SIGHUP", libc::SIGHUP),
    ("SIGQUIT", libc::SIGQUIT),
    ("SIGUSR1", libc::SIGUSR1),
    ("SIGUSR2", libc::SIGUSR2),
];

#[cfg(unix)]
extern "C" fn record_process_signal(signal: libc::c_int) {
    PENDING_PROCESS_SIGNALS.fetch_or(1 << signal, Ordering::Relaxed);
}

fn configure_process_signal(name: String, enabled: bool) -> bool {
    #[cfg(unix)]
    {
        let Some(&(_, signal)) = PROCESS_SIGNALS.iter().find(|(known, _)| *known == name) else {
            return false;
        };
        // SAFETY: the installed handler only performs an atomic or, which
        // is async-signal-safe. Disabling restores the platform default.
        unsafe {
            libc::signal(
                signal,
                if enabled {
                    record_process_signal as *const () as libc::sighandler_t
                } else {
                    libc::SIG_DFL
                },
            );
        }
        true
    }
    #[cfg(not(unix))]
    {
        let _ = (name, enabled);
        false
    }
}

/// Small libc-backed `process` queries for the JS shim (`getuid`, `umask`,
/// `kill`, resident set size). `kill` returns the errno (0 on success).
fn process_syscall(name: String, first: f64, second: f64) -> f64 {
    #[cfg(unix)]
    {
        // SAFETY: each call below is a plain libc query/signal with integer arguments.
        unsafe {
            match name.as_str() {
                "getpid" => f64::from(libc::getpid()),
                "getppid" => f64::from(libc::getppid()),
                "getuid" => libc::getuid() as f64,
                "geteuid" => libc::geteuid() as f64,
                "getgid" => libc::getgid() as f64,
                "getegid" => libc::getegid() as f64,
                "umask" => {
                    // `umask` can only be read by setting it, so put it straight back.
                    let previous = libc::umask(0o022);
                    libc::umask(if first.is_nan() {
                        previous
                    } else {
                        first as libc::mode_t
                    });
                    f64::from(previous)
                }
                "kill" => {
                    if libc::kill(first as libc::pid_t, second as libc::c_int) == 0 {
                        0.0
                    } else {
                        f64::from(
                            std::io::Error::last_os_error()
                                .raw_os_error()
                                .unwrap_or(libc::EPERM),
                        )
                    }
                }
                "rss" => std::fs::read_to_string("/proc/self/statm")
                    .ok()
                    .and_then(|text| text.split_whitespace().nth(1)?.parse::<f64>().ok())
                    .map_or(0.0, |pages| {
                        pages * libc::sysconf(libc::_SC_PAGESIZE) as f64
                    }),
                _ => f64::NAN,
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (name, first, second);
        f64::NAN
    }
}

fn dispatch_pending_process_signal(ctx: &Ctx<'_>) {
    #[cfg(unix)]
    {
        let pending = PENDING_PROCESS_SIGNALS.swap(0, Ordering::Relaxed);
        if pending == 0 {
            return;
        }
        for (name, signal) in PROCESS_SIGNALS {
            if pending & (1 << signal) == 0 {
                continue;
            }
            if let Ok(process) = ctx.globals().get::<_, Object>("process") {
                if let Ok(emit) = process.get::<_, Function>("emit") {
                    let _ = emit.call::<_, bool>((name,));
                }
            }
        }
    }
    #[cfg(not(unix))]
    let _ = ctx;
}

include!("quickjs/context.rs");
include!("quickjs/vm_context.rs");

include!("quickjs/platform_globals.rs");

include!("quickjs/api.rs");

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests;

/// Common C ABI result for external operations that can either return a
/// pointer-shaped value or throw. Exactly one pointer is non-null. Read an
/// owned result string through `thaw_arena::NativeStr` to retain embedded
/// NUL bytes, then release it with `thaw_arena::destroy_string` (or the ABI
/// companion `thaw_cstring_destroy`).
#[repr(C)]
pub struct ThawResult {
    pub value: *const c_char,
    pub error: *const c_char,
}

/// Handle-valued companion to `ThawResult`; its non-null error pointer has
/// the same `NativeStr` reader and `destroy_string` ownership contract.
#[repr(C)]
pub struct ThawHandleResult {
    pub value: u64,
    pub error: *const c_char,
}
