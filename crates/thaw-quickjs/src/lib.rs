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
use std::os::raw::{c_char, c_void};
#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::rc::Rc;
#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use base64::Engine as _;
use cbc::cipher::{
    block_padding::{NoPadding, Pkcs7},
    BlockDecryptMut, BlockEncryptMut, KeyIvInit,
};
use rquickjs::function::Args;
#[cfg(feature = "wasm")]
use rquickjs::Persistent;
use rquickjs::{Array, ArrayBuffer, Context, Ctx, Function, Object, Runtime, Value};
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
use sha2::{Digest, Sha256, Sha512};
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
);
static NAPI_BRIDGE: Mutex<Option<NapiBridge>> = Mutex::new(None);

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
) {
    *NAPI_BRIDGE.lock().unwrap() = Some((exports, call, handle, poll, pending));
}

fn install_napi_bridge(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let Some((exports, call, handle, _, _)) = *NAPI_BRIDGE.lock().unwrap() else {
        return Ok(());
    };
    let exports = Function::new(ctx.clone(), move || unsafe { to_str(exports()) })?;
    let call = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, name: String, args: String| unsafe {
            let _active = ActiveNapiContext::enter(&ctx);
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
}

#[cfg(feature = "tls")]
#[derive(Default)]
struct TlsCertificates {
    peer: Option<Vec<u8>>,
    local: Option<Vec<u8>>,
}

#[cfg(feature = "tls")]
#[derive(Debug)]
struct InsecureServerVerifier;

#[cfg(feature = "tls")]
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

#[cfg(feature = "wasm")]
include!("quickjs/wasm.rs");

fn run_quickjs_gc(ctx: Ctx<'_>) {
    ctx.run_gc();
}

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

fn digest_bytes(algorithm: &str, value: &[u8]) -> Vec<u8> {
    match algorithm {
        "sha1" => Sha1::digest(value).to_vec(),
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
fn network_interfaces_json() -> String {
    use std::ffi::CStr;
    use std::net::{Ipv4Addr, Ipv6Addr};

    let mut groups: std::collections::BTreeMap<String, Vec<serde_json::Value>> =
        std::collections::BTreeMap::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return "{}".to_string();
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
    serde_json::to_string(&groups).unwrap_or_else(|_| "{}".to_string())
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

include!("quickjs/intl.rs");

include!("quickjs/intl_time_zone_names.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_locale.rs");

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

#[cfg(feature = "intl")]
include!("quickjs/intl_segmenter.rs");

#[cfg(feature = "intl")]
include!("quickjs/intl_relative_time.rs");

include!("quickjs/asymmetric_crypto.rs");

#[cfg(unix)]
static PENDING_PROCESS_SIGNAL: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn record_process_signal(signal: libc::c_int) {
    PENDING_PROCESS_SIGNAL.store(signal, Ordering::Relaxed);
}

fn configure_process_signal(name: String, enabled: bool) -> bool {
    #[cfg(unix)]
    {
        let signal = match name.as_str() {
            "SIGINT" => libc::SIGINT,
            "SIGTERM" => libc::SIGTERM,
            _ => return false,
        };
        // SAFETY: the installed handler only performs an atomic store, which
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

fn dispatch_pending_process_signal(ctx: &Ctx<'_>) {
    #[cfg(unix)]
    {
        let signal = PENDING_PROCESS_SIGNAL.swap(0, Ordering::Relaxed);
        let name = match signal {
            libc::SIGINT => "SIGINT",
            libc::SIGTERM => "SIGTERM",
            _ => return,
        };
        if let Ok(process) = ctx.globals().get::<_, Object>("process") {
            if let Ok(emit) = process.get::<_, Function>("emit") {
                let _ = emit.call::<_, bool>((name,));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = ctx;
}

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
