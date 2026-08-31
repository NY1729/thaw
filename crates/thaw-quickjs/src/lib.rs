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
//! representation. A Promise-returning call is driven to completion by
//! polling QuickJS's job queue (`Promise::finish`) rather than true
//! non-blocking integration -- the same "poll until resolved" shortcut V1
//! async/await already takes (docs/design/async-await.md), consistent
//! rather than a special case.
//!
//! Generated code uses `thaw_js_call_result` to route unknown functions,
//! thrown JS exceptions, malformed arguments, and Promise rejections through
//! Thaw's `try`/`catch`. The original `thaw_js_call` JSON-error-object API is
//! retained for C ABI compatibility with older callers.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{Shutdown, TcpListener, TcpStream, UdpSocket};
use std::os::raw::c_char;
#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use base64::Engine as _;
use rquickjs::function::Args;
use rquickjs::{Array, ArrayBuffer, Context, Ctx, Function, Object, Persistent, Runtime, Value};
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime,
};
use rustls::server::WebPkiClientVerifier;
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, Error as RustlsError, RootCertStore,
    ServerConfig, ServerConnection, SignatureScheme, StreamOwned,
};
use sha2::{Digest, Sha256, Sha512};
use wasmi::Linker as WasmLinker;
use wasmi::{Caller as WasmCaller, Engine as WasmEngine, Extern as WasmExtern};
use wasmi::{Memory as WasmMemory, MemoryType as WasmMemoryType, Module as WasmModule};
use wasmi::{Store as WasmStore, Val as WasmVal, ValType as WasmValType};
use wasmi_wasi::sync::{ambient_authority, Dir as WasiDir, WasiCtxBuilder};
use wasmi_wasi::WasiCtx;

type NapiBridgeCallback = unsafe extern "C" fn(*const c_char, *const c_char) -> *const c_char;
type NapiBridgeExports = unsafe extern "C" fn() -> *const c_char;
type NapiBridgeHandle = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
) -> *const c_char;
static NAPI_BRIDGE: Mutex<Option<(NapiBridgeExports, NapiBridgeCallback, NapiBridgeHandle)>> =
    Mutex::new(None);

pub fn register_napi_bridge(
    exports: NapiBridgeExports,
    call: NapiBridgeCallback,
    handle: NapiBridgeHandle,
) {
    *NAPI_BRIDGE.lock().unwrap() = Some((exports, call, handle));
}

fn install_napi_bridge(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let Some((exports, call, handle)) = *NAPI_BRIDGE.lock().unwrap() else {
        return Ok(());
    };
    let exports = Function::new(ctx.clone(), move || unsafe { to_str(exports()) })?;
    let call = Function::new(ctx.clone(), move |name: String, args: String| unsafe {
        let name = CString::new(name).unwrap_or_default();
        let args = CString::new(args).unwrap_or_default();
        to_str(call(name.as_ptr(), args.as_ptr()))
    })?;
    ctx.globals().set("__thaw_napi_bridge_exports", exports)?;
    ctx.globals().set("__thaw_napi_bridge_call", call)?;
    let handle = Function::new(
        ctx.clone(),
        move |operation: String, target: String, name: String, args: String| unsafe {
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
type TlsStream = StreamOwned<ClientConnection, TcpStream>;
type TlsStreamTable = (u32, HashMap<u32, TlsStream>);
type TlsServerStream = StreamOwned<ServerConnection, TcpStream>;
type TlsServerStreamTable = (u32, HashMap<u32, TlsServerStream>);

struct TlsListener {
    socket: TcpListener,
    config: Arc<ServerConfig>,
    local_certificate: Vec<u8>,
}

#[derive(Default)]
struct TlsCertificates {
    peer: Option<Vec<u8>>,
    local: Option<Vec<u8>>,
}

#[derive(Debug)]
struct InsecureServerVerifier;

impl rustls::client::danger::ServerCertVerifier for InsecureServerVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, RustlsError> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, RustlsError> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, RustlsError> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

include!("quickjs/compression.rs");

include!("quickjs/filesystem.rs");

include!("quickjs/wasm.rs");

enum HostChildCommand {
    Stdin(Vec<u8>),
    StdinEnd,
    Kill(i32),
}

enum HostChildEvent {
    Spawn(u32),
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Error {
        message: String,
        code: String,
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
}

struct HostChildTable {
    next_handle: u32,
    children: HashMap<u32, HostChild>,
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
    Message(String),
    Stdout(String),
    Stderr(String),
    DirectRequest {
        target: u32,
        source: u32,
        request: u64,
        payload: String,
    },
    ParentDirectResult {
        request: u64,
        error: Option<String>,
    },
    PortMessage {
        port: String,
        payload: String,
    },
    Error(String),
    Exit(i32),
}

struct HostWorker {
    commands: Sender<HostWorkerCommand>,
    events: Receiver<HostWorkerEvent>,
    thread: Option<JoinHandle<()>>,
}

struct HostWorkerStart {
    bundle_source: String,
    source: String,
    worker_data_json: String,
    config_json: String,
    thread_id: u32,
    shared_env: Arc<Mutex<HashMap<String, String>>>,
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
        .chunks_exact(2)
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

fn digest_bytes(algorithm: &str, value: &[u8]) -> Vec<u8> {
    match algorithm {
        "sha256" => Sha256::digest(value).to_vec(),
        "sha512" => Sha512::digest(value).to_vec(),
        _ => Vec::new(),
    }
}

fn hmac_bytes(algorithm: &str, key: &[u8], value: &[u8]) -> Vec<u8> {
    let block_size = if algorithm == "sha512" { 128 } else { 64 };
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

fn os_info_json() -> String {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        value => value,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        value => value,
    };
    let mut hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "localhost".to_string())
        .trim()
        .to_string();
    if hostname.is_empty() {
        hostname = "localhost".to_string();
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let tmp = std::env::var("TMPDIR")
        .or_else(|_| std::env::var("TMP"))
        .unwrap_or_else(|_| "/tmp".to_string());
    let cpu_info = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let model = cpu_info
        .lines()
        .find_map(|line| line.strip_prefix("model name\t: "))
        .unwrap_or(arch)
        .to_string();
    let speed = cpu_info
        .lines()
        .find_map(|line| line.strip_prefix("cpu MHz\t\t: "))
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0)
        .round() as u64;
    let cpu_count = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let cpus = (0..cpu_count)
        .map(|_| serde_json::json!({ "model": model, "speed": speed, "times": { "user": 0, "nice": 0, "sys": 0, "idle": 0, "irq": 0 } }))
        .collect::<Vec<_>>();
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
    serde_json::json!({
        "platform": platform, "arch": arch, "type": if platform == "linux" { "Linux" } else { platform },
        "hostname": hostname, "homedir": home, "tmpdir": tmp, "release": std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim(),
        "version": std::fs::read_to_string("/proc/sys/kernel/version").unwrap_or_default().trim(), "machine": std::env::consts::ARCH,
        "endianness": if cfg!(target_endian = "little") { "LE" } else { "BE" }, "cpus": cpus,
        "totalmem": memory_value("MemTotal:"), "freemem": memory_value("MemAvailable:"), "uptime": uptime, "loadavg": loadavg,
        "userInfo": { "username": std::env::var("USER").unwrap_or_default(), "homedir": home, "shell": std::env::var("SHELL").unwrap_or_default(), "uid": 0, "gid": 0 }
    }).to_string()
}

include!("quickjs/processes.rs");

include!("quickjs/context.rs");

include!("quickjs/platform_globals.rs");

include!("quickjs/api.rs");

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests;

/// Common C ABI result for external operations that can either return a
/// pointer-shaped value or throw. Both pointers are owned by the callee for
/// the current request; exactly one is non-null.
#[repr(C)]
pub struct ThawResult {
    pub value: *const c_char,
    pub error: *const c_char,
}

#[repr(C)]
pub struct ThawHandleResult {
    pub value: u64,
    pub error: *const c_char,
}
