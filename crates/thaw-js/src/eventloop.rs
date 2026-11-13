//! Promise, microtask queue, async functions, timers, the event loop and a minimal `process`.
//!
//! Ordering follows Node: after the main script and after every macrotask (timer / immediate)
//! the `process.nextTick` queue drains first, then the promise-job queue, repeating until both
//! are empty; unhandled rejections are checked at the end of each such drain.
//!
//! Async functions run on a coroutine: a dedicated OS thread with a strict hand-off protocol
//! (exactly one of {host, coroutine} runs at any time, synchronised by channels), so the
//! tree-walking evaluator can be suspended at `await` without being rewritten in CPS.
//! ponytail: thread per in-flight async call (~64 MiB virtual stack each, lazily committed);
//! a CPS/bytecode evaluator (M5) replaces it. An `Interp` must not be moved while a coroutine
//! is suspended (`Package` boxes it; `co_step` asserts it).

use crate::*;
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::host::{adopt_host_tls, park_host_tls};

/// Timer/immediate ids are unique across every realm and thread (async bodies run on coroutine threads).
static NEXT_TIMER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_timer_id() -> u64 {
    NEXT_TIMER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

const TIMEOUT_MAX: f64 = 2147483647.0;
const CO_STACK: usize = 64 << 20;
const MAIN_FILE_LOADER: &str = include_str!("web/main_file.js");
const PROCESS_STDIN: &str = include_str!("web/process_stdin.js");
/// Per-thread evaluator state (depth, stack_limit, max_depth, run_id) carried across coroutine switches.
type CoSnapshot = (usize, usize, usize, u32);

pub(crate) enum PState {
    Pending,
    Fulfilled(Value),
    Rejected(Value),
}

pub(crate) struct PromiseData {
    state: PState,
    reactions: Vec<Reaction>,
    handled: bool,
}

struct Reaction {
    on_ful: Value,
    on_rej: Value,
    result: Option<Obj>,
    frame: Option<Value>,
}

enum Job {
    React {
        handler: Value,
        arg: Value,
        ok: bool,
        result: Option<Obj>,
    },
    Thenable {
        promise: Obj,
        thenable: Value,
        then: Value,
    },
    Call(Value, Vec<Value>),
}

struct Timer {
    id: u64,
    expiry: f64,
    seq: u64,
    delay: f64,
    repeat: bool,
    cb: Value,
    args: Vec<Value>,
    obj: Obj,
    refd: bool,
    frame: Option<Value>,
}

struct Imm {
    id: u64,
    cb: Value,
    args: Vec<Value>,
    obj: Obj,
    refd: bool,
    frame: Option<Value>,
}

// ----- coroutines -----

pub(crate) enum Resume {
    Value(Value),
    Throw(Value),
    /// `generator.return(v)`: unwinds the generator body (running `finally` blocks).
    Return(Value),
}

pub(crate) enum Yield {
    Await(Value),
    /// `yield v` inside a generator body.
    Yielded(Value),
    Done(R<Value>),
}

/// Moves `Rc`-bearing data across the hand-off; sound only because of the strict alternation.
pub(crate) struct SendBox<T>(pub(crate) T);
// SAFETY: see the module docs; only one side touches the payload at a time.
unsafe impl<T> Send for SendBox<T> {}
impl<T> SendBox<T> {
    pub(crate) fn into_inner(self) -> T {
        self.0
    }
}

type ReadyHook = extern "C" fn();

/// The native runtime's ready-continuation hand-off (`thaw_runtime_park_ready` / `thaw_runtime_adopt_ready`),
/// resolved through RTLD_DEFAULT like `drain_native_continuations`. Missing symbols make both no-ops.
static NATIVE_READY_HOOKS: std::sync::OnceLock<Option<(ReadyHook, ReadyHook)>> =
    std::sync::OnceLock::new();

/// Test override for the hand-off hooks (process-wide: coroutine threads must see it too).
#[cfg(test)]
pub(crate) static TEST_READY_HOOKS: std::sync::Mutex<Option<(ReadyHook, ReadyHook)>> =
    std::sync::Mutex::new(None);

/// Test-only count of coroutines that parked because their host dropped its channel ends.
#[cfg(test)]
pub(crate) static DISCONNECT_PARKS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// A suspended coroutine whose host dropped its channel ends: park forever. Callers must not
/// touch the Interp, TLS hand-offs or native-ready state before reaching this.
pub(crate) fn park_disconnected() -> ! {
    #[cfg(test)]
    DISCONNECT_PARKS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    loop {
        std::thread::park();
    }
}

fn ready_hooks() -> Option<(ReadyHook, ReadyHook)> {
    #[cfg(test)]
    if let Some(hooks) = *TEST_READY_HOOKS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
    {
        return Some(hooks);
    }
    *NATIVE_READY_HOOKS.get_or_init(resolve_native_ready_hooks)
}

#[cfg(unix)]
fn resolve_native_ready_hooks() -> Option<(ReadyHook, ReadyHook)> {
    // SAFETY: the symbols are the compiled runtime's `extern "C" fn()`s, looked up like the other natives.
    unsafe {
        let park = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_runtime_park_ready".as_ptr());
        let adopt = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_runtime_adopt_ready".as_ptr());
        if park.is_null() || adopt.is_null() {
            return None;
        }
        Some((
            std::mem::transmute::<*mut std::ffi::c_void, ReadyHook>(park),
            std::mem::transmute::<*mut std::ffi::c_void, ReadyHook>(adopt),
        ))
    }
}

#[cfg(not(unix))]
fn resolve_native_ready_hooks() -> Option<(ReadyHook, ReadyHook)> {
    None
}

/// Coroutine side, just before every hand-back: the continuations it settled move to the parked queue.
pub(crate) fn park_native_ready() {
    if let Some((park, _)) = ready_hooks() {
        park();
    }
}

/// Host side, right after a coroutine hands back: the parked continuations join this thread's queue.
pub(crate) fn adopt_native_ready() {
    if let Some((_, adopt)) = ready_hooks() {
        adopt();
    }
}

pub(crate) struct CoCtx {
    pub(crate) to_host: Sender<SendBox<Yield>>,
    pub(crate) from_host: Receiver<SendBox<Resume>>,
    /// The body of an async generator: `yield` awaits its operand first.
    pub(crate) agen: bool,
}

pub(crate) struct Co {
    pub(crate) to_co: Sender<SendBox<Resume>>,
    pub(crate) from_co: Receiver<SendBox<Yield>>,
    pub(crate) ctx: Rc<CoCtx>,
    interp: *mut Interp,
    promise: Obj,
    /// Set for an async generator: its yields and completion are routed to the generator driver.
    pub(crate) gen: Option<Obj>,
}

#[derive(Default)]
pub(crate) struct Rt {
    micro: VecDeque<(Option<Value>, Job)>,
    ticks: VecDeque<(Option<Value>, Value, Vec<Value>)>,
    unhandled: Vec<Obj>,
    /// The AsyncLocalStorage frame (an opaque JS value; None = undefined) in effect right now.
    pub(crate) actx: Option<Value>,
    pub(crate) cur_co: Vec<Rc<CoCtx>>,
    timers: Vec<Timer>,
    immediates: Vec<Imm>,
    cleared: Vec<u64>,
    pub(crate) arraybuffer_proto: Option<Obj>,
    /// SharedArrayBuffer.prototype; separate from ArrayBuffer.prototype (not an ArrayBuffer).
    pub(crate) sharedarraybuffer_proto: Option<Obj>,
    pub(crate) typed_base_proto: Option<Obj>,
    /// Prototype per `Ty`, in `Ty as usize` order.
    pub(crate) typed_protos: Vec<Obj>,
    /// Set by `construct` for the duration of entering a native constructor (`new Date()` vs `Date()`).
    pub(crate) constructing: bool,
    pub(crate) to_prim_key: Option<Rc<str>>,
    /// Every symbol by id, so `Object.getOwnPropertySymbols` can map `@@sym:<id>` keys back.
    pub(crate) symbols: HashMap<u64, Obj>,
    seq: u64,
    /// The loop turn's clock: the due test for timers. Timers take their start from the live clock.
    now: f64,
    /// True until the first loop turn starts: timers armed before it (by the main script) get a 2 ms
    /// head start, so that delays of 2 ms or less are due on the first turn, ahead of `setImmediate`.
    first_turn: bool,
    listeners: HashMap<String, Vec<(Value, bool)>>,
    promise_proto: Option<Obj>,
    timeout_proto: Option<Obj>,
    immediate_proto: Option<Obj>,
    pub(crate) has_instance_key: Option<Rc<str>>,
    pub(crate) pending_nt: Option<Value>,
    pub(crate) bigint_proto: Option<Obj>,
    pub(crate) generator_proto: Option<Obj>,
    pub(crate) async_generator_proto: Option<Obj>,
    process: Option<Obj>,
    /// Print console output as it happens instead of buffering it in `Interp::out`.
    pub(crate) stream: bool,
    partial: String,
    /// The process state every realm of this host thread shares (`exitCode`, the exit request).
    pub(crate) shared: std::rc::Rc<crate::procstate::ProcessShared>,
    /// The `exit` event was emitted for this realm; it is emitted at most once.
    pub(crate) exit_emitted: bool,
    /// This realm started `process.stdin`, so its loop polls the stdin reader.
    pub(crate) stdin_started: bool,
}

/// Node's `process.kill` signal names with their Linux numbers (the QuickJS runtime.js table).
const PROCESS_SIGNALS: [(&str, i32); 15] = [
    ("SIGHUP", libc::SIGHUP),
    ("SIGINT", libc::SIGINT),
    ("SIGQUIT", libc::SIGQUIT),
    ("SIGABRT", libc::SIGABRT),
    ("SIGKILL", libc::SIGKILL),
    ("SIGUSR1", libc::SIGUSR1),
    ("SIGSEGV", libc::SIGSEGV),
    ("SIGUSR2", libc::SIGUSR2),
    ("SIGPIPE", libc::SIGPIPE),
    ("SIGALRM", libc::SIGALRM),
    ("SIGTERM", libc::SIGTERM),
    ("SIGCHLD", libc::SIGCHLD),
    ("SIGCONT", libc::SIGCONT),
    ("SIGSTOP", libc::SIGSTOP),
    ("SIGTSTP", libc::SIGTSTP),
];

/// A Node-style error of `kind` with a `code` property, ready to throw.
fn node_error(it: &mut Interp, kind: &str, code: &str, msg: &str) -> Ctl {
    match it.make_error(kind, msg) {
        Ok(e) => {
            if let Value::Object(o) = &e {
                put(o, "code", Value::str(code));
            }
            Ctl::Throw(e)
        }
        Err(c) => c,
    }
}

/// Resident set size in bytes: the second `/proc/self/statm` field (pages); 0 when unreadable.
fn process_rss_bytes() -> u64 {
    let pages = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .unwrap_or(0);
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as u64;
    pages * page
}

/// Monotonic nanoseconds for `process.hrtime` and `process.hrtime.bigint` (offset by 1s, as before).
fn hrtime_ns() -> u64 {
    (clock_ms() * 1e6) as u64 + 1_000_000_000
}

fn clock_ms() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

thread_local! {
    // per thread: a worker runs its own event loop
    static LOOP_START: Cell<Option<Instant>> = const { Cell::new(None) };
    static LOOP_IDLE: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

fn mark_loop_started() {
    LOOP_START.with(|s| {
        if s.get().is_none() {
            s.set(Some(Instant::now()));
        }
    });
}

/// The loop has nothing to run: sleep, and count the time as idle.
fn loop_idle_sleep(d: Duration) {
    mark_loop_started();
    std::thread::sleep(d);
    LOOP_IDLE.with(|i| i.set(i.get() + d));
}

/// (active_ms, idle_ms) since the loop started; (0, 0) before it did.
fn host_loop_stats() -> (f64, f64) {
    match LOOP_START.with(|s| s.get()) {
        None => (0.0, 0.0),
        Some(start) => {
            let elapsed = start.elapsed();
            let idle = LOOP_IDLE.with(|i| i.get()).min(elapsed);
            (
                (elapsed - idle).as_secs_f64() * 1000.0,
                idle.as_secs_f64() * 1000.0,
            )
        }
    }
}

impl Interp {
    // ----- console plumbing (shared with process.stdout) -----

    pub(crate) fn emit_out(&mut self, text: &str, newline: bool) {
        let mut s = std::mem::take(&mut self.rt.partial);
        s.push_str(text);
        if newline {
            s.push('\n');
        }
        while let Some(i) = s.find('\n') {
            let line: String = s.drain(..=i).collect();
            let line = &*crate::builtins::lossy_out(&line[..line.len() - 1]);
            if self.rt.stream {
                if !crate::worker::capture_line(self, 1, &format!("{line}\n")) {
                    println!("{line}");
                }
            } else {
                self.out.push(line.to_string());
            }
        }
        self.rt.partial = s;
    }

    pub(crate) fn emit_err(&mut self, line: &str) {
        if self.rt.stream {
            if !crate::worker::capture_line(self, 2, &format!("{line}\n")) {
                eprintln!("{line}");
            }
        } else {
            self.err.push(line.to_string());
        }
    }

    /// `process.stderr.write` on the main thread: the bytes as written (no added newline), after any partial stdout line.
    pub(crate) fn emit_err_raw(&mut self, bytes: &[u8]) {
        self.flush_partial();
        let _ = std::io::Write::write_all(&mut std::io::stderr(), bytes);
    }

    /// `process.stdout.write` of a typed array or ArrayBuffer on the main thread: the bytes as they are, flushed now.
    pub(crate) fn emit_out_raw(&mut self, bytes: &[u8]) {
        self.flush_partial();
        let mut out = std::io::stdout().lock();
        let _ = std::io::Write::write_all(&mut out, bytes)
            .and_then(|()| std::io::Write::flush(&mut out));
    }

    /// Makes `shared` this realm's process state (`process.exitCode`, the exit request). Call before any script runs.
    pub(crate) fn share_process_state(
        &mut self,
        shared: std::rc::Rc<crate::procstate::ProcessShared>,
    ) {
        self.rt.shared = shared;
    }

    /// `process.exitCode = code` (None is `undefined`), as every realm of the host sees it.
    pub(crate) fn set_exit_code(&mut self, code: Option<f64>) {
        self.rt.shared.exit_code.set(code);
    }

    // ----- promises -----

    pub(crate) fn new_promise(&self) -> Obj {
        new_obj(
            self.rt.promise_proto.clone(),
            Kind::Promise(PromiseData {
                state: PState::Pending,
                reactions: Vec::new(),
                handled: false,
            }),
        )
    }

    fn need_callback(&mut self, cb: &Value) -> R<()> {
        if cb.is_callable() {
            return Ok(());
        }
        let shown = self.inspect(cb, false);
        let e = self.make_error(
            "TypeError",
            &format!(
                "The \"callback\" argument must be of type function. Received {}",
                received(cb, &shown)
            ),
        )?;
        if let Value::Object(o) = &e {
            put(o, "code", Value::str("ERR_INVALID_ARG_TYPE"));
        }
        Err(Ctl::Throw(e))
    }

    fn make_error(&mut self, class: &str, msg: &str) -> R<Value> {
        match self.throw_error::<()>(class, msg) {
            Err(Ctl::Throw(v)) => Ok(v),
            Err(e) => Err(e),
            Ok(()) => unreachable!(),
        }
    }

    fn settle(&mut self, p: &Obj, st: PState) {
        let (reactions, ok, val, handled) = {
            let mut k = p.kind.borrow_mut();
            let Kind::Promise(d) = &mut *k else { return };
            if !matches!(d.state, PState::Pending) {
                return;
            }
            let (ok, val) = match &st {
                PState::Fulfilled(v) => (true, v.clone()),
                PState::Rejected(v) => (false, v.clone()),
                PState::Pending => return,
            };
            d.state = st;
            (std::mem::take(&mut d.reactions), ok, val, d.handled)
        };
        if !ok && !handled {
            self.rt.unhandled.push(p.clone());
        }
        for r in reactions {
            self.rt.micro.push_back((
                r.frame,
                Job::React {
                    handler: if ok { r.on_ful } else { r.on_rej },
                    arg: val.clone(),
                    ok,
                    result: r.result,
                },
            ));
        }
    }

    pub(crate) fn reject_promise(&mut self, p: &Obj, reason: Value) {
        self.settle(p, PState::Rejected(reason));
    }

    /// The promise resolve procedure (adopts thenables through a job, like the spec).
    pub(crate) fn resolve_promise(&mut self, p: &Obj, v: Value) -> R<()> {
        if let Value::Object(o) = &v {
            if Rc::ptr_eq(o, p) {
                let e = self.make_error(
                    "TypeError",
                    "Chaining cycle detected for promise #<Promise>",
                )?;
                self.reject_promise(p, e);
                return Ok(());
            }
            let then = match self.get(&v, "then") {
                Ok(t) => t,
                Err(Ctl::Throw(e)) => {
                    self.reject_promise(p, e);
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            if then.is_callable() {
                self.rt.micro.push_back((
                    self.rt.actx.clone(),
                    Job::Thenable {
                        promise: p.clone(),
                        thenable: v,
                        then,
                    },
                ));
                return Ok(());
            }
        }
        self.settle(p, PState::Fulfilled(v));
        Ok(())
    }

    /// `resolve` / `reject` function pair sharing one "already resolved" flag.
    fn resolvers(&mut self, p: &Obj) -> (Value, Value) {
        let done = Rc::new(Cell::new(false));
        let (p1, d1) = (p.clone(), done.clone());
        let res = self.native(move |it, _, a| {
            if !d1.replace(true) {
                it.resolve_promise(&p1, arg(a, 0))?;
            }
            Ok(Value::Undefined)
        });
        let p2 = p.clone();
        let rej = self.native(move |it, _, a| {
            if !done.replace(true) {
                it.reject_promise(&p2, arg(a, 0));
            }
            Ok(Value::Undefined)
        });
        (res, rej)
    }

    fn promise_then(&mut self, p: &Obj, on_ful: Value, on_rej: Value, result: Option<Obj>) {
        let (ok, val) = {
            let mut k = p.kind.borrow_mut();
            let Kind::Promise(d) = &mut *k else { return };
            d.handled = true;
            match &d.state {
                PState::Pending => {
                    d.reactions.push(Reaction {
                        on_ful,
                        on_rej,
                        result,
                        frame: self.rt.actx.clone(),
                    });
                    return;
                }
                PState::Fulfilled(v) => (true, v.clone()),
                PState::Rejected(v) => (false, v.clone()),
            }
        };
        self.rt.micro.push_back((
            self.rt.actx.clone(),
            Job::React {
                handler: if ok { on_ful } else { on_rej },
                arg: val,
                ok,
                result,
            },
        ));
    }

    /// `PromiseResolve(%Promise%, v)`.
    fn promise_resolve_value(&mut self, v: Value) -> R<Obj> {
        if let Value::Object(o) = &v {
            if matches!(&*o.kind.borrow(), Kind::Promise(_)) {
                return Ok(o.clone());
            }
        }
        let p = self.new_promise();
        self.resolve_promise(&p, v)?;
        Ok(p)
    }

    /// Run `f` with `frame` as the current async-context frame; the previous frame is restored afterwards.
    pub(crate) fn in_frame<T>(
        &mut self,
        frame: Option<Value>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let prev = std::mem::replace(&mut self.rt.actx, frame);
        let r = f(self);
        self.rt.actx = prev;
        r
    }

    fn run_job(&mut self, job: Job) -> R<()> {
        match job {
            Job::React {
                handler,
                arg,
                ok,
                result,
            } => {
                let outcome = if handler.is_callable() {
                    self.call(&handler, &Value::Undefined, &[arg])
                } else if ok {
                    Ok(arg)
                } else {
                    Err(Ctl::Throw(arg))
                };
                match (outcome, result) {
                    (Ok(v), Some(r)) => self.resolve_promise(&r, v),
                    (Err(Ctl::Throw(e)), Some(r)) => {
                        self.reject_promise(&r, e);
                        Ok(())
                    }
                    (Err(e @ (Ctl::Unsupported(_) | Ctl::Exit(_))), _) => Err(e),
                    _ => Ok(()),
                }
            }
            Job::Thenable {
                promise,
                thenable,
                then,
            } => {
                let (res, rej) = self.resolvers(&promise);
                match self.call(&then, &thenable, &[res, rej.clone()]) {
                    Err(Ctl::Throw(e)) => {
                        self.call(&rej, &Value::Undefined, &[e])?;
                        Ok(())
                    }
                    Err(e) => Err(e),
                    Ok(_) => Ok(()),
                }
            }
            Job::Call(f, args) => match self.call(&f, &Value::Undefined, &args) {
                Err(e) => self.uncaught(e),
                Ok(_) => Ok(()),
            },
        }
    }

    /// Route an exception that escaped a callback to `process.on('uncaughtException')`, if any.
    fn uncaught(&mut self, e: Ctl) -> R<()> {
        match e {
            Ctl::Throw(v) => {
                // Node: every `uncaughtExceptionMonitor` listener runs first, then the `uncaughtException` handler
                if self
                    .rt
                    .listeners
                    .get("uncaughtExceptionMonitor")
                    .is_some_and(|l| !l.is_empty())
                {
                    self.emit(
                        "uncaughtExceptionMonitor",
                        &[v.clone(), Value::str("uncaughtException")],
                    )?;
                }
                if self
                    .rt
                    .listeners
                    .get("uncaughtException")
                    .is_some_and(|l| !l.is_empty())
                {
                    self.emit("uncaughtException", &[v, Value::str("uncaughtException")])?;
                    Ok(())
                } else {
                    Err(Ctl::Throw(v))
                }
            }
            e => Err(e),
        }
    }

    /// `__thaw_report_uncaught(error)`: hand an error to `process.on('uncaughtException')` or rethrow it.
    pub(crate) fn report_uncaught(&mut self, v: Value) -> R<Value> {
        self.uncaught(Ctl::Throw(v)).map(|()| Value::Undefined)
    }

    /// Drain `nextTick` callbacks, then promise jobs, until both queues are empty; then report
    /// unhandled rejections (which may queue more work).
    pub fn run_jobs(&mut self) -> R<()> {
        loop {
            loop {
                self.drain_ticks()?;
                while let Some((fr, j)) = self.rt.micro.pop_front() {
                    self.in_frame(fr, |it| it.run_job(j))?;
                }
                if self.rt.ticks.is_empty() && self.rt.micro.is_empty() {
                    break;
                }
            }
            let pending = std::mem::take(&mut self.rt.unhandled);
            for p in pending {
                let reason = {
                    let k = p.kind.borrow();
                    match &*k {
                        Kind::Promise(d) if !d.handled => match &d.state {
                            PState::Rejected(v) => v.clone(),
                            _ => continue,
                        },
                        _ => continue,
                    }
                };
                if self
                    .rt
                    .listeners
                    .get("unhandledRejection")
                    .is_some_and(|l| !l.is_empty())
                {
                    self.emit("unhandledRejection", &[reason, Value::Object(p)])?;
                } else if matches!(&reason, Value::Object(o) if !matches!(&*o.kind.borrow(), Kind::Closure{..}|Kind::Native(_)) && self.get(&reason, "message").is_ok_and(|m| !matches!(m, Value::Undefined)))
                {
                    return Err(Ctl::Throw(reason));
                } else {
                    let shown = self.inspect(&reason, false);
                    let e = self.make_error(
                        "Error",
                        &format!("This error originated either by throwing inside of an async function without a catch block, or by rejecting a promise which was not handled with .catch(). The promise rejected with the reason \"{shown}\"."),
                    )?;
                    if let Value::Object(o) = &e {
                        put(o, "code", Value::str("ERR_UNHANDLED_REJECTION"));
                    }
                    return Err(Ctl::Throw(e));
                }
            }
            if self.rt.ticks.is_empty() && self.rt.micro.is_empty() {
                return Ok(());
            }
        }
    }

    /// Run queued `nextTick` callbacks only (promise jobs stay queued). The routed entry drains ticks before
    /// the native promise queue, as Node does for a main's own ticks.
    pub fn drain_ticks(&mut self) -> R<()> {
        while let Some((fr, f, a)) = self.rt.ticks.pop_front() {
            if let Err(e) = self.in_frame(fr, |it| it.call(&f, &Value::Undefined, &a)) {
                self.uncaught(e)?;
            }
        }
        Ok(())
    }

    /// Fatal exit of a routed build: queued timers, immediates and jobs are dropped (Node runs none of them
    /// after an uncaught exception).
    pub(crate) fn drop_pending_work(&mut self) {
        self.rt.timers.clear();
        self.rt.immediates.clear();
        self.rt.ticks.clear();
        self.rt.micro.clear();
    }

    /// Whether this realm already emitted `exit` (it runs no more work after that).
    pub(crate) fn exit_emitted(&self) -> bool {
        self.rt.exit_emitted
    }

    /// Whether this realm has a `beforeExit` or `exit` listener (the idle pass is only observable then).
    pub(crate) fn exit_listeners(&self) -> bool {
        ["beforeExit", "exit"]
            .iter()
            .any(|name| self.rt.listeners.get(*name).is_some_and(|l| !l.is_empty()))
    }

    /// One native-continuation checkpoint (`turn_hook`), if the host installed one. The compiled runtime
    /// drains its promise jobs here, so a promise a due timer queued runs before the next due timer.
    fn turn_checkpoint(&mut self) {
        if let Some(hook) = self.turn_hook {
            hook(self);
        }
    }

    // ----- async functions / await -----

    /// Call an async closure: run its body on a coroutine up to the first `await`.
    pub(crate) fn call_async(
        &mut self,
        f: &Value,
        def: &Rc<FuncDef>,
        scope: &Rc<Scope>,
        this: &Value,
        args: &[Value],
    ) -> R<Value> {
        let promise = self.new_promise();
        let (co, snap) = self.new_co(f, def, scope, this, args, promise.clone(), None)?;
        self.co_wait(&co, snap)?;
        Ok(Value::Object(promise))
    }

    /// Start the coroutine thread for a closure body. On success the coroutine's context is on
    /// `cur_co` and the caller must `co_wait` (or the generator equivalent) on the result.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_co(
        &mut self,
        f: &Value,
        def: &Rc<FuncDef>,
        scope: &Rc<Scope>,
        this: &Value,
        args: &[Value],
        promise: Obj,
        gen: Option<Obj>,
    ) -> R<(Rc<Co>, CoSnapshot)> {
        let (to_co, from_host) = channel();
        let (to_host, from_co) = channel();
        let done_tx = to_host.clone();
        let ctx = Rc::new(CoCtx {
            to_host,
            from_host,
            agen: gen.is_some(),
        });
        let co = Rc::new(Co {
            to_co,
            from_co,
            ctx: ctx.clone(),
            interp: self as *mut Interp,
            promise,
            gen,
        });
        let payload = SendBox((
            self as *mut Interp,
            f.clone(),
            def.clone(),
            scope.clone(),
            this.clone(),
            args.to_vec(),
        ));
        self.rt.cur_co.push(ctx);
        let snap = self.snapshot();
        let key = self as *mut Interp as usize;
        park_host_tls(key);
        let spawned = std::thread::Builder::new()
            .stack_size(CO_STACK)
            .spawn(move || {
                adopt_host_tls(key);
                let r = {
                    let (ip, f, def, scope, this, args) = payload.into_inner();
                    // SAFETY: the host thread is blocked in `co_wait` until we send a message.
                    let it = unsafe { &mut *ip };
                    it.configure_stack(CO_STACK);
                    it.call_closure_sync(&f, &def, &scope, &this, &args)
                    // every Rc captured above is dropped here, before the hand-back
                };
                park_native_ready();
                // ponytail: each finished coroutine thread leaks its bump chunks (see
                // thaw_arena_detach_thread). Upgrade path: adopt the arena into the host thread.
                thaw_arena::thaw_arena_detach_thread();
                park_host_tls(key);
                let _ = done_tx.send(SendBox(Yield::Done(r)));
            });
        if spawned.is_err() {
            adopt_host_tls(key);
            self.rt.cur_co.pop();
            return self.throw_error("RangeError", "cannot start coroutine (thread limit)");
        }
        Ok((co, snap))
    }

    /// Per-thread evaluator state that must follow the thread across coroutine switches.
    pub(crate) fn snapshot(&self) -> CoSnapshot {
        (self.depth, self.stack_limit, self.max_depth, self.run_id)
    }

    pub(crate) fn restore(&mut self, s: CoSnapshot) {
        (self.depth, self.stack_limit, self.max_depth, self.run_id) = s;
    }

    /// Block until the coroutine yields or finishes, then act on what it did.
    pub(crate) fn co_wait(&mut self, co: &Rc<Co>, snap: CoSnapshot) -> R<()> {
        let y = co.from_co.recv();
        adopt_native_ready();
        adopt_host_tls(self as *mut Interp as usize);
        self.restore(snap);
        self.rt.cur_co.pop();
        match y.map(SendBox::into_inner) {
            Ok(Yield::Await(v)) => {
                let p = self.promise_resolve_value(v)?;
                let (c1, c2) = (co.clone(), co.clone());
                let on_ful = self.native(move |it, _, a| {
                    it.co_step(&c1, Resume::Value(arg(a, 0)))?;
                    Ok(Value::Undefined)
                });
                let on_rej = self.native(move |it, _, a| {
                    it.co_step(&c2, Resume::Throw(arg(a, 0)))?;
                    Ok(Value::Undefined)
                });
                self.promise_then(&p, on_ful, on_rej, None);
                Ok(())
            }
            Ok(Yield::Yielded(v)) => self.agen_yielded(co, v),
            Ok(Yield::Done(r)) if co.gen.is_some() => self.agen_done(co, r),
            Ok(Yield::Done(Ok(v))) => self.resolve_promise(&co.promise, v),
            Ok(Yield::Done(Err(Ctl::Throw(e)))) => {
                self.reject_promise(&co.promise, e);
                Ok(())
            }
            Ok(Yield::Done(Err(other))) => Err(other),
            Err(_) => panic!("async function thread terminated unexpectedly"),
        }
    }

    pub(crate) fn co_step(&mut self, co: &Rc<Co>, resume: Resume) -> R<()> {
        assert!(
            std::ptr::eq(co.interp, self),
            "an Interp must not be moved while an async function is suspended"
        );
        self.rt.cur_co.push(co.ctx.clone());
        let snap = self.snapshot();
        park_host_tls(self as *mut Interp as usize);
        let _ = co.to_co.send(SendBox(resume));
        self.co_wait(co, snap)
    }

    /// `await v` inside a coroutine.
    pub(crate) fn await_value(&mut self, v: Value) -> R<Value> {
        let Some(ctx) = self.rt.cur_co.last().cloned() else {
            return Err(Ctl::Unsupported("await".into()));
        };
        let snap = self.snapshot();
        park_native_ready();
        park_host_tls(self as *mut Interp as usize);
        let _ = ctx.to_host.send(SendBox(Yield::Await(v)));
        // the host runs from here until it resumes us
        // check the disconnect before touching `self` (the Interp may be torn down by the host)
        let Ok(msg) = ctx.from_host.recv() else {
            // the host dropped this coroutine (promise never settled): never touch shared state again
            park_disconnected()
        };
        adopt_host_tls(self as *mut Interp as usize);
        self.restore(snap);
        match msg.into_inner() {
            Resume::Value(v) => Ok(v),
            Resume::Throw(e) => Err(Ctl::Throw(e)),
            Resume::Return(v) => Err(Ctl::Return(v)),
        }
    }

    // ----- timers / event loop -----

    /// Run `f` from the next timer round (keeps the loop alive until it has run).
    pub(crate) fn defer_tick(&mut self, f: Value) -> R<()> {
        self.add_timer(&[f, Value::Number(0.0)], false).map(|_| ())
    }

    fn add_timer(&mut self, args: &[Value], repeat: bool) -> R<Value> {
        let cb = arg(args, 0);
        self.need_callback(&cb)?;
        let mut delay = match arg(args, 1) {
            Value::Undefined => 1.0,
            v => self.num(&v),
        };
        if !(1.0..=TIMEOUT_MAX).contains(&delay) {
            delay = 1.0;
        }
        self.rt.seq += 1;
        let id = next_timer_id();
        let obj = new_obj(self.rt.timeout_proto.clone(), Kind::Plain);
        put_hidden(&obj, "@@tid", Value::Number(id as f64));
        self.rt.timers.push(Timer {
            id,
            // the head start is for timers armed before the first loop turn only
            expiry: clock_ms() + delay - if self.rt.first_turn { 2.0 } else { 0.0 },
            seq: self.rt.seq,
            delay,
            repeat,
            cb,
            args: args.get(2..).unwrap_or(&[]).to_vec(),
            obj: obj.clone(),
            refd: true,
            frame: self.rt.actx.clone(),
        });
        Ok(Value::Object(obj))
    }

    /// The id of a Timeout/Immediate object (what crosses the host boundary in its place).
    pub(crate) fn timer_handle_id(&mut self, v: &Value) -> Option<u64> {
        let Value::Object(o) = v else { return None };
        find_own(o, "@@tid")?;
        self.timer_id(v)
    }

    /// The live Timeout/Immediate object for `id`, or the bare id once it is gone.
    pub(crate) fn timer_object(&self, id: u64) -> Value {
        self.rt
            .timers
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.obj.clone())
            .or_else(|| {
                self.rt
                    .immediates
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.obj.clone())
            })
            .map_or(Value::Number(id as f64), Value::Object)
    }

    fn timer_id(&mut self, v: &Value) -> Option<u64> {
        match v {
            Value::Number(n) => Some(*n as u64),
            Value::Str(s) => s.parse().ok(),
            Value::Object(o) => match find_own(o, "@@tid") {
                Some((Slot::Data(Value::Number(n)), _)) => Some(n as u64),
                _ => None,
            },
            _ => None,
        }
    }

    /// Remove timer/immediate `id` here, else in the realm that owns it. Ids are unique across realms.
    pub(crate) fn clear_timer_anywhere(&mut self, id: u64, immediate: bool) {
        let n = self.rt.timers.len() + self.rt.immediates.len();
        self.rt.timers.retain(|t| t.id != id);
        self.rt.immediates.retain(|t| t.id != id);
        if immediate {
            self.rt.cleared.push(id);
        }
        if self.rt.timers.len() + self.rt.immediates.len() != n {
            return;
        }
        crate::host::for_other_realms(self, |other| {
            other.rt.timers.retain(|t| t.id != id);
            other.rt.immediates.retain(|t| t.id != id);
            if immediate {
                other.rt.cleared.push(id);
            }
        });
    }

    /// ref/unref/hasRef on a Timeout/Immediate. `Some(r)` sets, `None` reads. Here first, else in the realm that owns it,
    /// else (entry already cleared or fired) the value stored on the object.
    fn set_ref(&mut self, this: &Value, refd: Option<bool>) -> bool {
        let Some(id) = self.timer_id(this) else {
            return false;
        };
        // The flag also lives on the object, so a cleared timer (no table entry) still reports it.
        if let (Some(r), Value::Object(o)) = (refd, this) {
            put_hidden(o, "@@refd", Value::Bool(r));
        }
        let mut found = self.set_ref_local(id, refd);
        if found.is_none() {
            crate::host::for_other_realms(self, |other| {
                if let Some(r) = other.set_ref_local(id, refd) {
                    found = Some(r);
                }
            });
        }
        found.unwrap_or_else(|| self.stored_ref(this))
    }

    /// Ref state of a timer whose table entry is gone: the last `ref`/`unref` value, else Node's default
    /// (Timeout/Interval true; Immediate false, as Node reports for a cleared or fired one).
    fn stored_ref(&self, this: &Value) -> bool {
        let Value::Object(o) = this else {
            return false;
        };
        match find_own(o, "@@refd") {
            Some((Slot::Data(Value::Bool(b)), _)) => b,
            _ => {
                let proto = o.proto.borrow().clone();
                matches!((proto, &self.rt.timeout_proto), (Some(p), Some(t)) if Rc::ptr_eq(&p, t))
            }
        }
    }

    /// `None` when `id` is not in this realm's tables.
    fn set_ref_local(&mut self, id: u64, refd: Option<bool>) -> Option<bool> {
        if let Some(t) = self.rt.timers.iter_mut().find(|t| t.id == id) {
            if let Some(r) = refd {
                t.refd = r;
            }
            return Some(t.refd);
        }
        let t = self.rt.immediates.iter_mut().find(|t| t.id == id)?;
        if let Some(r) = refd {
            t.refd = r;
        }
        Some(t.refd)
    }

    /// `Timeout.refresh`: restart the delay, in the owning realm (using its clock).
    fn refresh_timer(&mut self, id: u64) {
        let now = clock_ms();
        if let Some(t) = self.rt.timers.iter_mut().find(|t| t.id == id) {
            t.expiry = now + t.delay;
            return;
        }
        crate::host::for_other_realms(self, |other| {
            let now = clock_ms();
            if let Some(t) = other.rt.timers.iter_mut().find(|t| t.id == id) {
                t.expiry = now + t.delay;
            }
        });
    }

    fn run_timers(&mut self) -> R<()> {
        loop {
            let now = self.rt.now;
            let next = self
                .rt
                .timers
                .iter()
                .enumerate()
                .filter(|(_, t)| t.expiry <= now)
                .min_by(|(_, a), (_, b)| {
                    a.expiry
                        .partial_cmp(&b.expiry)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(a.seq.cmp(&b.seq))
                })
                .map(|(i, _)| i);
            let Some(i) = next else { return Ok(()) };
            let (id, cb, args, obj, repeat, delay, frame) = {
                let t = &mut self.rt.timers[i];
                let r = (
                    t.id,
                    t.cb.clone(),
                    t.args.clone(),
                    t.obj.clone(),
                    t.repeat,
                    t.delay,
                    t.frame.clone(),
                );
                if t.repeat {
                    t.expiry = f64::INFINITY; // parked while its callback runs
                } else {
                    self.rt.timers.remove(i);
                }
                r
            };
            // a repeat restarts from the clock read before its callback runs (as Node does)
            let start = if repeat { clock_ms() } else { 0.0 };
            if let Err(e) = self.in_frame(frame, |it| it.call(&cb, &Value::Object(obj), &args)) {
                self.uncaught(e)?;
            }
            if repeat {
                self.rt.seq += 1;
                let seq = self.rt.seq;
                if let Some(t) = self.rt.timers.iter_mut().find(|t| t.id == id) {
                    t.expiry = start + delay;
                    t.seq = seq;
                }
            }
            self.run_jobs()?;
            self.turn_checkpoint();
        }
    }

    fn run_immediates(&mut self) -> R<()> {
        let batch = std::mem::take(&mut self.rt.immediates);
        for im in batch {
            // a clearImmediate from an earlier callback of this batch removes later ones
            if self.rt.cleared.contains(&im.id) {
                continue;
            }
            if let Err(e) = self.in_frame(im.frame.clone(), |it| {
                it.call(&im.cb, &Value::Object(im.obj.clone()), &im.args)
            }) {
                self.uncaught(e)?;
            }
            self.run_jobs()?;
            self.turn_checkpoint();
        }
        self.rt.cleared.clear();
        Ok(())
    }

    /// Drive jobs, timers and immediates until the promise `v` settles or no work remains; a
    /// non-promise settles at once. Does not emit `beforeExit`/`exit`.
    pub fn await_settled(&mut self, v: &Value) -> R<Option<Result<Value, Value>>> {
        if !self.is_promise(v) {
            return Ok(Some(Ok(v.clone())));
        }
        self.mark_handled(v);
        mark_loop_started();
        loop {
            self.run_jobs()?;
            if let Some(state) = self.promise_state(v) {
                return Ok(Some(state));
            }
            if !self.loop_alive() {
                return Ok(None);
            }
            self.rt.first_turn = false;
            self.rt.now = clock_ms();
            self.run_timers()?;
            // the loop's own platform poll: a worker's exit (or child/zlib result) settles promises too
            self.poll_platform_events()?;
            if !self.rt.immediates.is_empty() {
                self.run_immediates()?;
                continue;
            }
            // a timer or event just settled the promise: do not sleep toward the next unrelated timer
            if self.promise_state(v).is_some() {
                continue;
            }
            if let Some(next) = self
                .rt
                .timers
                .iter()
                .map(|t| t.expiry)
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            {
                let mut wait = next - clock_ms();
                // a pending napi completion is polled every 2 ms, not only at the next timer
                if crate::napi_bridge::pending() {
                    wait = wait.min(2.0);
                }
                if wait > 0.0 {
                    loop_idle_sleep(Duration::from_secs_f64(wait / 1000.0));
                }
            } else if crate::child::active()
                || crate::zlib::active_for(self.realm_id)
                || crate::worker::active(self)
                || self.stdin_live()
                || crate::napi_bridge::pending()
            {
                // nothing timed to wait for, but a platform event will arrive: poll again shortly
                loop_idle_sleep(Duration::from_millis(2));
            }
        }
    }

    /// Referenced timers/immediates or queued jobs remain.
    pub fn work_pending(&self) -> bool {
        self.loop_alive() || !self.rt.ticks.is_empty() || !self.rt.micro.is_empty()
    }

    fn loop_alive(&self) -> bool {
        self.rt.timers.iter().any(|t| t.refd)
            || self.rt.immediates.iter().any(|i| i.refd)
            || crate::child::active()
            || crate::zlib::active_for(self.realm_id)
            || crate::worker::active(self)
            || self.stdin_live()
            || crate::napi_bridge::pending()
    }

    /// This realm reads `process.stdin` and the reader is still open and referenced.
    fn stdin_live(&self) -> bool {
        self.rt.stdin_started && crate::sysio::stdin_active()
    }

    /// Call the shim-installed `globalThis.__thaw_poll_platform_events` (async zlib jobs, child process output and exits),
    /// then hand pending worker events to `globalThis.__thaw_worker_dispatch`.
    fn poll_platform_events(&mut self) -> R<()> {
        crate::napi_bridge::poll();
        let g = Value::Object(self.global_object.clone());
        let f = self.get(&g, "__thaw_poll_platform_events")?;
        if matches!(f, Value::Object(_)) {
            self.call(&f, &g, &[])?;
            self.run_jobs()?;
        } else if crate::child::active() {
            // ponytail: this realm has no child_process hook (a call made through another package, e.g.
            // promisify(execFile)); the realm that owns the shim's `children` map drains the thread-wide table.
            // Ceiling: two realms each running their own shim can drain each other's events; upgrade = per-handle owner.
            let mut failed = None;
            crate::host::for_other_realms(self, |other| {
                if failed.is_some() {
                    return;
                }
                let og = Value::Object(other.global_object.clone());
                match other.get(&og, "__thaw_poll_platform_events") {
                    Ok(h @ Value::Object(_)) => {
                        if let Err(e) = other.call(&h, &og, &[]).and_then(|_| other.run_jobs()) {
                            failed = Some(e);
                        }
                    }
                    Ok(_) => {}
                    Err(e) => failed = Some(e),
                }
            });
            if let Some(e) = failed {
                return Err(e);
            }
        }
        let events = crate::worker::poll(self);
        if events != "[]" {
            let f = self.get(&g, "__thaw_worker_dispatch")?;
            if matches!(f, Value::Object(_)) {
                self.call(&f, &g, &[Value::str(&events)])?;
                self.run_jobs()?;
            }
        }
        if self.rt.stdin_started {
            let events = crate::sysio::poll_stdin();
            if events != "[]" {
                let f = self.get(&g, "__thaw_stdin_dispatch")?;
                if matches!(f, Value::Object(_)) {
                    self.call(&f, &g, &[Value::str(&events)])?;
                    self.run_jobs()?;
                }
            }
        }
        Ok(())
    }

    fn loop_inner(&mut self) -> R<()> {
        mark_loop_started();
        self.run_jobs()?;
        let mut first = true;
        while self.loop_alive() {
            self.check_interrupt()?;
            // ponytail: wall-clock slice, taken only at an iteration boundary so timer/immediate order within this
            // realm is unchanged. Upgrade: yield on a blocking native call too.
            if !std::mem::replace(&mut first, false) {
                if let Some(end) = self.slice_end {
                    if Instant::now() >= end && crate::host::other_packages_have_work() {
                        self.yielded = true;
                        return Ok(());
                    }
                }
            }
            self.rt.first_turn = false;
            self.rt.now = clock_ms();
            self.poll_platform_events()?;
            self.run_timers()?;
            // the compiled host's native continuations: a timer just resolved a native Promise they wait on
            if let Some(hook) = self.turn_hook {
                if hook(self) {
                    continue;
                }
            }
            if !self.rt.immediates.is_empty() {
                self.run_immediates()?;
                continue;
            }
            if !self.loop_alive() {
                break;
            }
            if let Some(next) = self
                .rt
                .timers
                .iter()
                .map(|t| t.expiry)
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            {
                let mut wait = next - clock_ms();
                // a worker wakes every 2ms so a terminate() is seen even while a timer is pending
                // a live compiled http server is polled every 2 ms too, so a long timer does not starve it
                if crate::child::active()
                    || crate::zlib::active_for(self.realm_id)
                    || crate::worker::active(self)
                    || self.stdin_live()
                    || self.interrupt.is_some()
                    || crate::host::native_servers_live()
                {
                    wait = wait.min(2.0);
                }
                if self.slice_end.is_some() && crate::host::other_packages_have_work() {
                    wait = wait.min(2.0);
                }
                if wait > 0.0 {
                    loop_idle_sleep(Duration::from_secs_f64(wait / 1000.0));
                }
            } else if crate::child::active()
                || crate::zlib::active_for(self.realm_id)
                || crate::worker::active(self)
                || self.stdin_live()
                || self.interrupt.is_some()
                || crate::host::native_servers_live()
            {
                loop_idle_sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }

    /// Run the event loop until no referenced timer/immediate remains, emitting `beforeExit` and
    /// `exit`. Returns the process exit code; an uncaught exception comes back as `Err`.
    pub fn run_event_loop(&mut self) -> R<i32> {
        // an exited realm runs nothing more (a routed build calls the loop once per terminal pass)
        if self.rt.exit_emitted {
            return Ok(self.exit_code());
        }
        let mut code = None;
        loop {
            match self.loop_inner() {
                Ok(()) => {
                    if self.yielded {
                        return Ok(0); // slice ended: not idle, so no beforeExit/exit; the host re-enters and reads `yielded`
                    }
                }
                Err(Ctl::Exit(c)) => {
                    code = Some(c);
                    break;
                }
                Err(e) => return self.fatal(e),
            }
            let before = self.exit_code();
            match self.emit("beforeExit", &[Value::Number(before as f64)]) {
                Ok(_) => {}
                Err(Ctl::Exit(c)) => {
                    code = Some(c);
                    break;
                }
                Err(e) => return Err(e),
            }
            if self.rt.ticks.is_empty() && self.rt.micro.is_empty() && !self.loop_alive() {
                break;
            }
        }
        let code = code.unwrap_or_else(|| self.exit_code());
        self.set_exit_code(Some(code as f64));
        self.emit_exit_event(code)?;
        Ok(self.exit_code())
    }

    /// `run_event_loop` without `beforeExit`/`exit`: a routed build's pass runs the loop and leaves the exit
    /// events to its idle pass, which runs once no native watch or server is left. A fatal error still emits
    /// `exit` with code 1, as `run_event_loop` does.
    pub(crate) fn run_loop_only(&mut self) -> R<i32> {
        match self.loop_inner() {
            Ok(()) => Ok(0),
            Err(Ctl::Exit(c)) => {
                self.set_exit_code(Some(c as f64));
                Ok(c)
            }
            Err(e) => self.fatal(e),
        }
    }

    /// Emits `exit` with `code` unless this realm already did.
    fn emit_exit_event(&mut self, code: i32) -> R<()> {
        if std::mem::replace(&mut self.rt.exit_emitted, true) {
            return Ok(());
        }
        match self.emit("exit", &[Value::Number(code as f64)]) {
            Ok(_) | Err(Ctl::Exit(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Emits `exit` for a process ending with `code` from the host (`process.exit` in another realm, or a
    /// terminal exit). Listeners may still change `process.exitCode`; the host reads it after this.
    pub(crate) fn emit_exit_once(&mut self, code: i32) {
        let _ = self.emit_exit_event(code);
    }

    /// Mark a promise as observed by the embedder (no unhandled-rejection report).
    pub fn mark_handled(&self, v: &Value) {
        if let Value::Object(o) = v {
            if let Kind::Promise(d) = &mut *o.kind.borrow_mut() {
                d.handled = true;
            }
        }
    }

    pub fn is_promise(&self, v: &Value) -> bool {
        matches!(v, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Promise(_)))
    }

    /// `Some(Ok(value))` / `Some(Err(reason))` once settled, `None` while pending.
    pub fn promise_state(&self, v: &Value) -> Option<Result<Value, Value>> {
        let Value::Object(o) = v else { return None };
        let k = o.kind.borrow();
        let Kind::Promise(d) = &*k else { return None };
        match &d.state {
            PState::Pending => None,
            PState::Fulfilled(v) => Some(Ok(v.clone())),
            PState::Rejected(v) => Some(Err(v.clone())),
        }
    }

    fn exit_code(&self) -> i32 {
        self.rt.shared.exit_code.get().map_or(0, |n| n as i32)
    }

    /// Run a script as a program: evaluate it, then run the event loop. Returns the exit code.
    pub fn run_main(&mut self, source: &str) -> R<i32> {
        let ran = self.run(source);
        self.finish_main(ran)
    }

    /// Run a file as the main CommonJS module (`require`, relative paths, JSON, index files), then
    /// the event loop. Returns the exit code.
    pub fn run_main_file(&mut self, path: &str) -> R<i32> {
        let mut quoted = String::new();
        crate::builtins::json_quote(path, &mut quoted);
        let ran = self.run(&format!("globalThis.__thaw_run_main_file({quoted})"));
        self.finish_main(ran)
    }

    fn finish_main(&mut self, ran: R<Value>) -> R<i32> {
        match ran {
            Ok(_) => self.run_event_loop(),
            Err(Ctl::Exit(c)) => {
                self.set_exit_code(Some(c as f64));
                self.emit_exit_event(c)?;
                Ok(self.exit_code())
            }
            Err(e) => self.fatal(e),
        }
    }

    /// An uncaught exception: `exit` listeners still run, with code 1.
    fn fatal(&mut self, e: Ctl) -> R<i32> {
        self.set_exit_code(Some(1.0));
        let _ = self.emit_exit_event(1);
        Err(e)
    }

    /// Print console output as it happens (a `node`-like standalone run) instead of buffering it.
    pub fn set_stream(&mut self, on: bool) {
        self.rt.stream = on;
    }

    /// Flush a trailing partial `process.stdout.write` line.
    pub fn flush_partial(&mut self) {
        let rest = std::mem::take(&mut self.rt.partial);
        if rest.is_empty() {
            return;
        }
        if self.rt.stream {
            if !crate::worker::capture_line(self, 1, &rest) {
                print!("{rest}");
                // A partial line has no newline, so Rust's stdout buffer would keep it past the C `main` exit.
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
        } else {
            self.out.push(rest);
        }
    }

    // ----- process events -----

    /// Whether `process.on(name)` has a listener in this realm.
    pub(crate) fn has_process_listeners(&self, name: &str) -> bool {
        self.rt.listeners.get(name).is_some_and(|l| !l.is_empty())
    }

    pub(crate) fn emit(&mut self, name: &str, args: &[Value]) -> R<bool> {
        let list = self.rt.listeners.get(name).cloned().unwrap_or_default();
        let this = self
            .rt
            .process
            .clone()
            .map_or(Value::Undefined, Value::Object);
        for (f, once) in &list {
            if *once {
                if let Some(l) = self.rt.listeners.get_mut(name) {
                    l.retain(|(g, _)| !strict_equals(g, f));
                }
            }
            self.call(f, &this, args)?;
        }
        Ok(!list.is_empty())
    }

    // ----- installation -----

    pub(crate) fn install_eventloop(&mut self) {
        let g = self.global.clone();
        self.rt.now = clock_ms();
        self.rt.first_turn = true;
        let promise_proto = self.plain();
        self.rt.promise_proto = Some(promise_proto.clone());

        // ----- Promise -----
        let ctor = self.native(|it, _, args| {
            let exec = arg(args, 0);
            if !exec.is_callable() {
                let shown = it.inspect(&exec, false);
                return it.throw_error(
                    "TypeError",
                    &format!("Promise resolver {shown} is not a function"),
                );
            }
            let p = it.new_promise();
            let (res, rej) = it.resolvers(&p);
            match it.call(&exec, &Value::Undefined, &[res, rej.clone()]) {
                Err(Ctl::Throw(e)) => {
                    it.call(&rej, &Value::Undefined, &[e])?;
                }
                Err(e) => return Err(e),
                Ok(_) => {}
            }
            Ok(Value::Object(p))
        });
        if let Value::Object(c) = &ctor {
            put_hidden(c, "prototype", Value::Object(promise_proto.clone()));
            put_hidden(&promise_proto, "constructor", ctor.clone());
            put_hidden(c, "name", Value::str("Promise"));
            self.set_native(c, "resolve", |it, _, a| {
                Ok(Value::Object(it.promise_resolve_value(arg(a, 0))?))
            });
            self.set_native(c, "reject", |it, _, a| {
                let p = it.new_promise();
                it.reject_promise(&p, arg(a, 0));
                Ok(Value::Object(p))
            });
        }
        self.set_native(&promise_proto, "then", |it, this, a| {
            let Value::Object(p) = this else {
                return it.throw_error(
                    "TypeError",
                    "Method Promise.prototype.then called on incompatible receiver",
                );
            };
            if !matches!(&*p.kind.borrow(), Kind::Promise(_)) {
                return it.throw_error(
                    "TypeError",
                    "Method Promise.prototype.then called on incompatible receiver",
                );
            }
            let result = it.new_promise();
            let pick = |v: Value| if v.is_callable() { v } else { Value::Undefined };
            it.promise_then(p, pick(arg(a, 0)), pick(arg(a, 1)), Some(result.clone()));
            Ok(Value::Object(result))
        });
        g.declare("Promise", ctor, true);

        // ----- timers -----
        let timeout_proto = self.plain();
        let immediate_proto = self.plain();
        self.rt.timeout_proto = Some(timeout_proto.clone());
        self.rt.immediate_proto = Some(immediate_proto.clone());
        for (proto, class) in [(&timeout_proto, "Timeout"), (&immediate_proto, "Immediate")] {
            let ctor = self.native(|_, _, _| Ok(Value::Undefined));
            if let Value::Object(c) = &ctor {
                put_hidden(c, "prototype", Value::Object(proto.clone()));
                put_hidden(c, "name", Value::str(class));
            }
            put_hidden(proto, "constructor", ctor);
        }
        for proto in [&timeout_proto, &immediate_proto] {
            self.set_native(proto, "ref", |it, this, _| {
                it.set_ref(this, Some(true));
                Ok(this.clone())
            });
            self.set_native(proto, "unref", |it, this, _| {
                it.set_ref(this, Some(false));
                Ok(this.clone())
            });
            self.set_native(proto, "hasRef", |it, this, _| {
                Ok(Value::Bool(it.set_ref(this, None)))
            });
        }
        // Node: a Timeout/Interval converts to its numeric id (`+t`, `${t}`, `'a' + t`), and
        // `[Symbol.dispose]` clears it. Immediates have no toPrimitive; both classes get dispose.
        let to_prim = self.well_known_key("toPrimitive");
        let dispose = self.well_known_key("dispose");
        let tp = self.native(|it, this, _| {
            Ok(it
                .timer_id(this)
                .map_or(Value::Undefined, |id| Value::Number(id as f64)))
        });
        put_hidden(&timeout_proto, &to_prim, tp);
        for (proto, immediate) in [(&timeout_proto, false), (&immediate_proto, true)] {
            let d = self.native(move |it, this, _| {
                if let Some(id) = it.timer_id(this) {
                    it.clear_timer_anywhere(id, immediate);
                }
                Ok(Value::Undefined)
            });
            put_hidden(proto, &dispose, d);
        }
        self.set_native(&timeout_proto, "refresh", |it, this, _| {
            if let Some(id) = it.timer_id(this) {
                it.refresh_timer(id);
            }
            Ok(this.clone())
        });
        self.set_native(&timeout_proto, "close", |it, this, _| {
            if let Some(id) = it.timer_id(this) {
                it.clear_timer_anywhere(id, false);
            }
            Ok(this.clone())
        });
        for (name, repeat) in [("setTimeout", false), ("setInterval", true)] {
            let f = self.native(move |it, _, a| it.add_timer(a, repeat));
            g.declare(name, f, true);
        }
        for name in ["clearTimeout", "clearInterval"] {
            let f = self.native(|it, _, a| {
                if let Some(id) = it.timer_id(&arg(a, 0)) {
                    it.clear_timer_anywhere(id, false);
                }
                Ok(Value::Undefined)
            });
            g.declare(name, f, true);
        }
        let f = self.native(|it, _, a| {
            let cb = arg(a, 0);
            it.need_callback(&cb)?;
            let id = next_timer_id();
            let obj = new_obj(it.rt.immediate_proto.clone(), Kind::Plain);
            put_hidden(&obj, "@@tid", Value::Number(id as f64));
            it.rt.immediates.push(Imm {
                id,
                cb,
                args: a.get(1..).unwrap_or(&[]).to_vec(),
                obj: obj.clone(),
                refd: true,
                frame: it.rt.actx.clone(),
            });
            Ok(Value::Object(obj))
        });
        g.declare("setImmediate", f, true);
        let f = self.native(|it, _, a| {
            if let Some(id) = it.timer_id(&arg(a, 0)) {
                it.clear_timer_anywhere(id, true);
            }
            Ok(Value::Undefined)
        });
        g.declare("clearImmediate", f, true);
        let f = self.native(|it, _, a| {
            let cb = arg(a, 0);
            it.need_callback(&cb)?;
            it.rt
                .micro
                .push_back((it.rt.actx.clone(), Job::Call(cb, Vec::new())));
            Ok(Value::Undefined)
        });
        g.declare("queueMicrotask", f, true);

        let get = self.native(|it, _, _| Ok(it.rt.actx.clone().unwrap_or(Value::Undefined)));
        let swap = self.native(|it, _, a| {
            let next = match arg(a, 0) {
                Value::Undefined => None,
                v => Some(v),
            };
            Ok(std::mem::replace(&mut it.rt.actx, next).unwrap_or(Value::Undefined))
        });
        let bridge = new_obj(None, Kind::Plain);
        put(&bridge, "get", get);
        put(&bridge, "swap", swap);
        g.declare(
            "__thaw_async_context_bootstrap",
            Value::Object(bridge),
            true,
        );

        self.install_process();
    }

    /// `process.stdin` (web/process_stdin.js). Needs `__thaw_process_is_tty` from `install_sysio`.
    pub(crate) fn install_process_stdin(&mut self) {
        self.run(PROCESS_STDIN)
            .unwrap_or_else(|_| panic!("thaw-js process.stdin must evaluate"));
    }

    /// Promise combinators in JS; needs `Error`, so it runs after the main prelude.
    pub(crate) fn run_promise_prelude(&mut self) {
        self.run(PROMISE_PRELUDE)
            .unwrap_or_else(|_| panic!("thaw-js promise prelude must evaluate"));
    }

    /// Defines `__thaw_run_main_file` (the disk CommonJS loader) and its file-read host primitive.
    pub(crate) fn install_main_file_loader(&mut self) {
        let read = self.native(|it, _, a| {
            let path = it.to_str(&arg(a, 0)).to_string();
            match std::fs::read_to_string(&path) {
                Ok(source) => Ok(Value::str(&source)),
                Err(e) => it.throw_error("Error", &format!("{e}: {path}")),
            }
        });
        self.global.declare("__thaw_worker_read_source", read, true);
        self.run(MAIN_FILE_LOADER)
            .unwrap_or_else(|_| panic!("thaw-js main file loader must evaluate"));
    }

    fn install_process(&mut self) {
        let g = self.global.clone();
        let p = self.plain();
        self.rt.process = Some(p.clone());
        put(&p, "title", Value::str("node"));
        put(&p, "version", Value::str("v22.23.1"));
        put(&p, "platform", Value::str("linux"));
        put(&p, "arch", Value::str("x64"));
        // node-gyp / bindings read `process.config.variables` and `target_defaults` (empty, as a build-free runtime)
        let config = self.plain();
        put(&config, "variables", Value::Object(self.plain()));
        put(&config, "target_defaults", Value::Object(self.plain()));
        put(&p, "config", Value::Object(config));
        put(&p, "pid", Value::Number(std::process::id() as f64));
        // The compiled binary re-runs itself as `node` (thaw_tjs_run_cli handles `-e`).
        let arguments: Vec<String> = std::env::args().collect();
        let exec_path = std::env::current_exe()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| arguments.first().cloned().unwrap_or_default());
        put(&p, "execPath", Value::str(&exec_path));
        // `exitCode` lives in the realm-shared state, so every package's `process` agrees on it.
        let exit_code_get = self.native(|it, _, _| {
            Ok(match it.rt.shared.exit_code.get() {
                Some(n) => Value::Number(n),
                None => Value::Undefined,
            })
        });
        let exit_code_set = self.native(|it, _, a| {
            let code = match arg(a, 0) {
                Value::Undefined | Value::Null => None,
                v => Some(it.num(&v)),
            };
            it.set_exit_code(code);
            Ok(Value::Undefined)
        });
        define_accessor(
            &p,
            "exitCode",
            Some(exit_code_get),
            Some(exit_code_set),
            true,
        );
        let versions = self.plain();
        put(&versions, "node", Value::str("22.23.1"));
        put(&versions, "v8", Value::str("12.4.254.21-node.33"));
        put(&p, "versions", Value::Object(versions));
        let release = self.plain();
        put(&release, "name", Value::str("node"));
        put(&p, "release", Value::Object(release));
        // Mirrors thaw-quickjs context.rs: argv[0] is the host binary, then the forwarded arguments.
        let (node_argv, node_exec_argv) = node_argv_split(&arguments);
        let argv = self.array(node_argv.iter().map(|a| Value::str(a)).collect());
        put(&p, "argv", argv);
        let exec_argv = self.array(node_exec_argv.iter().map(|a| Value::str(a)).collect());
        put(&p, "execArgv", exec_argv);
        put(&p, "env", self.make_process_env());
        self.set_native(&p, "cwd", |_, _, _| {
            Ok(Value::str(
                &std::env::current_dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|_| "/".into()),
            ))
        });
        self.set_native(&p, "chdir", |it, _, a| {
            let dest = it.to_str(&arg(a, 0)).to_string();
            let cwd = std::env::current_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|_| "/".into());
            if std::env::set_current_dir(&dest).is_err() {
                let e = it.make_error(
                    "Error",
                    &format!("ENOENT: no such file or directory, chdir '{cwd}' -> '{dest}'"),
                )?;
                if let Value::Object(o) = &e {
                    put(o, "code", Value::str("ENOENT"));
                }
                return Err(Ctl::Throw(e));
            }
            Ok(Value::Undefined)
        });
        self.set_native(&p, "binding", |it, _, a| {
            let name = it.to_str(&arg(a, 0));
            it.throw_error("Error", &format!("No such module: {name}"))
        });
        let perf_now = self.native(|_, _, _| Ok(Value::Number(clock_ms())));
        g.declare("__thaw_perf_now", perf_now, true);
        let loop_stats = self.native(|it, _, _| {
            let (active, idle) = host_loop_stats();
            Ok(it.array(vec![Value::Number(active), Value::Number(idle)]))
        });
        g.declare("__thaw_loop_stats", loop_stats, true);
        // Atomics.wait's only blocking primitive (extras.rs); ms <= 0 or non-finite returns at once.
        let sleep_ms = self.native(|it, _, a| {
            let ms = it.num(&arg(a, 0));
            if ms.is_finite() && ms > 0.0 {
                std::thread::sleep(std::time::Duration::from_secs_f64(ms / 1000.0));
            }
            Ok(Value::Undefined)
        });
        g.declare("__thaw_sleep_ms", sleep_ms, true);
        self.set_native(&p, "uptime", |_, _, _| {
            Ok(Value::Number(clock_ms() / 1000.0))
        });
        let hrtime = self.native(|it, _, a| {
            let ns = hrtime_ns();
            let (mut s, mut n) = ((ns / 1_000_000_000) as f64, (ns % 1_000_000_000) as f64);
            if let Value::Object(_) = arg(a, 0) {
                let prev = it.array_items(&arg(a, 0));
                if prev.len() == 2 {
                    s -= it.num(&prev[0]);
                    n -= it.num(&prev[1]);
                    if n < 0.0 {
                        s -= 1.0;
                        n += 1e9;
                    }
                }
            }
            Ok(it.array(vec![Value::Number(s), Value::Number(n)]))
        });
        if let Value::Object(h) = &hrtime {
            self.set_native(h, "bigint", |_, _, _| {
                Ok(crate::bigint::big(num_bigint::BigInt::from(hrtime_ns())))
            });
        }
        put_hidden(&p, "hrtime", hrtime);
        let memory_usage = self.native(|it, _, _| {
            let o = it.plain();
            for k in ["rss", "heapTotal", "heapUsed", "external", "arrayBuffers"] {
                put(&o, k, Value::Number(0.0));
            }
            Ok(Value::Object(o))
        });
        if let Value::Object(f) = &memory_usage {
            self.set_native(f, "rss", |_, _, _| {
                Ok(Value::Number(process_rss_bytes() as f64))
            });
        }
        put_hidden(&p, "memoryUsage", memory_usage);
        self.set_native(&p, "getuid", |_, _, _| {
            Ok(Value::Number(unsafe { libc::getuid() } as f64))
        });
        self.set_native(&p, "geteuid", |_, _, _| {
            Ok(Value::Number(unsafe { libc::geteuid() } as f64))
        });
        self.set_native(&p, "getgid", |_, _, _| {
            Ok(Value::Number(unsafe { libc::getgid() } as f64))
        });
        self.set_native(&p, "getegid", |_, _, _| {
            Ok(Value::Number(unsafe { libc::getegid() } as f64))
        });
        self.set_native(&p, "umask", |it, _, a| {
            let mask = match arg(a, 0) {
                Value::Undefined => None,
                Value::Number(n) => Some(n as libc::mode_t),
                v => {
                    let text = it.to_str(&v).to_string();
                    match u32::from_str_radix(text.trim(), 8) {
                        Ok(m) => Some(m as libc::mode_t),
                        Err(_) => {
                            return Err(node_error(
                                it,
                                "TypeError",
                                "ERR_INVALID_ARG_VALUE",
                                &format!(
                                "The argument 'mask' must be an octal string. Received '{text}'"
                            ),
                            ))
                        }
                    }
                }
            };
            // Setting a mask returns the previous one; a read sets 0 and restores it.
            let old = match mask {
                Some(m) => unsafe { libc::umask(m) },
                None => {
                    let cur = unsafe { libc::umask(0) };
                    unsafe { libc::umask(cur) };
                    cur
                }
            };
            Ok(Value::Number(old as f64))
        });
        self.set_native(&p, "kill", |it, _, a| {
            let pid = match arg(a, 0) {
                Value::Number(n) if n.fract() == 0.0 => n as i32,
                _ => {
                    return Err(node_error(
                        it,
                        "TypeError",
                        "ERR_INVALID_ARG_TYPE",
                        "The \"pid\" argument must be of type number.",
                    ))
                }
            };
            let sig = arg(a, 1);
            let number: i32 = match &sig {
                Value::Undefined => libc::SIGTERM,
                Value::Number(n) => *n as i32,
                v => {
                    let text = it.to_str(v).to_string();
                    match PROCESS_SIGNALS.iter().find(|(name, _)| *name == text) {
                        Some((_, n)) => *n,
                        None => {
                            return Err(node_error(
                                it,
                                "TypeError",
                                "ERR_UNKNOWN_SIGNAL",
                                &format!("Unknown signal: {text}"),
                            ))
                        }
                    }
                }
            };
            let name = PROCESS_SIGNALS
                .iter()
                .find(|(_, n)| *n == number)
                .map(|(name, _)| *name);
            // A signal to this process with a listener is delivered as the listener's event, on the tick queue.
            if let Some(name) = name.filter(|name| {
                number != 0 && pid as u32 == std::process::id() && it.has_process_listeners(name)
            }) {
                let event = name.to_string();
                let deliver = it.native(move |it, _, _| {
                    it.emit(&event, &[Value::str(&event)])?;
                    Ok(Value::Undefined)
                });
                it.rt.ticks.push_back((it.rt.actx.clone(), deliver, vec![]));
                return Ok(Value::Bool(true));
            }
            if unsafe { libc::kill(pid, number) } != 0 {
                let raw = std::io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or(libc::EINVAL);
                let errno = if raw == libc::ESRCH || raw == libc::EPERM {
                    raw
                } else {
                    libc::EINVAL
                };
                let code = if errno == libc::ESRCH {
                    "ESRCH"
                } else if errno == libc::EPERM {
                    "EPERM"
                } else {
                    "EINVAL"
                };
                let e = it.make_error("Error", &format!("kill {code}"))?;
                if let Value::Object(o) = &e {
                    put(o, "errno", Value::Number(-(errno as f64)));
                    put(o, "code", Value::str(code));
                    put(o, "syscall", Value::str("kill"));
                }
                return Err(Ctl::Throw(e));
            }
            Ok(Value::Bool(true))
        });
        self.set_native(&p, "nextTick", |it, _, a| {
            let cb = arg(a, 0);
            it.need_callback(&cb)?;
            it.rt
                .ticks
                .push_back((it.rt.actx.clone(), cb, a.get(1..).unwrap_or(&[]).to_vec()));
            Ok(Value::Undefined)
        });
        self.set_native(&p, "exit", |it, _, a| {
            let code = match arg(a, 0) {
                Value::Undefined => it.exit_code(),
                v => it.num(&v) as i32,
            };
            // The host ends the process at its boundary (host.rs), so every realm's exit listeners run.
            it.set_exit_code(Some(code as f64));
            it.rt.shared.exit_requested.set(true);
            Err(Ctl::Exit(code))
        });
        self.set_native(&p, "emitWarning", |it, _, a| {
            let first = arg(a, 0);
            // `(warning, options)` or `(warning, type, code)`
            let (mut kind, mut code) = (arg(a, 1), arg(a, 2));
            let opts = arg(a, 1);
            if matches!(opts, Value::Object(_)) && !opts.is_callable() {
                kind = it.get(&opts, "type")?;
                code = it.get(&opts, "code")?;
            }
            let warning = match &first {
                Value::Object(o) if crate::builtins::is_error_instance(it, o) => first.clone(),
                _ => {
                    let msg = it.to_str(&first);
                    let e = it.make_error("Error", &msg)?;
                    if let Value::Object(o) = &e {
                        put(o, "name", Value::str("Warning"));
                    }
                    e
                }
            };
            if let Value::Object(o) = &warning {
                if !matches!(kind, Value::Undefined) {
                    let t = it.to_str(&kind);
                    put(o, "name", Value::str(&t));
                }
                if !matches!(code, Value::Undefined) {
                    put(o, "code", code.clone());
                }
            }
            // Node emits 'warning' asynchronously; stderr is only the fallback when nobody listens.
            let cb = it.native(move |it, _, _| {
                if !it.emit("warning", std::slice::from_ref(&warning))? {
                    let name = it.get(&warning, "name")?;
                    let name = it.to_str(&name);
                    let msg = it.get(&warning, "message")?;
                    let msg = it.to_str(&msg);
                    let c = it.get(&warning, "code")?;
                    let prefix = if matches!(c, Value::Undefined) {
                        String::new()
                    } else {
                        format!("[{}] ", it.to_str(&c))
                    };
                    let pid = std::process::id();
                    it.emit_err(&format!("(node:{pid}) {prefix}{name}: {msg}"));
                }
                Ok(Value::Undefined)
            });
            it.rt.ticks.push_back((it.rt.actx.clone(), cb, vec![]));
            Ok(Value::Undefined)
        });
        for (name, once) in [("on", false), ("addListener", false), ("once", true)] {
            self.set_native(&p, name, move |it, this, a| {
                let ev = it.to_str(&arg(a, 0)).to_string();
                it.rt
                    .listeners
                    .entry(ev)
                    .or_default()
                    .push((arg(a, 1), once));
                Ok(this.clone())
            });
        }
        for name in ["off", "removeListener"] {
            self.set_native(&p, name, |it, this, a| {
                let ev = it.to_str(&arg(a, 0)).to_string();
                if let Some(l) = it.rt.listeners.get_mut(&ev) {
                    l.retain(|(f, _)| !strict_equals(f, &arg(a, 1)));
                }
                Ok(this.clone())
            });
        }
        self.set_native(&p, "removeAllListeners", |it, this, a| {
            match arg(a, 0) {
                Value::Undefined => it.rt.listeners.clear(),
                v => {
                    let ev = it.to_str(&v).to_string();
                    it.rt.listeners.remove(&ev);
                }
            }
            Ok(this.clone())
        });
        self.set_native(&p, "emit", |it, _, a| {
            let ev = it.to_str(&arg(a, 0)).to_string();
            Ok(Value::Bool(it.emit(&ev, a.get(1..).unwrap_or(&[]))?))
        });
        self.set_native(&p, "listenerCount", |it, _, a| {
            let ev = it.to_str(&arg(a, 0)).to_string();
            Ok(Value::Number(
                it.rt.listeners.get(&ev).map_or(0, Vec::len) as f64
            ))
        });
        for (name, is_err) in [("stdout", false), ("stderr", true)] {
            let s = self.plain();
            self.set_native(&s, "write", move |it, _, a| {
                // a typed array or ArrayBuffer carries bytes; anything else is text
                let chunk = arg(a, 0);
                let raw = Interp::view_bytes(&chunk).or_else(|| buffer_json_bytes(it, &chunk));
                let text = match &raw {
                    Some(b) => String::from_utf8_lossy(b).into_owned(),
                    None => it.to_str(&chunk).to_string(),
                };
                // a main-thread stream writes its bytes as they come; a worker's output is captured by its parent
                let direct = it.rt.stream && !crate::worker::is_worker(it);
                if is_err {
                    if direct {
                        it.emit_err_raw(raw.as_deref().unwrap_or(text.as_bytes()));
                    } else {
                        it.emit_err(text.trim_end_matches('\n'));
                    }
                } else if let (true, Some(b)) = (direct, raw.as_deref()) {
                    it.emit_out_raw(b);
                } else {
                    it.emit_out(&text, false);
                    if direct {
                        it.flush_partial();
                    }
                }
                // Node: write(chunk[, encoding][, callback]) calls the trailing callback on a later tick
                if let Some(cb) = a
                    .get(1..)
                    .unwrap_or(&[])
                    .iter()
                    .rev()
                    .find(|v| v.is_callable())
                {
                    it.rt
                        .ticks
                        .push_back((it.rt.actx.clone(), cb.clone(), vec![]));
                }
                Ok(Value::Bool(true))
            });
            // Node leaves isTTY undefined on a pipe or file
            if crate::sysio::fd_is_tty(if is_err { 2 } else { 1 }) {
                put(&s, "isTTY", Value::Bool(true));
            }
            put(&p, name, Value::Object(s));
        }
        g.declare("process", Value::Object(p), true);
    }
}

/// `{"type":"Buffer","data":[0..255,...]}`: a Buffer inside a compiled `any`/Json comes back across the
/// boundary in Node's `Buffer.prototype.toJSON` shape; write it as its bytes. Strict: exactly those two
/// own keys, `data` an array whose every item is an integer 0..=255 (no getters or coercion run).
fn buffer_json_bytes(it: &mut Interp, v: &Value) -> Option<Vec<u8>> {
    let Value::Object(o) = v else { return None };
    if v.is_callable() || !matches!(&*o.kind.borrow(), Kind::Plain) {
        return None;
    }
    let keys = it.own_keys(v);
    if keys.len() != 2
        || !keys.iter().any(|k| &**k == "type")
        || !keys.iter().any(|k| &**k == "data")
    {
        return None;
    }
    let kind = it.get(v, "type").ok()?;
    if !matches!(kind, Value::Str(_)) || &*it.to_str(&kind) != "Buffer" {
        return None;
    }
    let data = it.get(v, "data").ok()?;
    let Value::Object(d) = &data else { return None };
    if !matches!(&*d.kind.borrow(), Kind::Array(_)) {
        return None;
    }
    it.array_items_raw(&data)
        .iter()
        .map(|item| match item {
            Value::Number(n) if n.fract() == 0.0 && (0.0..=255.0).contains(n) => Some(*n as u8),
            _ => None,
        })
        .collect()
}

/// How Node's `ERR_INVALID_ARG_TYPE` message describes a bad argument.
fn received(v: &Value, shown: &str) -> String {
    match v {
        Value::Undefined | Value::Hole => "undefined".into(),
        Value::Null => "null".into(),
        Value::Bool(b) => format!("type boolean ({b})"),
        Value::Number(_) => format!("type number ({shown})"),
        Value::Str(s) => format!("type string ('{s}')"),
        Value::BigInt(b) => format!("type bigint ({b}n)"),
        Value::Object(_) => "an instance of Object".into(),
    }
}

/// Promise combinators written in JS on top of the native `then`/`resolve`: same job counts as the spec.
const PROMISE_PRELUDE: &str = r#"
(function () {
  function def(o, name, f) {
    Object.defineProperty(o, name, { value: f, writable: true, enumerable: false, configurable: true });
  }
  var P = Promise;
  var AE = function (errors, message) {
    if (!(this instanceof AE)) return new AE(errors, message);
    if (message !== undefined) def(this, 'message', String(message));
    def(this, 'errors', errors);
    def(this, 'stack', 'AggregateError' + (message !== undefined ? ': ' + String(message) : ''));
  };
  AE.prototype = Object.create(Error.prototype);
  def(AE.prototype, 'constructor', AE);
  def(AE.prototype, 'name', 'AggregateError');
  def(AE.prototype, 'message', '');
  AggregateError = AE;
  def(P.prototype, 'catch', function (r) { return this.then(undefined, r); });
  def(P.prototype, 'finally', function (f) {
    if (typeof f !== 'function') return this.then(f, f);
    return this.then(
      function (v) { return P.resolve(f()).then(function () { return v; }); },
      function (e) { return P.resolve(f()).then(function () { throw e; }); });
  });
  function items(it) {
    var out = [];
    for (var x of it) out.push(x);
    return out;
  }
  def(P, 'all', function (it) {
    return new P(function (res, rej) {
      var xs = items(it), n = xs.length, out = [], left = n;
      if (n === 0) { res(out); return; }
      xs.forEach(function (x, i) {
        out.push(undefined);
        P.resolve(x).then(function (v) { out[i] = v; if (--left === 0) res(out); }, rej);
      });
    });
  });
  def(P, 'allSettled', function (it) {
    return new P(function (res) {
      var xs = items(it), n = xs.length, out = [], left = n;
      if (n === 0) { res(out); return; }
      xs.forEach(function (x, i) {
        out.push(undefined);
        P.resolve(x).then(
          function (v) { out[i] = { status: 'fulfilled', value: v }; if (--left === 0) res(out); },
          function (e) { out[i] = { status: 'rejected', reason: e }; if (--left === 0) res(out); });
      });
    });
  });
  def(P, 'race', function (it) {
    return new P(function (res, rej) {
      items(it).forEach(function (x) { P.resolve(x).then(res, rej); });
    });
  });
  def(P, 'any', function (it) {
    return new P(function (res, rej) {
      var xs = items(it), n = xs.length, errs = [], left = n;
      if (n === 0) { rej(new AE([], 'All promises were rejected')); return; }
      xs.forEach(function (x, i) {
        errs.push(undefined);
        P.resolve(x).then(res, function (e) {
          errs[i] = e;
          if (--left === 0) rej(new AE(errs, 'All promises were rejected'));
        });
      });
    });
  });
  def(P, 'withResolvers', function () {
    var r = {};
    r.promise = new P(function (a, b) { r.resolve = a; r.reject = b; });
    return r;
  });
})();
"#;

/// Index of the `-e` flag among leading flags, the same rule as thaw-quickjs `cli_eval`.
pub(crate) fn cli_eval_index(arguments: &[String]) -> Option<usize> {
    arguments
        .iter()
        .enumerate()
        .skip(1)
        .take_while(|(_, argument)| argument.starts_with('-'))
        .find_map(|(index, argument)| (argument == "-e").then_some(index))
}

/// The script a `node [flags] file.js [args]` start runs: the first non-flag argument, when it is a
/// `.js`/`.cjs` file that exists (the same rule as thaw-quickjs `cli_script`).
pub(crate) fn cli_script_index(arguments: &[String]) -> Option<usize> {
    let index = arguments
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, argument)| (!argument.starts_with('-')).then_some(index))?;
    let script = arguments.get(index)?;
    let path = std::path::Path::new(script);
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("js" | "cjs")
    )
    .then_some(index)
    .filter(|_| path.is_file())
}

/// `(process.argv, process.execArgv)` for the process arguments, matching thaw-quickjs context.rs:
/// `-e SRC` keeps `-e SRC` in execArgv and drops it from argv; a script keeps its path first in argv
/// and the flags before it in execArgv; otherwise argv is every argument.
pub(crate) fn node_argv_split(arguments: &[String]) -> (Vec<String>, Vec<String>) {
    match cli_eval_index(arguments) {
        Some(index) => {
            let mut argv = vec![arguments.first().cloned().unwrap_or_default()];
            argv.extend(arguments.iter().skip(index + 2).cloned());
            (argv, arguments[1..arguments.len().min(index + 2)].to_vec())
        }
        None => match cli_script_index(arguments) {
            Some(index) => (
                [&arguments[..1], &arguments[index..]].concat(),
                arguments[1..index].to_vec(),
            ),
            // Node's `[node, script, ...args]` shape: the executable stands for the script slot.
            None => {
                let mut argv = arguments.to_vec();
                if let Some(exe) = arguments.first().cloned() {
                    argv.insert(0, exe);
                }
                (argv, Vec::new())
            }
        },
    }
}

#[cfg(test)]
mod cli_argv_tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn cli_eval_and_script_split_matches_quickjs_rules() {
        let (argv, exec) = node_argv_split(&args(&["app", "-e", "src", "tail"]));
        assert_eq!(argv, args(&["app", "tail"]));
        assert_eq!(exec, args(&["-e", "src"]));
        let (argv, exec) = node_argv_split(&args(&["app", "--x", "-e", "src"]));
        assert_eq!(argv, args(&["app"]));
        assert_eq!(exec, args(&["--x", "-e", "src"]));
        // A script (no -e) keeps the plain argument list, as before.
        let (argv, exec) = node_argv_split(&args(&["app", "a", "b"]));
        assert_eq!(argv, args(&["app", "app", "a", "b"]));
        assert!(exec.is_empty());
        // `-e` after a positional argument is not a flag, so it is not an eval.
        let (argv, _) = node_argv_split(&args(&["app", "a", "-e", "src"]));
        assert_eq!(argv, args(&["app", "app", "a", "-e", "src"]));
    }

    #[test]
    fn script_split_needs_an_existing_js_file_and_keeps_its_flags_in_exec_argv() {
        let dir =
            std::env::temp_dir().join(format!("thaw-cli-script-split-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("app.js");
        std::fs::write(&script, "").unwrap();
        let script = script.display().to_string();
        let (argv, exec) = node_argv_split(&args(&["app", "--no-warnings", &script, "tail"]));
        assert_eq!(argv, args(&["app", &script, "tail"]));
        assert_eq!(exec, args(&["--no-warnings"]));
        // A missing file or a non-JS extension is not a script start.
        let (argv, _) = node_argv_split(&args(&["app", "missing.js", "x"]));
        assert_eq!(argv, args(&["app", "app", "missing.js", "x"]));
        let (argv, _) = node_argv_split(&args(&["app", "notes.txt"]));
        assert_eq!(argv, args(&["app", "app", "notes.txt"]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_eval_source_returns_the_exit_status() {
        assert_eq!(crate::host::run_eval_source("1 + 1"), 0);
        assert_eq!(crate::host::run_eval_source("process.exitCode = 3"), 3);
        assert_eq!(crate::host::run_eval_source("throw new Error('boom')"), 1);
    }

    #[test]
    fn process_exec_path_is_the_current_binary_not_a_hard_coded_node() {
        let mut it = Interp::new();
        it.run("console.log(process.execPath)").unwrap();
        let expected = std::env::current_exe().unwrap().display().to_string();
        assert_eq!(it.out.join("").trim(), expected);
    }
}

#[cfg(test)]
mod process_state_tests {
    use super::*;

    fn output(it: &mut Interp, src: &str) -> String {
        it.out.clear();
        it.run(src)
            .unwrap_or_else(|_| panic!("script must run: {src}"));
        it.out.join("|")
    }

    #[test]
    fn exit_code_is_an_accessor_over_shared_state() {
        let mut it = Interp::new();
        assert_eq!(
            output(&mut it, "console.log(process.exitCode)"),
            "undefined"
        );
        assert_eq!(
            output(
                &mut it,
                "process.exitCode = 3; console.log(process.exitCode, typeof process.exitCode)"
            ),
            "3 number"
        );
        assert_eq!(it.rt.shared.exit_code.get(), Some(3.0));
    }

    #[test]
    fn two_interps_sharing_state_see_one_exit_code() {
        let shared = crate::procstate::host_shared();
        let mut a = Interp::new();
        let mut b = Interp::new();
        a.share_process_state(shared.clone());
        b.share_process_state(shared);
        a.run("process.exitCode = 6").unwrap();
        assert_eq!(output(&mut b, "console.log(process.exitCode)"), "6");
    }

    #[test]
    fn process_exit_with_no_argument_uses_the_shared_exit_code() {
        let mut it = Interp::new();
        assert_eq!(
            it.run_main("process.exitCode = 9; process.exit()").ok(),
            Some(9)
        );
    }

    #[test]
    fn plain_argv_duplicates_the_executable() {
        let words: Vec<String> = ["app", "a"].iter().map(|w| w.to_string()).collect();
        let (argv, exec) = node_argv_split(&words);
        assert_eq!(argv, ["app", "app", "a"]);
        assert!(exec.is_empty());
    }

    #[test]
    fn stdout_istty_is_absent_off_a_tty() {
        let mut it = Interp::new();
        let expected = if crate::sysio::fd_is_tty(1) {
            "boolean"
        } else {
            "undefined"
        };
        assert_eq!(
            output(&mut it, "console.log(typeof process.stdout.isTTY)"),
            expected
        );
    }

    fn main_output(src: &str) -> String {
        let mut it = Interp::new();
        it.run_main(src)
            .unwrap_or_else(|_| panic!("program must run: {src}"));
        it.out.join("|")
    }

    #[test]
    fn process_ids_umask_and_rss_are_numbers() {
        assert_eq!(
            main_output(
                "console.log(typeof process.getuid(), typeof process.geteuid(), typeof process.getgid(), typeof process.getegid(), typeof process.umask(), typeof process.memoryUsage.rss())"
            ),
            "number number number number number number"
        );
    }

    #[test]
    fn process_kill_own_pid_with_listener_is_delivered_on_the_tick_queue() {
        assert_eq!(
            main_output(
                "process.on('SIGUSR2', (s) => console.log('got', s)); console.log(process.kill(process.pid, 'SIGUSR2')); console.log('sync');"
            ),
            "true|sync|got SIGUSR2"
        );
    }

    #[test]
    fn process_kill_signal_zero_and_bad_arguments_match_node_errors() {
        assert_eq!(
            main_output("console.log(process.kill(process.pid, 0));"),
            "true"
        );
        assert_eq!(
            main_output(
                "try { process.kill(process.pid, 'SIGFOO'); } catch (e) { console.log(e.code); }"
            ),
            "ERR_UNKNOWN_SIGNAL"
        );
        assert_eq!(
            main_output("try { process.kill('1'); } catch (e) { console.log(e.code); }"),
            "ERR_INVALID_ARG_TYPE"
        );
        assert_eq!(
            main_output("try { process.kill(2147483646, 0); } catch (e) { console.log(e.code, e.syscall, e.errno); }"),
            "ESRCH kill -3"
        );
    }

    #[test]
    fn process_env_assignment_reads_back() {
        assert_eq!(
            main_output("process.env.THAW_W4_PROBE = 'hello'; console.log(process.env.THAW_W4_PROBE, typeof process.env.THAW_W4_PROBE); delete process.env.THAW_W4_PROBE; console.log(process.env.THAW_W4_PROBE === undefined);"),
            "hello string|true"
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn stdin_object_exists_with_isTTY_undefined_off_a_tty() {
        let mut it = Interp::new();
        let expected = if crate::sysio::fd_is_tty(0) {
            "boolean"
        } else {
            "undefined"
        };
        assert_eq!(
            output(
                &mut it,
                "console.log(typeof process.stdin.on, typeof process.stdin.isTTY)"
            ),
            format!("function {expected}")
        );
    }
}
