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
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

const TIMEOUT_MAX: f64 = 2147483647.0;
const CO_STACK: usize = 64 << 20;
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
    pub(crate) typed_base_proto: Option<Obj>,
    /// Prototype per `Ty`, in `Ty as usize` order.
    pub(crate) typed_protos: Vec<Obj>,
    /// Set by `construct` for the duration of entering a native constructor (`new Date()` vs `Date()`).
    pub(crate) constructing: bool,
    pub(crate) to_prim_key: Option<Rc<str>>,
    /// Every symbol by id, so `Object.getOwnPropertySymbols` can map `@@sym:<id>` keys back.
    pub(crate) symbols: HashMap<u64, Obj>,
    next_id: u64,
    seq: u64,
    now: f64,
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
}

fn clock_ms() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
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
                println!("{line}");
            } else {
                self.out.push(line.to_string());
            }
        }
        self.rt.partial = s;
    }

    pub(crate) fn emit_err(&mut self, line: &str) {
        if self.rt.stream {
            eprintln!("{line}");
        } else {
            self.err.push(line.to_string());
        }
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
            Ctl::Throw(v)
                if self
                    .rt
                    .listeners
                    .get("uncaughtException")
                    .is_some_and(|l| !l.is_empty()) =>
            {
                self.emit("uncaughtException", &[v, Value::str("uncaughtException")])?;
                Ok(())
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
                while let Some((fr, f, a)) = self.rt.ticks.pop_front() {
                    if let Err(e) = self.in_frame(fr, |it| it.call(&f, &Value::Undefined, &a)) {
                        self.uncaught(e)?;
                    }
                }
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
        let spawned = std::thread::Builder::new()
            .stack_size(CO_STACK)
            .spawn(move || {
                let r = {
                    let (ip, f, def, scope, this, args) = payload.into_inner();
                    // SAFETY: the host thread is blocked in `co_wait` until we send a message.
                    let it = unsafe { &mut *ip };
                    it.configure_stack(CO_STACK);
                    it.call_closure_sync(&f, &def, &scope, &this, &args)
                    // every Rc captured above is dropped here, before the hand-back
                };
                let _ = done_tx.send(SendBox(Yield::Done(r)));
            });
        if spawned.is_err() {
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
        let _ = co.to_co.send(SendBox(resume));
        self.co_wait(co, snap)
    }

    /// `await v` inside a coroutine.
    pub(crate) fn await_value(&mut self, v: Value) -> R<Value> {
        let Some(ctx) = self.rt.cur_co.last().cloned() else {
            return Err(Ctl::Unsupported("await".into()));
        };
        let snap = self.snapshot();
        let _ = ctx.to_host.send(SendBox(Yield::Await(v)));
        // the host runs from here until it resumes us
        let msg = ctx.from_host.recv();
        self.restore(snap);
        match msg.map(SendBox::into_inner) {
            Ok(Resume::Value(v)) => Ok(v),
            Ok(Resume::Throw(e)) => Err(Ctl::Throw(e)),
            Ok(Resume::Return(v)) => Err(Ctl::Return(v)),
            Err(_) => loop {
                // the host dropped this coroutine (promise never settled): never touch shared state again
                std::thread::park();
            },
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
        self.rt.next_id += 1;
        self.rt.seq += 1;
        let id = self.rt.next_id;
        let obj = new_obj(self.rt.timeout_proto.clone(), Kind::Plain);
        put_hidden(&obj, "@@tid", Value::Number(id as f64));
        self.rt.timers.push(Timer {
            id,
            expiry: self.rt.now + delay,
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

    fn set_ref(&mut self, this: &Value, refd: Option<bool>) -> bool {
        let Some(id) = self.timer_id(this) else {
            return false;
        };
        if let Some(t) = self.rt.timers.iter_mut().find(|t| t.id == id) {
            if let Some(r) = refd {
                t.refd = r;
            }
            return t.refd;
        }
        if let Some(t) = self.rt.immediates.iter_mut().find(|t| t.id == id) {
            if let Some(r) = refd {
                t.refd = r;
            }
            return t.refd;
        }
        false
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
            if let Err(e) = self.in_frame(frame, |it| it.call(&cb, &Value::Object(obj), &args)) {
                self.uncaught(e)?;
            }
            if repeat {
                self.rt.seq += 1;
                let (now, seq) = (self.rt.now, self.rt.seq);
                if let Some(t) = self.rt.timers.iter_mut().find(|t| t.id == id) {
                    t.expiry = now + delay;
                    t.seq = seq;
                }
            }
            self.run_jobs()?;
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
        loop {
            self.run_jobs()?;
            if let Some(state) = self.promise_state(v) {
                return Ok(Some(state));
            }
            if !self.loop_alive() {
                return Ok(None);
            }
            self.rt.now = clock_ms();
            self.run_timers()?;
            if !self.rt.immediates.is_empty() {
                self.run_immediates()?;
                continue;
            }
            if let Some(next) = self
                .rt
                .timers
                .iter()
                .map(|t| t.expiry)
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            {
                let wait = next - clock_ms();
                if wait > 0.0 {
                    std::thread::sleep(Duration::from_secs_f64(wait / 1000.0));
                }
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
            || crate::zlib::active()
    }

    /// Call the shim-installed `globalThis.__thaw_poll_platform_events` (async zlib jobs, child process output and exits).
    fn poll_platform_events(&mut self) -> R<()> {
        let g = Value::Object(self.global_object.clone());
        let f = self.get(&g, "__thaw_poll_platform_events")?;
        if matches!(f, Value::Object(_)) {
            self.call(&f, &g, &[])?;
            self.run_jobs()?;
        }
        Ok(())
    }

    fn loop_inner(&mut self) -> R<()> {
        self.run_jobs()?;
        while self.loop_alive() {
            self.rt.now = clock_ms();
            self.poll_platform_events()?;
            self.run_timers()?;
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
                if crate::child::active() || crate::zlib::active() {
                    wait = wait.min(2.0);
                }
                if wait > 0.0 {
                    std::thread::sleep(Duration::from_secs_f64(wait / 1000.0));
                }
            } else if crate::child::active() || crate::zlib::active() {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }

    /// Run the event loop until no referenced timer/immediate remains, emitting `beforeExit` and
    /// `exit`. Returns the process exit code; an uncaught exception comes back as `Err`.
    pub fn run_event_loop(&mut self) -> R<i32> {
        let mut code = None;
        loop {
            match self.loop_inner() {
                Ok(()) => {}
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
        if let Some(p) = self.rt.process.clone() {
            put(&p, "exitCode", Value::Number(code as f64));
        }
        match self.emit("exit", &[Value::Number(code as f64)]) {
            Ok(_) | Err(Ctl::Exit(_)) => {}
            Err(e) => return Err(e),
        }
        Ok(self.exit_code())
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

    fn exit_code(&mut self) -> i32 {
        let Some(p) = self.rt.process.clone() else {
            return 0;
        };
        match find_own(&p, "exitCode") {
            Some((Slot::Data(Value::Number(n)), _)) => n as i32,
            _ => 0,
        }
    }

    /// Run a script as a program: evaluate it, then run the event loop. Returns the exit code.
    pub fn run_main(&mut self, source: &str) -> R<i32> {
        match self.run(source) {
            Ok(_) => self.run_event_loop(),
            Err(Ctl::Exit(c)) => {
                if let Some(p) = self.rt.process.clone() {
                    put(&p, "exitCode", Value::Number(c as f64));
                }
                match self.emit("exit", &[Value::Number(c as f64)]) {
                    Ok(_) | Err(Ctl::Exit(_)) => Ok(self.exit_code()),
                    Err(e) => Err(e),
                }
            }
            Err(e) => self.fatal(e),
        }
    }

    /// An uncaught exception: `exit` listeners still run, with code 1.
    fn fatal(&mut self, e: Ctl) -> R<i32> {
        if let Some(p) = self.rt.process.clone() {
            put(&p, "exitCode", Value::Number(1.0));
        }
        let _ = self.emit("exit", &[Value::Number(1.0)]);
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
            print!("{rest}");
        } else {
            self.out.push(rest);
        }
    }

    // ----- process events -----

    fn emit(&mut self, name: &str, args: &[Value]) -> R<bool> {
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

    /// Advance the timer base by `ms` (start-up work that must not count against program timers).
    #[cfg(feature = "intl")]
    pub(crate) fn skip_timer_clock(&mut self, ms: f64) {
        self.rt.now += ms;
    }

    pub(crate) fn install_eventloop(&mut self) {
        let g = self.global.clone();
        self.rt.now = clock_ms() - 2.0;
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
        self.set_native(&timeout_proto, "refresh", |it, this, _| {
            if let Some(id) = it.timer_id(this) {
                let now = it.rt.now;
                if let Some(t) = it.rt.timers.iter_mut().find(|t| t.id == id) {
                    t.expiry = now + t.delay;
                }
            }
            Ok(this.clone())
        });
        self.set_native(&timeout_proto, "close", |it, this, _| {
            if let Some(id) = it.timer_id(this) {
                it.rt.timers.retain(|t| t.id != id);
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
                    it.rt.timers.retain(|t| t.id != id);
                }
                Ok(Value::Undefined)
            });
            g.declare(name, f, true);
        }
        let f = self.native(|it, _, a| {
            let cb = arg(a, 0);
            it.need_callback(&cb)?;
            it.rt.next_id += 1;
            let id = it.rt.next_id;
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
                it.rt.immediates.retain(|t| t.id != id);
                it.rt.cleared.push(id);
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

    /// Promise combinators in JS; needs `Error`, so it runs after the main prelude.
    pub(crate) fn run_promise_prelude(&mut self) {
        self.run(PROMISE_PRELUDE)
            .unwrap_or_else(|_| panic!("thaw-js promise prelude must evaluate"));
    }

    fn install_process(&mut self) {
        let g = self.global.clone();
        let p = self.plain();
        self.rt.process = Some(p.clone());
        put(&p, "title", Value::str("node"));
        put(&p, "version", Value::str("v22.23.1"));
        put(&p, "platform", Value::str("linux"));
        put(&p, "arch", Value::str("x64"));
        put(&p, "pid", Value::Number(std::process::id() as f64));
        put(&p, "execPath", Value::str("/usr/local/bin/node"));
        put(&p, "exitCode", Value::Undefined);
        let versions = self.plain();
        put(&versions, "node", Value::str("22.23.1"));
        put(&versions, "v8", Value::str("12.4.254.21-node.33"));
        put(&p, "versions", Value::Object(versions));
        let release = self.plain();
        put(&release, "name", Value::str("node"));
        put(&p, "release", Value::Object(release));
        let argv = self.array(vec![
            Value::str("/usr/local/bin/node"),
            Value::str("main.js"),
        ]);
        put(&p, "argv", argv);
        let exec_argv = self.array(vec![]);
        put(&p, "execArgv", exec_argv);
        let env = self.plain();
        for (k, v) in std::env::vars() {
            put(&env, &k, Value::str(&v));
        }
        put(&p, "env", Value::Object(env));
        self.set_native(&p, "cwd", |_, _, _| {
            Ok(Value::str(
                &std::env::current_dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|_| "/".into()),
            ))
        });
        self.set_native(&p, "binding", |it, _, a| {
            let name = it.to_str(&arg(a, 0));
            it.throw_error("Error", &format!("No such module: {name}"))
        });
        self.set_native(&p, "uptime", |_, _, _| {
            Ok(Value::Number(clock_ms() / 1000.0))
        });
        self.set_native(&p, "hrtime", |it, _, a| {
            let ns = (clock_ms() * 1e6) as u64 + 1_000_000_000;
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
        self.set_native(&p, "memoryUsage", |it, _, _| {
            let o = it.plain();
            for k in ["rss", "heapTotal", "heapUsed", "external", "arrayBuffers"] {
                put(&o, k, Value::Number(0.0));
            }
            Ok(Value::Object(o))
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
            Err(Ctl::Exit(code))
        });
        self.set_native(&p, "emitWarning", |it, _, a| {
            let msg = it.to_str(&arg(a, 0));
            let pid = std::process::id();
            it.emit_err(&format!("(node:{pid}) Warning: {msg}"));
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
                let text = it.to_str(&arg(a, 0)).to_string();
                if is_err {
                    it.emit_err(text.trim_end_matches('\n'));
                } else {
                    it.emit_out(&text, false);
                }
                Ok(Value::Bool(true))
            });
            put(&s, "isTTY", Value::Bool(false));
            put(&p, name, Value::Object(s));
        }
        g.declare("process", Value::Object(p), true);
    }
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
