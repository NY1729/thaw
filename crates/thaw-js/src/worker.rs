//! `node:worker_threads` workers (W2a/W2b): one OS thread and one `Interp` per worker. Messages cross threads as the
//! wire strings of worker_wire.js, so this module only moves `String`s. The parent side is the `workers` table of the
//! spawning `Interp` (`WorkerState`); the worker side is its `side`. The event loop drains both via [`poll`].
//! The state lives on the `Interp`, not in a thread-local: async functions and generators run on coroutine threads
//! that call these natives with the same `Interp`, so the state must follow the `Interp`, not the OS thread.
//! `terminate()` sets an interrupt flag that the worker's `Interp` checks (`Interp::check_interrupt`), so busy loops stop.

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::builtins::json_quote;
use crate::*;

/// Stack reserved for a worker's JS (virtual memory, committed on use).
const STACK: usize = 64 << 20;
/// Wire of an error value that could not itself be encoded.
const OPAQUE_ERROR: &str = r#"{"root":{"v":"uncaught exception"},"nodes":[]}"#;
/// The worker-side bootstrap (worker_bootstrap.js), run in the worker's own realm before its source.
const BOOTSTRAP: &str = include_str!("web/worker_bootstrap.js");

/// Parent to worker commands. `Terminate` also sets the interrupt flag, which is what stops a busy worker.
enum Cmd {
    Message(String),
    /// Stdin bytes as hex, and whether this is the end of the stream (`worker.stdin.end()`).
    Stdin(String, bool),
    Terminate,
}

enum Evt {
    Online,
    Message(String),
    Error(String),
    Exit(i32),
    /// Captured stdout (fd 1) or stderr (fd 2) bytes as hex (only when the parent asked for them).
    Stdio(u8, String),
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
}

fn quoted(s: &str) -> String {
    let mut out = String::new();
    json_quote(s, &mut out);
    out
}

fn spawn(it: &mut Interp, source: String, opts: String) -> u32 {
    let (tx, cmd_rx) = mpsc::channel();
    let (evt_tx, rx) = mpsc::channel();
    let interrupt = Arc::new(AtomicBool::new(false));
    let flag = interrupt.clone();
    let join = std::thread::Builder::new()
        .name("thaw-js-worker".into())
        .stack_size(STACK)
        .spawn(move || run_worker(&source, &opts, cmd_rx, evt_tx, flag))
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
    rx: Receiver<Cmd>,
    tx: Sender<Evt>,
    interrupt: Arc<AtomicBool>,
) {
    let exit_tx = tx.clone();
    let code = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut it = Box::new(Interp::new());
        it.configure_stack(STACK);
        it.set_stream(true);
        it.interrupt = Some(interrupt.clone());
        it.worker.side = Some(Side {
            rx,
            tx,
            interrupt,
            alive: Arc::new(AtomicBool::new(false)),
            capture_out: false,
            capture_err: false,
        });
        run_script(&mut it, source, opts)
    }))
    .unwrap_or(1);
    let _ = exit_tx.send(Evt::Exit(code));
}

fn run_script(it: &mut Interp, source: &str, opts: &str) -> i32 {
    it.global
        .declare("__thaw_worker_opts", Value::str(opts), true);
    match it.run(BOOTSTRAP) {
        Ok(_) => {}
        // terminate() arrived before the bootstrap finished: the worker exits with the interrupt's code
        Err(Ctl::Exit(code)) => return code,
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
            });
        }
    }
    it.worker.workers.retain(|_, w| {
        if w.exited {
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
        Ok(Value::Number(f64::from(spawn(it, source, opts))))
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
