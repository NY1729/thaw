//! `node:worker_threads` workers (W2a/W2b): one OS thread and one `Interp` per worker. Messages cross threads as the
//! wire strings of worker_wire.js, so this module only moves `String`s. The parent side is the `TABLE` of the
//! spawning thread; the worker side is the `SIDE` of the worker's own thread. The event loop drains both via [`poll`].
//! `terminate()` sets an interrupt flag that the worker's `Interp` checks (`Interp::check_interrupt`), so busy loops stop.

use std::cell::RefCell;
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
    Terminate,
}

enum Evt {
    Online,
    Message(String),
    Error(String),
    Exit(i32),
}

struct Worker {
    tx: Sender<Cmd>,
    rx: Receiver<Evt>,
    interrupt: Arc<AtomicBool>,
    refd: bool,
    exited: bool,
    join: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct Table {
    next: u32,
    workers: BTreeMap<u32, Worker>,
}

struct Side {
    rx: Receiver<Cmd>,
    tx: Sender<Evt>,
    interrupt: Arc<AtomicBool>,
    /// The worker's parentPort has a referenced `message` listener (set by the bootstrap).
    alive: bool,
}

thread_local! {
    static TABLE: RefCell<Table> = RefCell::new(Table::default());
    static SIDE: RefCell<Option<Side>> = const { RefCell::new(None) };
}

fn quoted(s: &str) -> String {
    let mut out = String::new();
    json_quote(s, &mut out);
    out
}

fn spawn(source: String, opts: String) -> u32 {
    let (tx, cmd_rx) = mpsc::channel();
    let (evt_tx, rx) = mpsc::channel();
    let interrupt = Arc::new(AtomicBool::new(false));
    let flag = interrupt.clone();
    let join = std::thread::Builder::new()
        .name("thaw-js-worker".into())
        .stack_size(STACK)
        .spawn(move || run_worker(&source, &opts, cmd_rx, evt_tx, flag))
        .expect("worker thread spawns");
    TABLE.with(|t| {
        let mut t = t.borrow_mut();
        t.next += 1;
        let id = t.next;
        let worker = Worker {
            tx,
            rx,
            interrupt,
            refd: true,
            exited: false,
            join: Some(join),
        };
        t.workers.insert(id, worker);
        id
    })
}

fn to_parent(e: Evt) {
    SIDE.with(|s| {
        if let Some(side) = s.borrow().as_ref() {
            let _ = side.tx.send(e);
        }
    });
}

/// Runs on the worker thread: the script, then its event loop; always ends with an `Exit` event.
fn run_worker(
    source: &str,
    opts: &str,
    rx: Receiver<Cmd>,
    tx: Sender<Evt>,
    interrupt: Arc<AtomicBool>,
) {
    let flag = interrupt.clone();
    SIDE.with(|s| {
        *s.borrow_mut() = Some(Side {
            rx,
            tx,
            interrupt,
            alive: false,
        })
    });
    let code =
        std::panic::catch_unwind(AssertUnwindSafe(|| run_script(source, opts, flag))).unwrap_or(1);
    to_parent(Evt::Exit(code));
}

fn run_script(source: &str, opts: &str, interrupt: Arc<AtomicBool>) -> i32 {
    let mut it = Box::new(Interp::new());
    it.configure_stack(STACK);
    it.set_stream(true);
    it.interrupt = Some(interrupt);
    it.global
        .declare("__thaw_worker_opts", Value::str(opts), true);
    it.run(BOOTSTRAP)
        .unwrap_or_else(|_| panic!("worker bootstrap must evaluate"));
    let global = Value::Object(it.global_object.clone());
    let exports = it
        .get(&global, "__thaw_worker_side_exports")
        .unwrap_or(Value::Undefined);
    it.install_require(vec![
        ("node:worker_threads".into(), exports.clone()),
        ("worker_threads".into(), exports),
    ]);
    to_parent(Evt::Online);
    let r = it.run_commonjs(source).and_then(|_| it.run_event_loop());
    match r {
        Ok(code) | Err(Ctl::Exit(code)) => code,
        Err(e) => {
            let wire = match e {
                Ctl::Throw(v) => wire_of(&mut it, &v),
                _ => OPAQUE_ERROR.into(),
            };
            to_parent(Evt::Error(wire));
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
pub(crate) fn active() -> bool {
    let listening = SIDE.with(|s| s.borrow().as_ref().is_some_and(|s| s.alive));
    listening || TABLE.with(|t| t.borrow().workers.values().any(|w| w.refd && !w.exited))
}

/// Pending worker events as a JSON array (`[]` when none): parent events carry a `handle`, worker events
/// (`message` from the parent) do not. Joins and forgets a worker once its `exit` is delivered.
pub(crate) fn poll() -> String {
    let mut items = Vec::new();
    TABLE.with(|t| {
        let mut t = t.borrow_mut();
        for (id, w) in t.workers.iter_mut() {
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
                });
            }
        }
        t.workers.retain(|_, w| {
            if w.exited {
                if let Some(j) = w.join.take() {
                    let _ = j.join();
                }
            }
            !w.exited
        });
    });
    SIDE.with(|s| {
        if let Some(side) = s.borrow().as_ref() {
            while let Ok(cmd) = side.rx.try_recv() {
                match cmd {
                    Cmd::Message(v) => {
                        items.push(format!(r#"{{"type":"message","value":{}}}"#, quoted(&v)))
                    }
                    // the parent already set the flag; setting it here too covers a command sent alone
                    Cmd::Terminate => side.interrupt.store(true, Ordering::Relaxed),
                }
            }
        }
    });
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
        Ok(Value::Number(f64::from(spawn(source, opts))))
    });
    g.declare("__thaw_worker_spawn", f, true);
    let f = it.native(|it, _, a| {
        let (h, wire) = (it.num(&arg(a, 0)) as u32, it.to_str(&arg(a, 1)).to_string());
        TABLE.with(|t| {
            if let Some(w) = t.borrow().workers.get(&h) {
                let _ = w.tx.send(Cmd::Message(wire));
            }
        });
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_post", f, true);
    let f = it.native(|it, _, a| {
        let h = it.num(&arg(a, 0)) as u32;
        TABLE.with(|t| {
            if let Some(w) = t.borrow().workers.get(&h) {
                w.interrupt.store(true, Ordering::Relaxed);
                let _ = w.tx.send(Cmd::Terminate);
            }
        });
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_terminate", f, true);
    let f = it.native(|it, _, a| {
        let (h, refd) = (it.num(&arg(a, 0)) as u32, arg(a, 1).truthy());
        TABLE.with(|t| {
            if let Some(w) = t.borrow_mut().workers.get_mut(&h) {
                w.refd = refd;
            }
        });
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_ref", f, true);
    let f = it.native(|it, _, a| {
        to_parent(Evt::Message(it.to_str(&arg(a, 0)).to_string()));
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_send", f, true);
    let f = it.native(|_, _, a| {
        let alive = arg(a, 0).truthy();
        SIDE.with(|s| {
            if let Some(side) = s.borrow_mut().as_mut() {
                side.alive = alive;
            }
        });
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_set_alive", f, true);
}
