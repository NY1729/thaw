//! `node:worker_threads` workers (W2a/W2b): one OS thread and one `Interp` per worker. Messages cross threads as the
//! wire strings of worker_wire.js, so this module only moves `String`s. The parent side is the `workers` table of the
//! spawning `Interp` (`WorkerState`); the worker side is its `side`. The event loop drains both via [`poll`].
//! The state lives on the `Interp`, not in a thread-local: async functions and generators run on coroutine threads
//! that call these natives with the same `Interp`, so the state must follow the `Interp`, not the OS thread.
//! `terminate()` sets an interrupt flag that the worker's `Interp` checks (`Interp::check_interrupt`), so busy loops stop.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::builtins::json_quote;
use crate::heap_limit;
use crate::process_env::{private_env_table, share_env_table, EnvTable};
use crate::typed::SabBlock;
use crate::*;

/// Stack reserved for a worker's JS (virtual memory, committed on use) when `stackSizeMb` is not given.
const STACK: usize = 64 << 20;
/// Wire of an error value that could not itself be encoded.
const OPAQUE_ERROR: &str = r#"{"root":{"v":"uncaught exception"},"nodes":[]}"#;
/// The worker-side bootstrap (worker_bootstrap.js), run in the worker's own realm before its source.
const BOOTSTRAP: &str = include_str!("web/worker_bootstrap.js");
/// Wire of the error a worker over its heap limit reports (Node: ERR_WORKER_OUT_OF_MEMORY, exit code 1).
const OOM_WIRE: &str = r#"{"root":{"r":0},"nodes":[{"t":"Error","n":"Error","m":"Worker terminated due to reaching memory limit: JS heap out of memory","k":"ERR_WORKER_OUT_OF_MEMORY"}]}"#;

/// Parent to worker commands. `Terminate` also sets the interrupt flag, which is what stops a busy worker.
enum Cmd {
    Message(String),
    /// Stdin bytes as hex, and whether this is the end of the stream (`worker.stdin.end()`).
    Stdin(String, bool),
    Terminate,
    /// `postMessageToThread` from thread `source`, and the sender's ack id (answered with `ThreadAck`).
    Thread(u32, u32, String),
    /// The answer to a `postMessageToThread` this worker sent: ack id, status, error wire.
    ThreadAck(u32, String, String),
    /// A message to the port end at this address, and its wire (routed by the main thread, see `PORT_ROUTES`).
    Port(u64, String),
    /// The peer of the port end at this address closed.
    PortClose(u64),
}

enum Evt {
    Online,
    Message(String),
    Error(String),
    Exit(i32),
    /// Captured stdout (fd 1) or stderr (fd 2) bytes as hex (only when the parent asked for them).
    Stdio(u8, String),
    /// `postMessageToThread` from the worker to the parent: the worker's ack id, and the wire.
    Thread(u32, String),
    /// The parent's answer to a `postMessageToThread` the worker sent: ack id, status, error wire.
    ThreadAck(u32, String, String),
    /// A message to a port end this worker owns (sent to the main thread, which routes it).
    Port(u64, String),
    /// The peer of a port end this worker owns closed.
    PortClose(u64),
    /// The answer to a `cpuUsage` request: its ack id, then the worker thread's user and system CPU microseconds.
    Cpu(u32, u64, u64),
    /// The answer to a `getHeapStatistics` request: its ack id, then the worker thread's `__thaw_memory_usage` JSON.
    Heap(u32, String),
}

/// `worker.cpuUsage()` and `worker.getHeapStatistics()` requests queued by the parent for the worker thread: (ack,
/// is a heap request). The parent queues an ack and raises `requested`; the worker takes the queue in `serve_cpu`,
/// from its interrupt check.
#[derive(Default)]
pub(crate) struct CpuProbe {
    requested: AtomicBool,
    acks: Mutex<Vec<(u32, bool)>>,
}

/// A worker that can receive a `postMessageToThread`: its command channel, and its port token (a port moved to it is
/// routed by that token).
struct ThreadEntry {
    tx: Sender<Cmd>,
    token: u64,
}

/// The process-wide thread table of `postMessageToThread`, keyed by (hub token, thread id). The hub is the main thread's
/// port token (a worker's spawner), so the ids of two Interps in one process never meet. The main thread (0) has no
/// entry: a worker reaches it through its parent channel. Removal and the queue drain happen under this lock (run_worker),
/// so a send either queues before the drain or finds no thread.
static THREADS: Mutex<BTreeMap<(u64, u32), ThreadEntry>> = Mutex::new(BTreeMap::new());

struct Worker {
    tx: Sender<Cmd>,
    rx: Receiver<Evt>,
    interrupt: Arc<AtomicBool>,
    refd: bool,
    exited: bool,
    /// The worker's port token (its port routes name it as their owner).
    token: u64,
    join: Option<JoinHandle<()>>,
    cpu: Arc<CpuProbe>,
}

struct Side {
    rx: Receiver<Cmd>,
    tx: Sender<Evt>,
    /// This worker's port token, and the port token of the main thread that spawned it (the hub).
    token: u64,
    hub: u64,
    /// This worker's thread id (its `threadId`, the key of its THREADS entry).
    thread_id: u32,
    /// The thread-id counter of the tree (shared with every worker it spawns, see `thread_ids`).
    ids: Arc<AtomicU32>,
    interrupt: Arc<AtomicBool>,
    /// The worker's parentPort has a referenced `message` listener (set by the bootstrap).
    alive: Arc<AtomicBool>,
    cpu: Arc<CpuProbe>,
    /// The entry is an ES module (`__thaw_run_main_esm`): it runs without the CommonJS globals.
    esm: bool,
}

/// Per-`Interp` worker state: the workers this Interp spawned (`workers`, keyed by handle), and on a worker thread
/// the channel to its parent (`side`).
#[derive(Default)]
pub(crate) struct WorkerState {
    next: u32,
    workers: BTreeMap<u32, Worker>,
    side: Option<Side>,
    /// The environment table this realm's `process.env` reads and writes (`process_env.rs`); None is the real
    /// process environment (the main realm until a `SHARE_ENV` worker is spawned).
    pub(crate) env_table: Option<EnvTable>,
    /// A referenced port this realm listens on keeps the loop alive (set by messaging.js).
    pub(crate) ports_live: bool,
    /// This Interp's port token as the main thread (allocated on first use; a worker realm uses its `Side`).
    token: u64,
    /// `port` events for this realm's own ports, routed by a script (main thread), delivered by the next poll.
    local: Vec<String>,
    /// The registry bundles this realm loaded, in load order: a worker whose file is one of their modules runs it.
    bundles: Vec<LoadedBundle>,
    /// The tree's thread-id counter, used when this Interp is the main thread (a worker uses its `Side`'s).
    ids: Arc<AtomicU32>,
}

/// A registry bundle loaded into this realm: its package name, its text (one copy per loaded package, made once at
/// load and shared by every worker spawned from it), and its factory keys.
struct LoadedBundle {
    package: String,
    text: Arc<str>,
    keys: BTreeSet<String>,
}

/// The bundled module a worker starts as its main module (see [`main_bundle`]).
pub(crate) struct MainBundle {
    package: String,
    text: Arc<str>,
    key: String,
}

/// Records a bundle loaded into `it` (`Package::load_named`), so a worker spawned from `it` can run its modules.
pub(crate) fn register_bundle(
    it: &mut Interp,
    package: &str,
    text: &str,
    parts: &crate::scan::BundleParts,
) {
    let keys = parts.factories.iter().map(|(key, _)| key.clone()).collect();
    it.worker.bundles.push(LoadedBundle {
        package: package.to_string(),
        text: Arc::from(text),
        keys,
    });
}

/// The bundled module a worker's file names, `/thaw_modules/<key>` (the virtual `__filename` of a bundled module):
/// the first loaded bundle with that factory key, as `require` resolves a key. None keeps the disk loader.
fn main_bundle(it: &Interp, file: &str) -> Option<MainBundle> {
    let key = file.strip_prefix("/thaw_modules/")?;
    let bundle = it.worker.bundles.iter().find(|b| b.keys.contains(key))?;
    Some(MainBundle {
        package: bundle.package.clone(),
        text: bundle.text.clone(),
        key: key.to_string(),
    })
}

/// SharedArrayBuffer blocks in flight between Interps: the wire carries only the id. An encode enters the block
/// under a new id, the one decode that reads that wire takes it out (`sab_import`), and `release_wire` takes out
/// the ones a wire nobody decodes would have carried.
static SAB_IN_FLIGHT: Mutex<BTreeMap<u64, Arc<SabBlock>>> = Mutex::new(BTreeMap::new());
static SAB_NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn sab_table() -> MutexGuard<'static, BTreeMap<u64, Arc<SabBlock>>> {
    SAB_IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner())
}

/// Enters `block` for one wire; returns the id that wire carries.
pub(crate) fn sab_export(block: Arc<SabBlock>) -> u64 {
    let id = SAB_NEXT_ID.fetch_add(1, Ordering::Relaxed);
    sab_table().insert(id, block);
    id
}

/// Takes the block of a wire node (once: the decode that reads the node owns the reference now).
pub(crate) fn sab_import(id: u64) -> Option<Arc<SabBlock>> {
    sab_table().remove(&id)
}

/// A moved ArrayBuffer's bytes and its maxByteLength (`Some` for a resizable buffer).
type MovedBytes = (Vec<u8>, Option<usize>);

/// ArrayBuffers in flight between Interps (postMessage with a transfer list): the wire carries only the id, and the
/// bytes move, they are not copied. `ab_export` takes the Vec out of the sender's buffer; the decode that reads the
/// wire (`ab_import`) gives it to the receiver's new buffer.
static AB_IN_FLIGHT: Mutex<BTreeMap<u64, MovedBytes>> = Mutex::new(BTreeMap::new());
static AB_NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn ab_table() -> MutexGuard<'static, BTreeMap<u64, MovedBytes>> {
    AB_IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner())
}

/// Enters moved bytes for one wire; returns the id that wire carries.
pub(crate) fn ab_export(bytes: Vec<u8>, max: Option<usize>) -> u64 {
    let id = AB_NEXT_ID.fetch_add(1, Ordering::Relaxed);
    ab_table().insert(id, (bytes, max));
    id
}

/// Takes the bytes of a wire node (once: the decode that reads the node owns them now).
pub(crate) fn ab_import(id: u64) -> Option<MovedBytes> {
    ab_table().remove(&id)
}

/// The ids of the `"t":"<kind>","id":N` nodes in a wire, both as the wire itself writes them and escaped (a wire
/// nested in a JSON string, like the workerData in a worker's spawn options).
fn node_ids(wire: &str, kind: &str) -> Vec<u64> {
    let plain = format!(r#""t":"{kind}","id":"#);
    let escaped = plain.replace('"', r#"\""#);
    let mut ids = Vec::new();
    for pattern in [plain, escaped] {
        let mut rest = wire;
        while let Some(at) = rest.find(&pattern) {
            rest = &rest[at + pattern.len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(id) = digits.parse::<u64>() {
                ids.push(id);
            }
        }
    }
    ids
}

/// A port end's route: the realm that owns it (its port token; a worker's token is fixed at spawn, the main realm's
/// is allocated once per Interp) and the address of its peer end.
#[derive(Clone, Copy)]
struct Route {
    owner: u64,
    peer: u64,
}

/// Cross-thread MessagePort routing (worker_threads PR3). Every port end that crosses threads has an address; a
/// message to an address goes to the realm that owns it. The main thread is the hub: a worker's send goes to it
/// (`Evt::Port`), and it forwards to the owner. Tokens and addresses come from one process-wide counter, so routes of
/// two Interps in one process never meet. `PORT_ROUTES` holds the owner and peer of each address that is live or in
/// flight. `PORT_FLIGHT` holds the ends whose wire is encoded but not yet decoded by its reader: a released wire (its
/// reader ended, or nothing took it) hangs up their peers through `PORT_HANGUPS` (owner, peer), which the owner's
/// poll delivers.
static PORT_ROUTES: Mutex<BTreeMap<u64, Route>> = Mutex::new(BTreeMap::new());
static PORT_FLIGHT: Mutex<BTreeSet<u64>> = Mutex::new(BTreeSet::new());
static PORT_HANGUPS: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());
static PORT_NEXT: AtomicU64 = AtomicU64::new(1);

fn locked<T>(table: &'static Mutex<T>) -> MutexGuard<'static, T> {
    table.lock().unwrap_or_else(|e| e.into_inner())
}

/// The `(address, peer address)` of each `Port` node in a wire (plain, or nested in a JSON string).
fn port_nodes(wire: &str) -> Vec<(u64, u64)> {
    fn digits(s: &str) -> (&str, &str) {
        s.split_at(s.bytes().take_while(u8::is_ascii_digit).count())
    }
    let mut found = Vec::new();
    for (open, mid) in [
        (r#""t":"Port","p":"#, r#","q":"#),
        (r#"\"t\":\"Port\",\"p\":"#, r#",\"q\":"#),
    ] {
        let mut rest = wire;
        while let Some(at) = rest.find(open) {
            rest = &rest[at + open.len()..];
            let (p, after_p) = digits(rest);
            rest = after_p;
            if let Some(tail) = after_p.strip_prefix(mid) {
                let (q, after_q) = digits(tail);
                if let (Ok(p), Ok(q)) = (p.parse(), q.parse()) {
                    found.push((p, q));
                }
                rest = after_q;
            }
        }
    }
    found
}

/// A fresh address for a port end.
pub(crate) fn port_address() -> u64 {
    PORT_NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Releases the blocks a wire carries when nobody will decode it (a worker that ended first, or a send that could not
/// be delivered). Every such node was written by the encoder, and a table entry is dropped once it is taken.
pub(crate) fn release_wire(wire: &str) {
    for id in node_ids(wire, "SAB") {
        sab_table().remove(&id);
    }
    for id in node_ids(wire, "ABT") {
        ab_table().remove(&id);
    }
    // a port end still in flight is freed: its route goes, and its peer hangs up (once; a decoded end is landed)
    for (addr, peer) in port_nodes(wire) {
        if locked(&PORT_FLIGHT).remove(&addr) {
            locked(&PORT_ROUTES).remove(&addr);
            let owner = locked(&PORT_ROUTES).get(&peer).map(|r| r.owner);
            if let Some(owner) = owner {
                locked(&PORT_HANGUPS).push((owner, peer));
            }
        }
    }
}

/// Releases what a dropped command carries (its wire never reaches a reader).
fn release_cmd(cmd: &Cmd) {
    if let Cmd::Message(v) | Cmd::Thread(_, _, v) | Cmd::Port(_, v) = cmd {
        release_wire(v);
    }
}

/// Releases what a dropped event carries (its wire never reaches a reader).
fn release_evt(evt: &Evt) {
    if let Evt::Message(v) | Evt::Error(v) | Evt::Thread(_, v) | Evt::Port(_, v) = evt {
        release_wire(v);
    }
}

/// The owner-side item a `port` (wire) or `portClose` (no wire) event becomes for this realm's dispatch.
fn port_item(addr: u64, wire: Option<&str>) -> String {
    match wire {
        Some(v) => format!(r#"{{"type":"port","addr":{addr},"value":{}}}"#, quoted(v)),
        None => format!(r#"{{"type":"portClose","addr":{addr}}}"#),
    }
}

/// Routes a port event (`wire`, or None for a hang-up) to the thread that owns `addr`. Main thread only: the item for
/// this realm is returned (owner 0), a worker gets the command; with no route the wire is released.
fn route_port(it: &Interp, addr: u64, wire: Option<String>) -> Option<String> {
    let owner = locked(&PORT_ROUTES).get(&addr).map(|r| r.owner);
    let Some(owner) = owner else {
        if let Some(v) = &wire {
            release_wire(v);
        }
        return None;
    };
    if owner == it.worker.token {
        return Some(port_item(addr, wire.as_deref()));
    }
    match it.worker.workers.values().find(|w| w.token == owner) {
        Some(w) => send_cmd(
            w,
            match wire {
                Some(v) => Cmd::Port(addr, v),
                None => Cmd::PortClose(addr),
            },
        ),
        None => {
            if let Some(v) = &wire {
                release_wire(v);
            }
        }
    }
    None
}

/// The port token of this realm: a worker's (from its `Side`), or the Interp's own as the main thread (allocated on
/// first use).
fn realm_token(it: &mut Interp) -> u64 {
    if let Some(side) = &it.worker.side {
        return side.token;
    }
    if it.worker.token == 0 {
        it.worker.token = port_address();
    }
    it.worker.token
}

/// Sends a command to a worker; a worker whose thread has ended releases what the command carries.
fn send_cmd(w: &Worker, cmd: Cmd) {
    if let Err(mpsc::SendError(lost)) = w.tx.send(cmd) {
        release_cmd(&lost);
    }
}

/// This Interp's hub: the main thread's port token (the main thread itself, or the one that spawned this worker).
fn hub_of(it: &mut Interp) -> u64 {
    match it.worker.side.as_ref().map(|side| side.hub) {
        Some(hub) => hub,
        None => realm_token(it),
    }
}

/// The `postMessageToThread` source id of this Interp: a worker's thread id, or 0 for the main thread.
fn thread_id_of(it: &Interp) -> u32 {
    it.worker.side.as_ref().map_or(0, |side| side.thread_id)
}

/// The thread-id counter of this Interp's tree: a worker shares its spawner's, so ids are unique across nesting levels.
fn thread_ids(it: &Interp) -> Arc<AtomicU32> {
    match &it.worker.side {
        Some(side) => side.ids.clone(),
        None => it.worker.ids.clone(),
    }
}

/// Sends a `postMessageToThread` to thread `target`, from this Interp. A worker's send to 0 goes to the main thread
/// through its parent channel. False when the thread is not there (it never existed, or has left): the wire is released
/// and the sender settles FAILED.
fn thread_send(it: &mut Interp, target: u32, ack: u32, wire: String) -> bool {
    if target == 0 && is_worker(it) {
        to_parent(it, Evt::Thread(ack, wire));
        return true;
    }
    let key = (hub_of(it), target);
    let source = thread_id_of(it);
    let sent = match locked(&THREADS).get(&key) {
        Some(entry) => entry
            .tx
            .send(Cmd::Thread(source, ack, wire))
            .map_err(|mpsc::SendError(cmd)| cmd),
        None => Err(Cmd::Thread(source, ack, wire)),
    };
    match sent {
        Ok(()) => true,
        Err(cmd) => {
            release_cmd(&cmd);
            false
        }
    }
}

/// Answers a `postMessageToThread` that came from thread `source`: the main thread (0) through the parent channel, or the
/// sending worker by its entry. A sender that has left is not answered.
fn thread_reply(it: &mut Interp, source: u32, ack: u32, status: String, wire: String) {
    if source == 0 {
        to_parent(it, Evt::ThreadAck(ack, status, wire));
        return;
    }
    let key = (hub_of(it), source);
    if let Some(entry) = locked(&THREADS).get(&key) {
        let _ = entry.tx.send(Cmd::ThreadAck(ack, status, wire));
    }
}

fn quoted(s: &str) -> String {
    let mut out = String::new();
    json_quote(s, &mut out);
    out
}

/// Heap limit in bytes for a worker's `maxOldGenerationSizeMb` (NaN, not positive or -1: no limit).
fn heap_bytes(heap_mb: f64) -> Option<usize> {
    (heap_mb.is_finite() && heap_mb > 0.0).then(|| (heap_mb * f64::from(1u32 << 20)) as usize)
}

/// Thread stack for a worker's `stackSizeMb` (NaN or not positive: unset, [`STACK`]). Best effort: 16 MiB per Node
/// MiB, since an interpreter frame is much larger than a V8 frame; clamped to [8 MiB, 512 MiB].
pub(crate) fn stack_bytes(stack_mb: f64) -> usize {
    if stack_mb.is_finite() && stack_mb > 0.0 {
        ((stack_mb * f64::from(16u32 << 20)) as usize).clamp(8 << 20, 512 << 20)
    } else {
        STACK
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    it: &mut Interp,
    source: String,
    opts: String,
    stack_mb: f64,
    heap_mb: f64,
    share_env: bool,
    // (the worker's port token, the spawning realm's port token: the hub its ports route through)
    tokens: (u64, u64),
    // the worker's thread id (its `threadId`)
    thread_id: u32,
    // the bundled module the worker runs as its main module instead of the disk loader's `source`
    main: Option<MainBundle>,
    // the entry is an ES module
    esm: bool,
) -> u32 {
    let (tx, cmd_rx) = mpsc::channel();
    let (evt_tx, rx) = mpsc::channel();
    let ids = thread_ids(it);
    let interrupt = Arc::new(AtomicBool::new(false));
    let cpu = Arc::new(CpuProbe::default());
    let stack = stack_bytes(stack_mb);
    let heap = heap_bytes(heap_mb);
    // a SHARE_ENV worker reads and writes the table of this realm; any other worker gets a table of its own
    let env = if share_env {
        share_env_table(it)
    } else {
        private_env_table()
    };
    // registered before the thread starts, so a send from now on is queued for it (or fails once it has left)
    locked(&THREADS).insert(
        (tokens.1, thread_id),
        ThreadEntry {
            tx: tx.clone(),
            token: tokens.0,
        },
    );
    let side = Side {
        rx: cmd_rx,
        tx: evt_tx,
        token: tokens.0,
        hub: tokens.1,
        thread_id,
        ids,
        interrupt: interrupt.clone(),
        alive: Arc::new(AtomicBool::new(false)),
        cpu: cpu.clone(),
        esm,
    };
    let join = std::thread::Builder::new()
        .name("thaw-js-worker".into())
        .stack_size(stack)
        .spawn(move || run_worker(&source, &opts, stack, heap, side, env, main))
        .expect("worker thread spawns");
    it.worker.next += 1;
    let id = it.worker.next;
    it.worker.workers.insert(
        id,
        Worker {
            tx,
            rx,
            interrupt,
            refd: true,
            exited: false,
            token: tokens.0,
            join: Some(join),
            cpu,
        },
    );
    id
}

impl CpuProbe {
    /// Parent side: queues `ack` (a heap request when `heap`) and wakes the worker's next interrupt check.
    fn request(&self, ack: u32, heap: bool) {
        let mut acks = self.acks.lock().unwrap_or_else(|e| e.into_inner());
        acks.push((ack, heap));
        self.requested.store(true, Ordering::Relaxed);
    }
}

/// This thread's CPU time as (user, system) microseconds: getrusage(RUSAGE_THREAD) on Linux. Elsewhere the thread's
/// total from CLOCK_THREAD_CPUTIME_ID, all of it counted as user time.
fn thread_cpu_micros() -> (u64, u64) {
    #[cfg(target_os = "linux")]
    {
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut ru) };
        let micros = |t: libc::timeval| t.tv_sec as u64 * 1_000_000 + t.tv_usec as u64;
        (micros(ru.ru_utime), micros(ru.ru_stime))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
        (ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000, 0)
    }
}

/// Worker side, from `Interp::check_interrupt`: answers the queued `cpuUsage` requests with this thread's CPU time.
/// Costs one relaxed load when nothing is queued (and nothing at all on a non-worker Interp).
#[inline]
pub(crate) fn serve_cpu_if_requested(it: &Interp) {
    if let Some(side) = &it.worker.side {
        if side.cpu.requested.load(Ordering::Relaxed) {
            serve_cpu(side, it);
        }
    }
}

fn serve_cpu(side: &Side, it: &Interp) {
    let acks = {
        let mut acks = side.cpu.acks.lock().unwrap_or_else(|e| e.into_inner());
        side.cpu.requested.store(false, Ordering::Relaxed);
        std::mem::take(&mut *acks)
    };
    if acks.is_empty() {
        return;
    }
    let (user, system) = thread_cpu_micros();
    // the memory walk is only paid for when a heap request is queued
    let usage = acks
        .iter()
        .any(|&(_, heap)| heap)
        .then(|| crate::sysio::memory_usage_json(it));
    for (ack, heap) in acks {
        let evt = match &usage {
            Some(json) if heap => Evt::Heap(ack, json.clone()),
            _ => Evt::Cpu(ack, user, system),
        };
        let _ = side.tx.send(evt);
    }
}

fn to_parent(it: &Interp, e: Evt) {
    if let Some(side) = &it.worker.side {
        // the parent is gone: nobody will read what this event carries
        if let Err(mpsc::SendError(lost)) = side.tx.send(e) {
            release_evt(&lost);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A console line (`text` with its newline) of a worker: sent to the parent and true (false when not a worker).
pub(crate) fn capture_line(it: &Interp, fd: u8, text: &str) -> bool {
    capture_bytes(it, fd, text.as_bytes())
}

/// The bytes of a worker's write to fd 1 or 2, sent to the parent exactly as given. The parent routes them (to
/// worker.stdout when it asked for capture, else to its own process.stdout / process.stderr). False only when this
/// is not a worker, so the caller writes the process's own fd.
pub(crate) fn capture_bytes(it: &Interp, fd: u8, bytes: &[u8]) -> bool {
    if it.worker.side.is_none() {
        return false;
    }
    to_parent(it, Evt::Stdio(fd, hex(bytes)));
    true
}

/// How long a stopping worker waits for each child it terminated to exit. A child that runs past it (stuck in a native
/// that does not poll the interrupt) is detached: its thread only touches its own state and ends by itself.
// ponytail: one fixed grace per child, not a shared budget; a child stuck forever keeps its thread until process exit.
const STOP_GRACE: Duration = Duration::from_secs(1);

/// Before this Interp is dropped (or its script panicked): each child it still has is terminated and joined, so no child
/// thread outlives the state it reports to. Its own `Exit` is the last event it sends, which ends the drain. A child that
/// does not stop within `STOP_GRACE` is detached instead, with a diagnostic. No 'exit' events are delivered for these
/// children.
fn stop_children(it: &mut Interp) {
    for mut w in std::mem::take(&mut it.worker.workers).into_values() {
        let mut stuck = false;
        if !w.exited {
            w.interrupt.store(true, Ordering::Relaxed);
            let _ = w.tx.send(Cmd::Terminate);
            let deadline = Instant::now() + STOP_GRACE;
            loop {
                match w
                    .rx
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                {
                    Ok(evt) => {
                        let exit = matches!(evt, Evt::Exit(_));
                        release_evt(&evt);
                        if exit {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        stuck = true;
                        break;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        }
        while let Ok(evt) = w.rx.try_recv() {
            release_evt(&evt);
        }
        if let Some(join) = w.join.take() {
            if stuck {
                eprintln!("thaw-js: a worker's child did not stop within {STOP_GRACE:?}; its thread is left running");
            } else {
                let _ = join.join();
            }
        }
    }
}

/// Runs on the worker thread: the script, then its event loop; always ends with an `Exit` event.
fn run_worker(
    source: &str,
    opts: &str,
    stack: usize,
    heap: Option<usize>,
    side: Side,
    env: EnvTable,
    main: Option<MainBundle>,
) {
    let exit_tx = side.tx.clone();
    let key = (side.hub, side.thread_id);
    let code = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut it = Box::new(Interp::runtime());
        // the limit counts what the script allocates, not the runtime built above
        if let Some(bytes) = heap {
            heap_limit::enable(bytes);
        }
        it.configure_stack(stack);
        it.set_stream(true);
        it.interrupt = Some(side.interrupt.clone());
        it.worker.side = Some(side);
        it.worker.env_table = Some(env);
        // a panic in the script still stops its children: the unwind stops here, not at the outer catch
        let ran =
            std::panic::catch_unwind(AssertUnwindSafe(|| run_script(&mut it, source, opts, main)));
        stop_children(&mut it);
        let code = ran.unwrap_or(1);
        // leaving the thread table and taking the queue happen under one lock: a send either queued before this (its
        // wire is released here) or finds no thread. Messages the parent sent that this worker never read still carry
        // their blocks; so does a workerData the bootstrap never reached (its wire is inside the spawn options)
        let unread: Vec<Cmd> = {
            let mut threads = locked(&THREADS);
            threads.remove(&key);
            it.worker
                .side
                .as_ref()
                .map_or_else(Vec::new, |side| side.rx.try_iter().collect())
        };
        for cmd in &unread {
            release_cmd(cmd);
        }
        release_wire(opts);
        code
    }))
    .unwrap_or(1);
    // a panicked worker leaves the table too (its queue went with its channel)
    locked(&THREADS).remove(&key);
    let _ = exit_tx.send(Evt::Exit(code));
}

fn run_script(it: &mut Interp, source: &str, opts: &str, main: Option<MainBundle>) -> i32 {
    it.global
        .declare("__thaw_worker_opts", Value::str(opts), true);
    match it.run_builtin(BOOTSTRAP) {
        Ok(_) => {}
        // terminate() arrived before the bootstrap finished: Node exits 0 (no script ever started)
        Err(Ctl::Exit(_)) => return check_heap(it).unwrap_or(0),
        Err(_) => panic!("worker bootstrap must evaluate"),
    }
    let global = Value::Object(it.global_object.clone());
    let exports = it
        .get(&global, "__thaw_worker_side_exports")
        .unwrap_or(Value::Undefined);
    // an ES-module entry gets no global `require` (its own imports are resolved by the loader)
    let esm = it.worker.side.as_ref().is_some_and(|side| side.esm);
    if !esm {
        it.install_require(vec![
            ("node:worker_threads".into(), exports.clone()),
            ("worker_threads".into(), exports),
        ]);
    }
    to_parent(it, Evt::Online);
    let entry = match &main {
        Some(m) => crate::host::run_bundle_main(it, &m.package, &m.text, &m.key),
        None if esm => it.run(source),
        None => it.run_commonjs(source),
    };
    let r = entry.and_then(|_| it.run_event_loop());
    if let Some(code) = check_heap(it) {
        return code;
    }
    match r {
        Ok(code) => code,
        // process.exit runs the worker's 'exit' listeners; terminate() (the interrupt's Exit) does not
        Err(Ctl::Exit(code)) => {
            if it.rt.shared.exit_requested.get() {
                it.emit_exit_once(code);
            }
            code
        }
        Err(e) => {
            let wire = match e {
                Ctl::Throw(v) => wire_of(it, &v),
                _ => OPAQUE_ERROR.into(),
            };
            to_parent(it, Evt::Error(wire));
            1
        }
    }
}

/// The worker's exit when it tripped its heap limit (`heap_limit`): the ERR_WORKER_OUT_OF_MEMORY error to the parent,
/// then exit code 1. None when the limit was not tripped.
fn check_heap(it: &Interp) -> Option<i32> {
    heap_limit::tripped().then(|| {
        to_parent(it, Evt::Error(OOM_WIRE.into()));
        1
    })
}

fn wire_of(it: &mut Interp, v: &Value) -> String {
    let g = Value::Object(it.global_object.clone());
    let encode = it
        .get(&g, "__thaw_worker_encode")
        .unwrap_or(Value::Undefined);
    match it.call(&encode, &g, std::slice::from_ref(v)) {
        Ok(w) => it.to_str(&w).to_string(),
        Err(_) => OPAQUE_ERROR.into(),
    }
}

/// This interpreter runs as a worker thread (its exit stays local, its stdio is captured by the parent).
pub(crate) fn is_worker(it: &Interp) -> bool {
    it.worker.side.is_some()
}

/// Whether the loop must stay alive for workers: a referenced worker still running on the parent side, or
/// (on a worker thread) a referenced parentPort listener.
pub(crate) fn active(it: &Interp) -> bool {
    let listening = it
        .worker
        .side
        .as_ref()
        .is_some_and(|s| s.alive.load(Ordering::Relaxed));
    listening
        || it.worker.ports_live
        || !it.worker.local.is_empty()
        || it.worker.workers.values().any(|w| w.refd && !w.exited)
}

/// One pending item of a poll, in arrival order: a dispatch JSON item, or a port event to route once the loop over
/// the workers is done (`None` is a hang-up).
enum Item {
    Json(String),
    Route(u64, Option<String>),
}

/// Whether a dispatch item is a port delivery (`port_item`): the only kind a synchronous read may take.
fn is_port_item(json: &str) -> bool {
    json.starts_with(r#"{"type":"port"#)
}

/// Pending worker events as a JSON array (`[]` when none): parent events carry a `handle`, worker events
/// (`message` from the parent) do not. Joins and forgets a worker once its `exit` is delivered.
pub(crate) fn poll(it: &mut Interp) -> String {
    pump(it, false)
}

/// The port deliveries pending now (`[]` when none), for a synchronous receiveMessageOnPort. Every other event goes
/// back to the local queue, so the next loop turn delivers it in its order.
pub(crate) fn poll_ports(it: &mut Interp) -> String {
    pump(it, true)
}

fn pump(it: &mut Interp, ports_only: bool) -> String {
    let mut items: Vec<Item> = it.worker.local.drain(..).map(Item::Json).collect();
    let mut exited_tokens = Vec::new();
    for (id, w) in it.worker.workers.iter_mut() {
        while !w.exited {
            let e = match w.rx.try_recv() {
                Ok(e) => e,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => Evt::Exit(1),
            };
            let json = match e {
                Evt::Online => format!(r#"{{"handle":{id},"type":"online"}}"#),
                Evt::Message(v) => format!(
                    r#"{{"handle":{id},"type":"message","value":{}}}"#,
                    quoted(&v)
                ),
                Evt::Error(v) => {
                    format!(r#"{{"handle":{id},"type":"error","value":{}}}"#, quoted(&v))
                }
                Evt::Exit(code) => {
                    w.exited = true;
                    exited_tokens.push(w.token);
                    format!(r#"{{"handle":{id},"type":"exit","code":{code}}}"#)
                }
                Evt::Stdio(fd, hex) => {
                    let name = if fd == 1 { "stdout" } else { "stderr" };
                    format!(r#"{{"handle":{id},"type":"{name}","value":"{hex}"}}"#)
                }
                Evt::Thread(ack, v) => format!(
                    r#"{{"handle":{id},"type":"thread","ack":{ack},"value":{}}}"#,
                    quoted(&v)
                ),
                Evt::ThreadAck(ack, status, v) => format!(
                    r#"{{"handle":{id},"type":"threadAck","ack":{ack},"status":{},"value":{}}}"#,
                    quoted(&status),
                    quoted(&v)
                ),
                Evt::Port(addr, v) => {
                    items.push(Item::Route(addr, Some(v)));
                    continue;
                }
                Evt::PortClose(addr) => {
                    items.push(Item::Route(addr, None));
                    continue;
                }
                Evt::Cpu(ack, user, system) => format!(
                    r#"{{"handle":{id},"type":"cpu","ack":{ack},"user":{user},"system":{system}}}"#
                ),
                Evt::Heap(ack, usage) => {
                    format!(r#"{{"handle":{id},"type":"heap","ack":{ack},"usage":{usage}}}"#)
                }
            };
            items.push(Item::Json(json));
        }
    }
    // a worker that exited takes its ports down: each peer hangs up (its owner's routes go with it)
    for token in exited_tokens {
        let mut routes = locked(&PORT_ROUTES);
        let owned: Vec<u64> = routes
            .iter()
            .filter(|(_, r)| r.owner == token)
            .map(|(addr, _)| *addr)
            .collect();
        for addr in owned {
            if let Some(route) = routes.remove(&addr) {
                items.push(Item::Route(route.peer, None));
            }
        }
    }
    // the hang-ups this Interp owns (its main token, or one of its workers): others' stay for their own poll
    let mine: Vec<u64> = std::iter::once(it.worker.token)
        .chain(it.worker.workers.values().map(|w| w.token))
        .collect();
    let mut hangups = locked(&PORT_HANGUPS);
    let (ours, rest): (Vec<_>, Vec<_>) = hangups
        .drain(..)
        .partition(|(owner, _)| mine.contains(owner));
    *hangups = rest;
    drop(hangups);
    for (_, peer) in ours {
        items.push(Item::Route(peer, None));
    }
    it.worker.workers.retain(|_, w| {
        if w.exited {
            // events the parent never polled still carry their blocks
            while let Ok(e) = w.rx.try_recv() {
                release_evt(&e);
            }
            if let Some(j) = w.join.take() {
                let _ = j.join();
            }
        }
        !w.exited
    });
    if let Some(side) = &it.worker.side {
        while let Ok(cmd) = side.rx.try_recv() {
            let json = match cmd {
                Cmd::Message(v) => {
                    format!(r#"{{"type":"message","value":{}}}"#, quoted(&v))
                }
                Cmd::Stdin(hex, end) => {
                    format!(r#"{{"type":"stdin","value":"{hex}","end":{end}}}"#)
                }
                Cmd::Thread(source, ack, v) => format!(
                    r#"{{"type":"thread","source":{source},"ack":{ack},"value":{}}}"#,
                    quoted(&v)
                ),
                Cmd::ThreadAck(ack, status, v) => format!(
                    r#"{{"type":"threadAck","ack":{ack},"status":{},"value":{}}}"#,
                    quoted(&status),
                    quoted(&v)
                ),
                Cmd::Port(addr, v) => port_item(addr, Some(&v)),
                Cmd::PortClose(addr) => port_item(addr, None),
                // the parent already set the flag; setting it here too covers a command sent alone
                Cmd::Terminate => {
                    side.interrupt.store(true, Ordering::Relaxed);
                    continue;
                }
            };
            items.push(Item::Json(json));
        }
    }
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Json(json) if ports_only && !is_port_item(&json) => it.worker.local.push(json),
            Item::Json(json) => out.push(json),
            Item::Route(addr, wire) => out.extend(route_port(it, addr, wire)),
        }
    }
    format!("[{}]", out.join(","))
}

pub(crate) fn install(it: &mut Interp) {
    let g = it.global.clone();
    let f = it.native(|it, _, a| {
        let source = it.to_str(&arg(a, 0)).to_string();
        // opts are optional: a bare `spawn(source)` keeps the W2a behaviour
        let opts = if matches!(arg(a, 1), Value::Undefined) {
            "{}".to_string()
        } else {
            it.to_str(&arg(a, 1)).to_string()
        };
        // arg 2 is the heap limit in MiB (-1 = none, enforced by heap_limit); arg 3 is `stackSizeMb` (absent or not
        // positive = the default stack)
        let stack_mb = it.num(&arg(a, 3));
        let heap_mb = it.num(&arg(a, 2));
        // arg 4 is `env: SHARE_ENV` (the worker shares this realm's environment table)
        let share_env = arg(a, 4).truthy();
        // arg 5 is the worker's port token (its port routes name it as their owner); the tree's hub (the main thread's
        // token) is the hub, so a nested worker's ports and thread messages stay in the one namespace
        let token = it.num(&arg(a, 5)) as u64;
        let hub = hub_of(it);
        // arg 6 is the file a non-eval Worker runs (undefined for eval): a bundled module's path runs from the bundle
        let file = match arg(a, 6) {
            Value::Undefined => None,
            v => Some(it.to_str(&v).to_string()),
        };
        let main = file.and_then(|f| main_bundle(it, &f));
        // arg 7 is true for an ES-module entry (`__thaw_run_main_esm`), which runs without the CommonJS globals
        let esm = arg(a, 7).truthy();
        // arg 8 is the worker's thread id (its `threadId`): its key in the thread table
        let thread_id = it.num(&arg(a, 8)) as u32;
        Ok(Value::Number(f64::from(spawn(
            it,
            source,
            opts,
            stack_mb,
            heap_mb,
            share_env,
            (token, hub),
            thread_id,
            main,
            esm,
        ))))
    });
    g.declare("__thaw_worker_spawn", f, true);
    let f = it.native(|it, _, a| {
        let (h, wire) = (it.num(&arg(a, 0)) as u32, it.to_str(&arg(a, 1)).to_string());
        match it.worker.workers.get(&h) {
            Some(w) => send_cmd(w, Cmd::Message(wire)),
            // no such worker: nothing will decode the wire
            None => release_wire(&wire),
        }
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_post", f, true);
    let f = it.native(|it, _, a| {
        let (h, hex, end) = (
            it.num(&arg(a, 0)) as u32,
            it.to_str(&arg(a, 1)).to_string(),
            arg(a, 2).truthy(),
        );
        // false when the worker is gone (exited or its thread dropped the channel): the shim fails the write
        let sent = match it.worker.workers.get(&h) {
            Some(w) if !w.exited => w.tx.send(Cmd::Stdin(hex, end)).is_ok(),
            _ => false,
        };
        Ok(Value::Bool(sent))
    });
    g.declare("__thaw_worker_stdin", f, true);
    // postMessageToThread (W-D), by thread id: a send from this thread (`target` 0 is the main thread) queues on the
    // target's channel, or reports false (the wire is released; the shim settles FAILED). `__thaw_thread_token` is the
    // target's port token (ports in the message are routed by it), or -1 when there is no such thread.
    let f = it.native(|it, _, a| {
        let target = it.num(&arg(a, 0)) as u32;
        let token = if target == 0 && is_worker(it) {
            Some(hub_of(it))
        } else {
            let key = (hub_of(it), target);
            locked(&THREADS).get(&key).map(|entry| entry.token)
        };
        Ok(Value::Number(token.map_or(-1.0, |t| t as f64)))
    });
    g.declare("__thaw_thread_token", f, true);
    // the next `threadId` of this tree (1, 2, ...; the main thread is 0)
    let f = it.native(|it, _, _| {
        Ok(Value::Number(f64::from(
            thread_ids(it).fetch_add(1, Ordering::Relaxed) + 1,
        )))
    });
    g.declare("__thaw_thread_next", f, true);
    let f = it.native(|it, _, a| {
        let (target, ack) = (it.num(&arg(a, 0)) as u32, it.num(&arg(a, 1)) as u32);
        let wire = it.to_str(&arg(a, 2)).to_string();
        Ok(Value::Bool(thread_send(it, target, ack, wire)))
    });
    g.declare("__thaw_thread_send", f, true);
    // answers a send that came from thread `source` (the `threadId` the receiver got with it)
    let f = it.native(|it, _, a| {
        let (source, ack) = (it.num(&arg(a, 0)) as u32, it.num(&arg(a, 1)) as u32);
        let (status, wire) = (
            it.to_str(&arg(a, 2)).to_string(),
            it.to_str(&arg(a, 3)).to_string(),
        );
        thread_reply(it, source, ack, status, wire);
        Ok(Value::Undefined)
    });
    g.declare("__thaw_thread_reply", f, true);
    // a wire that will not be sent (a send to a thread the worker cannot reach): its moved bytes are released
    let f = it.native(|it, _, a| {
        let wire = it.to_str(&arg(a, 0)).to_string();
        release_wire(&wire);
        Ok(Value::Undefined)
    });
    g.declare("__thaw_wire_release", f, true);
    let f = it.native(|it, _, a| {
        let h = it.num(&arg(a, 0)) as u32;
        if let Some(w) = it.worker.workers.get(&h) {
            w.interrupt.store(true, Ordering::Relaxed);
            let _ = w.tx.send(Cmd::Terminate);
        }
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_terminate", f, true);
    // worker.cpuUsage() / worker.getHeapStatistics(): queues `ack` for the worker; false when the worker is gone
    // (JS rejects ERR_WORKER_NOT_RUNNING)
    for (name, heap) in [("__thaw_worker_cpu", false), ("__thaw_worker_heap", true)] {
        let f = it.native(move |it, _, a| {
            let (h, ack) = (it.num(&arg(a, 0)) as u32, it.num(&arg(a, 1)) as u32);
            let sent = match it.worker.workers.get(&h) {
                Some(w) if !w.exited => {
                    w.cpu.request(ack, heap);
                    true
                }
                _ => false,
            };
            Ok(Value::Bool(sent))
        });
        g.declare(name, f, true);
    }
    let f = it.native(|it, _, a| {
        let (h, refd) = (it.num(&arg(a, 0)) as u32, arg(a, 1).truthy());
        if let Some(w) = it.worker.workers.get_mut(&h) {
            w.refd = refd;
        }
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_ref", f, true);
    let f = it.native(|it, _, a| {
        let msg = it.to_str(&arg(a, 0)).to_string();
        to_parent(it, Evt::Message(msg));
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_send", f, true);
    let f = it.native(|it, _, a| {
        let alive = arg(a, 0).truthy();
        if let Some(side) = &it.worker.side {
            side.alive.store(alive, Ordering::Relaxed);
        }
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_set_alive", f, true);
    // MessagePort ends across threads (messaging.js): a fresh address; the owner of an address; a move (the port end
    // `p` goes to thread `dest`, its peer `q` is this thread's when `local`); a landed (decoded) end; a send to an
    // address; a close (the peer is told unless `notify` is false); the loop-liveness of this realm's ports.
    let f = it.native(|_, _, _| Ok(Value::Number(port_address() as f64)));
    g.declare("__thaw_port_address", f, true);
    let f = it.native(|it, _, a| {
        let owner = locked(&PORT_ROUTES)
            .get(&(it.num(&arg(a, 0)) as u64))
            .map_or(0, |r| r.owner);
        Ok(Value::Number(owner as f64))
    });
    g.declare("__thaw_port_owner", f, true);
    // the token a wire to the main thread routes through: the spawning realm's (a worker), else this Interp's own
    let f = it.native(|it, _, _| {
        let hub = match &it.worker.side {
            Some(side) => side.hub,
            None => realm_token(it),
        };
        Ok(Value::Number(hub as f64))
    });
    g.declare("__thaw_port_hub", f, true);
    let f = it.native(|it, _, a| {
        let (p, q) = (it.num(&arg(a, 0)) as u64, it.num(&arg(a, 1)) as u64);
        let dest = it.num(&arg(a, 2)) as u64;
        let peer_here = arg(a, 3).truthy();
        let me = realm_token(it);
        {
            let mut routes = locked(&PORT_ROUTES);
            routes.insert(
                p,
                Route {
                    owner: dest,
                    peer: q,
                },
            );
            if peer_here {
                routes.insert(q, Route { owner: me, peer: p });
            }
        }
        locked(&PORT_FLIGHT).insert(p);
        Ok(Value::Undefined)
    });
    g.declare("__thaw_port_move", f, true);
    let f = it.native(|it, _, a| {
        locked(&PORT_FLIGHT).remove(&(it.num(&arg(a, 0)) as u64));
        Ok(Value::Undefined)
    });
    g.declare("__thaw_port_landed", f, true);
    let f = it.native(|it, _, a| {
        let addr = it.num(&arg(a, 0)) as u64;
        let wire = it.to_str(&arg(a, 1)).to_string();
        if is_worker(it) {
            to_parent(it, Evt::Port(addr, wire));
        } else if let Some(item) = route_port(it, addr, Some(wire)) {
            it.worker.local.push(item);
        }
        Ok(Value::Undefined)
    });
    g.declare("__thaw_port_send", f, true);
    let f = it.native(|it, _, a| {
        let (own, peer) = (it.num(&arg(a, 0)) as u64, it.num(&arg(a, 1)) as u64);
        let notify = arg(a, 2).truthy();
        locked(&PORT_ROUTES).remove(&own);
        if notify {
            if is_worker(it) {
                to_parent(it, Evt::PortClose(peer));
            } else if let Some(item) = route_port(it, peer, None) {
                it.worker.local.push(item);
            }
        }
        Ok(Value::Undefined)
    });
    g.declare("__thaw_port_closed", f, true);
    let f = it.native(|it, _, a| {
        it.worker.ports_live = arg(a, 0).truthy();
        Ok(Value::Undefined)
    });
    g.declare("__thaw_port_live", f, true);
    // receiveMessageOnPort's sync read: the port deliveries already posted by the other threads (JSON, like poll)
    let f = it.native(|it, _, _| Ok(Value::str(&poll_ports(it))));
    g.declare("__thaw_worker_poll_ports", f, true);
}

#[cfg(test)]
mod teardown_tests {
    use super::*;

    #[test]
    fn worker_teardown_gives_up_on_a_child_that_does_not_stop() {
        // a child that ignores its Terminate (a stand-in for a native that never polls the interrupt) and exits 8 s
        // later: the stop returns after the grace, not after the child
        let (tx, cmd_rx) = mpsc::channel::<Cmd>();
        let (evt_tx, rx) = mpsc::channel::<Evt>();
        let join = std::thread::spawn(move || {
            let _commands = cmd_rx;
            std::thread::sleep(Duration::from_secs(8));
            let _ = evt_tx.send(Evt::Exit(0));
        });
        let mut it = Interp::new();
        it.worker.workers.insert(
            1,
            Worker {
                tx,
                rx,
                interrupt: Arc::new(AtomicBool::new(false)),
                refd: true,
                exited: false,
                token: 0,
                join: Some(join),
                cpu: Arc::new(CpuProbe::default()),
            },
        );
        let started = Instant::now();
        stop_children(&mut it);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "teardown waited {:?} for a child that never stopped",
            started.elapsed()
        );
    }
}

#[cfg(test)]
mod transfer_tests {
    use super::*;

    #[test]
    fn release_wire_frees_an_unconsumed_array_buffer() {
        // a wire nobody decodes (its worker ended first) gives its moved bytes back to the table
        let id = ab_export(vec![1, 2, 3], Some(8));
        let wire = format!(r#"{{"root":{{"r":1}},"nodes":[{{"t":"ABT","id":{id}}}]}}"#);
        release_wire(&wire);
        assert!(ab_import(id).is_none());
        // the same node nested in a JSON string (the worker's opts carry workerData that way)
        let id = ab_export(vec![9], None);
        let wire = format!(r#"{{"root":{{"r":1}},"nodes":[{{"t":"ABT","id":{id}}}]}}"#);
        release_wire(&format!(
            r#"{{"workerData":"{}"}}"#,
            wire.replace('"', r#"\""#)
        ));
        assert!(ab_import(id).is_none());
        // a decode takes the entry once; releasing the wire afterwards finds nothing
        let id = ab_export(vec![7, 7], None);
        let wire = format!(r#"{{"root":{{"r":1}},"nodes":[{{"t":"ABT","id":{id}}}]}}"#);
        assert_eq!(ab_import(id), Some((vec![7, 7], None)));
        release_wire(&wire);
        assert!(ab_import(id).is_none());
    }

    #[test]
    fn release_wire_frees_a_port_in_flight_and_hangs_up_its_peer() {
        // a port end encoded for a worker (in flight, owner 7) whose wire nobody decodes: its route goes, and the
        // peer end (q) is told to hang up
        let (p, q) = (port_address(), port_address());
        let (worker, main) = (port_address(), port_address());
        locked(&PORT_ROUTES).insert(
            p,
            Route {
                owner: worker,
                peer: q,
            },
        );
        locked(&PORT_ROUTES).insert(
            q,
            Route {
                owner: main,
                peer: p,
            },
        );
        locked(&PORT_FLIGHT).insert(p);
        let wire =
            format!(r#"{{"root":{{"r":1}},"nodes":[{{"t":"Port","p":{p},"q":{q},"qs":[]}}]}}"#);
        release_wire(&wire);
        assert!(!locked(&PORT_ROUTES).contains_key(&p));
        assert!(locked(&PORT_HANGUPS).contains(&(main, q)));
        // a landed end (its reader decoded the wire) is not freed by releasing the same wire again
        let (p, q) = (port_address(), port_address());
        locked(&PORT_ROUTES).insert(p, Route { owner: 0, peer: q });
        locked(&PORT_FLIGHT).insert(p);
        locked(&PORT_FLIGHT).remove(&p);
        let wire =
            format!(r#"{{"root":{{"r":1}},"nodes":[{{"t":"Port","p":{p},"q":{q},"qs":[]}}]}}"#);
        release_wire(&wire);
        assert!(locked(&PORT_ROUTES).contains_key(&p));
        assert!(!locked(&PORT_HANGUPS).iter().any(|(_, peer)| *peer == q));
    }
}
