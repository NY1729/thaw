//! `node:worker_threads` workers (W2a): one OS thread and one `Interp` per worker. Messages cross threads as the
//! wire strings of worker_wire.js, so this module only moves `String`s. The parent side is the `TABLE` of the
//! spawning thread; the worker side is the `SIDE` of the worker's own thread. The event loop drains both via [`poll`].

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;

use crate::builtins::json_quote;
use crate::*;

/// Stack reserved for a worker's JS (virtual memory, committed on use).
const STACK: usize = 64 << 20;
/// Wire of an error value that could not itself be encoded.
const OPAQUE_ERROR: &str = r#"{"root":{"v":"uncaught exception"},"nodes":[]}"#;

enum Evt {
    Online,
    Message(String),
    Error(String),
    Exit(i32),
}

struct Worker {
    tx: Sender<String>,
    rx: Receiver<Evt>,
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
    rx: Receiver<String>,
    tx: Sender<Evt>,
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

fn spawn(source: String) -> u32 {
    let (tx, cmd_rx) = mpsc::channel();
    let (evt_tx, rx) = mpsc::channel();
    let join = std::thread::Builder::new()
        .name("thaw-js-worker".into())
        .stack_size(STACK)
        .spawn(move || run_worker(&source, cmd_rx, evt_tx))
        .expect("worker thread spawns");
    TABLE.with(|t| {
        let mut t = t.borrow_mut();
        t.next += 1;
        let id = t.next;
        let worker = Worker {
            tx,
            rx,
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
fn run_worker(source: &str, rx: Receiver<String>, tx: Sender<Evt>) {
    SIDE.with(|s| *s.borrow_mut() = Some(Side { rx, tx }));
    to_parent(Evt::Online);
    let code = std::panic::catch_unwind(AssertUnwindSafe(|| run_script(source))).unwrap_or(1);
    to_parent(Evt::Exit(code));
}

fn run_script(source: &str) -> i32 {
    let mut it = Box::new(Interp::new());
    it.configure_stack(STACK);
    it.set_stream(true);
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

/// Whether a referenced worker is still running: the parent's loop stays alive for it.
pub(crate) fn active() -> bool {
    TABLE.with(|t| t.borrow().workers.values().any(|w| w.refd && !w.exited))
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
            while let Ok(v) = side.rx.try_recv() {
                items.push(format!(r#"{{"type":"message","value":{}}}"#, quoted(&v)));
            }
        }
    });
    format!("[{}]", items.join(","))
}

pub(crate) fn install(it: &mut Interp) {
    let g = it.global.clone();
    let f = it.native(|it, _, a| {
        let source = it.to_str(&arg(a, 0)).to_string();
        Ok(Value::Number(f64::from(spawn(source))))
    });
    g.declare("__thaw_worker_spawn", f, true);
    let f = it.native(|it, _, a| {
        let (h, wire) = (it.num(&arg(a, 0)) as u32, it.to_str(&arg(a, 1)).to_string());
        TABLE.with(|t| {
            if let Some(w) = t.borrow().workers.get(&h) {
                let _ = w.tx.send(wire);
            }
        });
        Ok(Value::Undefined)
    });
    g.declare("__thaw_worker_post", f, true);
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
}
