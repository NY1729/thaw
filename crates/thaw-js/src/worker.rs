//! `node:worker_threads` workers (W2a/W2b): one OS thread and one `Interp` per worker. Messages cross threads as the
//! wire strings of worker_wire.js, so this module only moves `String`s. The parent side is the `workers` table of the
//! spawning `Interp` (`WorkerState`); the worker side is its `side`. The event loop drains both via [`poll`].
//! The state lives on the `Interp`, not in a thread-local: async functions and generators run on coroutine threads
//! that call these natives with the same `Interp`, so the state must follow the `Interp`, not the OS thread.
//! `terminate()` sets an interrupt flag that the worker's `Interp` checks (`Interp::check_interrupt`), so busy loops stop.

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

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
}

struct Worker {
    tx: Sender<Cmd>,
    rx: Receiver<Evt>,
    interrupt: Arc<AtomicBool>,
    refd: bool,
    exited: bool,
    join: Option<JoinHandle<()>>,
}

struct Side {
    rx: Receiver<Cmd>,
    tx: Sender<Evt>,
    interrupt: Arc<AtomicBool>,
    /// The worker's parentPort has a referenced `message` listener (set by the bootstrap).
    alive: Arc<AtomicBool>,
    /// The parent passed `stdout: true` / `stderr: true`: console output goes to the parent, not the process fd.
    capture_out: bool,
    capture_err: bool,
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

/// Releases the blocks a wire carries when nobody will decode it. The wire is JSON and quotes inside strings are
/// escaped, so every `"t":"SAB","id":N` found here is a node the encoder wrote.
pub(crate) fn release_wire(wire: &str) {
    const NODE: &str = r#""t":"SAB","id":"#;
    let mut rest = wire;
    while let Some(at) = rest.find(NODE) {
        rest = &rest[at + NODE.len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(id) = digits.parse::<u64>() {
            sab_table().remove(&id);
        }
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

fn spawn(
    it: &mut Interp,
    source: String,
    opts: String,
    stack_mb: f64,
    heap_mb: f64,
    share_env: bool,
) -> u32 {
    let (tx, cmd_rx) = mpsc::channel();
    let (evt_tx, rx) = mpsc::channel();
    let interrupt = Arc::new(AtomicBool::new(false));
    let stack = stack_bytes(stack_mb);
    let heap = heap_bytes(heap_mb);
    // a SHARE_ENV worker reads and writes the table of this realm; any other worker gets a table of its own
    let env = if share_env {
        share_env_table(it)
    } else {
        private_env_table()
    };
    let side = Side {
        rx: cmd_rx,
        tx: evt_tx,
        interrupt: interrupt.clone(),
        alive: Arc::new(AtomicBool::new(false)),
        capture_out: false,
        capture_err: false,
    };
    let join = std::thread::Builder::new()
        .name("thaw-js-worker".into())
        .stack_size(stack)
        .spawn(move || run_worker(&source, &opts, stack, heap, side, env))
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
            join: Some(join),
        },
    );
    id
}

fn to_parent(it: &Interp, e: Evt) {
    if let Some(side) = &it.worker.side {
        let _ = side.tx.send(e);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A console line (`text` with its newline) of a worker whose parent captures that fd: sent to the parent and true;
/// false when it stays on the process's own fd (the default).
pub(crate) fn capture_line(it: &Interp, fd: u8, text: &str) -> bool {
    let Some(side) = &it.worker.side else {
        return false;
    };
    let on = if fd == 1 {
        side.capture_out
    } else {
        side.capture_err
    };
    if on {
        to_parent(it, Evt::Stdio(fd, hex(text.as_bytes())));
    }
    on
}

/// Runs on the worker thread: the script, then its event loop; always ends with an `Exit` event.
fn run_worker(
    source: &str,
    opts: &str,
    stack: usize,
    heap: Option<usize>,
    side: Side,
    env: EnvTable,
) {
    let exit_tx = side.tx.clone();
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
        let code = run_script(&mut it, source, opts);
        // messages the parent sent that this worker never read still carry their blocks
        if let Some(side) = &it.worker.side {
            while let Ok(cmd) = side.rx.try_recv() {
                if let Cmd::Message(v) | Cmd::Thread(_, _, v) = cmd {
                    release_wire(&v);
                }
            }
        }
        code
    }))
    .unwrap_or(1);
    let _ = exit_tx.send(Evt::Exit(code));
}

fn run_script(it: &mut Interp, source: &str, opts: &str) -> i32 {
    it.global
        .declare("__thaw_worker_opts", Value::str(opts), true);
    match it.run_builtin(BOOTSTRAP) {
        Ok(_) => {}
        // terminate() arrived before the bootstrap finished: the worker exits with the interrupt's code
        Err(Ctl::Exit(code)) => return check_heap(it).unwrap_or(code),
        Err(_) => panic!("worker bootstrap must evaluate"),
    }
    let global = Value::Object(it.global_object.clone());
    let exports = it
        .get(&global, "__thaw_worker_side_exports")
        .unwrap_or(Value::Undefined);
    it.install_require(vec![
        ("node:worker_threads".into(), exports.clone()),
        ("worker_threads".into(), exports),
    ]);
    to_parent(it, Evt::Online);
    let r = it.run_commonjs(source).and_then(|_| it.run_event_loop());
    if let Some(code) = check_heap(it) {
        return code;
    }
    match r {
        Ok(code) | Err(Ctl::Exit(code)) => code,
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
    listening || it.worker.workers.values().any(|w| w.refd && !w.exited)
}

/// Pending worker events as a JSON array (`[]` when none): parent events carry a `handle`, worker events
/// (`message` from the parent) do not. Joins and forgets a worker once its `exit` is delivered.
pub(crate) fn poll(it: &mut Interp) -> String {
    let mut items = Vec::new();
    for (id, w) in it.worker.workers.iter_mut() {
        while !w.exited {
            let e = match w.rx.try_recv() {
                Ok(e) => e,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => Evt::Exit(1),
            };
            items.push(match e {
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
            });
        }
    }
    it.worker.workers.retain(|_, w| {
        if w.exited {
            // events the parent never polled still carry their blocks
            while let Ok(e) = w.rx.try_recv() {
                if let Evt::Message(v) | Evt::Error(v) | Evt::Thread(_, v) = e {
                    release_wire(&v);
                }
            }
            if let Some(j) = w.join.take() {
                let _ = j.join();
            }
        }
        !w.exited
    });
    if let Some(side) = &it.worker.side {
        while let Ok(cmd) = side.rx.try_recv() {
            match cmd {
                Cmd::Message(v) => {
                    items.push(format!(r#"{{"type":"message","value":{}}}"#, quoted(&v)))
                }
                Cmd::Stdin(hex, end) => {
                    items.push(format!(r#"{{"type":"stdin","value":"{hex}","end":{end}}}"#))
                }
                Cmd::Thread(source, ack, v) => items.push(format!(
                    r#"{{"type":"thread","source":{source},"ack":{ack},"value":{}}}"#,
                    quoted(&v)
                )),
                Cmd::ThreadAck(ack, status, v) => items.push(format!(
                    r#"{{"type":"threadAck","ack":{ack},"status":{},"value":{}}}"#,
                    quoted(&status),
                    quoted(&v)
                )),
                // the parent already set the flag; setting it here too covers a command sent alone
                Cmd::Terminate => side.interrupt.store(true, Ordering::Relaxed),
            }
        }
    }
    format!("[{}]", items.join(","))
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
        Ok(Value::Number(f64::from(spawn(
            it, source, opts, stack_mb, heap_mb, share_env,
        ))))
    });
    g.declare("__thaw_worker_spawn", f, true);
    let f = it.native(|it, _, a| {
        let (h, wire) = (it.num(&arg(a, 0)) as u32, it.to_str(&arg(a, 1)).to_string());
        if let Some(w) = it.worker.workers.get(&h) {
            let _ = w.tx.send(Cmd::Message(wire));
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
        if let Some(w) = it.worker.workers.get(&h) {
            let _ = w.tx.send(Cmd::Stdin(hex, end));
        }
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_stdin", f, true);
    // postMessageToThread (W-D): the parent sends to a worker (`source` is the sender's thread id) and answers a
    // worker's send (`ack` names the worker's pending send); a worker sends to the parent and answers a parent's send.
    let f = it.native(|it, _, a| {
        let h = it.num(&arg(a, 0)) as u32;
        let (source, ack) = (it.num(&arg(a, 1)) as u32, it.num(&arg(a, 2)) as u32);
        let wire = it.to_str(&arg(a, 3)).to_string();
        if let Some(w) = it.worker.workers.get(&h) {
            let _ = w.tx.send(Cmd::Thread(source, ack, wire));
        }
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_thread_send", f, true);
    let f = it.native(|it, _, a| {
        let h = it.num(&arg(a, 0)) as u32;
        let ack = it.num(&arg(a, 1)) as u32;
        let (status, wire) = (
            it.to_str(&arg(a, 2)).to_string(),
            it.to_str(&arg(a, 3)).to_string(),
        );
        if let Some(w) = it.worker.workers.get(&h) {
            let _ = w.tx.send(Cmd::ThreadAck(ack, status, wire));
        }
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_thread_ack", f, true);
    let f = it.native(|it, _, a| {
        let ack = it.num(&arg(a, 0)) as u32;
        let wire = it.to_str(&arg(a, 1)).to_string();
        to_parent(it, Evt::Thread(ack, wire));
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_thread_to_parent", f, true);
    let f = it.native(|it, _, a| {
        let ack = it.num(&arg(a, 0)) as u32;
        let (status, wire) = (
            it.to_str(&arg(a, 1)).to_string(),
            it.to_str(&arg(a, 2)).to_string(),
        );
        to_parent(it, Evt::ThreadAck(ack, status, wire));
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_thread_reply", f, true);
    let f = it.native(|it, _, a| {
        let h = it.num(&arg(a, 0)) as u32;
        if let Some(w) = it.worker.workers.get(&h) {
            w.interrupt.store(true, Ordering::Relaxed);
            let _ = w.tx.send(Cmd::Terminate);
        }
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_terminate", f, true);
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
    let f = it.native(|it, _, a| {
        let (out, err) = (arg(a, 0).truthy(), arg(a, 1).truthy());
        if let Some(side) = &mut it.worker.side {
            side.capture_out = out;
            side.capture_err = err;
        }
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_capture", f, true);
}
