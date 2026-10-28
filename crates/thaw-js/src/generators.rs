//! Generators and async generators, built on the coroutine machinery of `eventloop.rs`.
//!
//! A generator object owns a lazily started coroutine thread; `next` / `throw` / `return` resume
//! it with a [`Resume`] and wait for the next [`Yield`]. Async generators queue their requests and
//! are driven from the event loop (an `await` inside the body suspends the driver, not the caller).
//! ponytail: one OS thread per started generator that is not run to completion (parked until exit).

use crate::eventloop::{Co, Resume, SendBox, Yield};
use crate::*;
use std::collections::VecDeque;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum GStatus {
    Start,
    Suspended,
    Running,
    Done,
}

#[derive(Clone, Copy)]
pub(crate) enum Req {
    Next,
    Throw,
    Return,
}

struct GenStart {
    f: Value,
    def: Rc<FuncDef>,
    scope: Rc<Scope>,
    this: Value,
    args: Vec<Value>,
}

pub(crate) struct GenData {
    status: Cell<GStatus>,
    start: RefCell<Option<GenStart>>,
    co: RefCell<Option<Rc<Co>>>,
    queue: RefCell<VecDeque<(Req, Value, Obj)>>,
}

fn gen_data(this: &Value) -> Option<Rc<GenData>> {
    match this {
        Value::Object(o) => match &*o.kind.borrow() {
            Kind::Generator(g) => Some(g.clone()),
            _ => None,
        },
        _ => None,
    }
}

impl Interp {
    pub(crate) fn iter_result(&self, v: Value, done: bool) -> Value {
        let o = self.plain();
        put(&o, "value", v);
        put(&o, "done", Value::Bool(done));
        Value::Object(o)
    }

    pub(crate) fn well_known_key(&mut self, name: &str) -> Rc<str> {
        let sym = self
            .lookup(&self.global.clone(), "Symbol")
            .unwrap_or(Value::Undefined);
        let tag = self.get(&sym, name).unwrap_or(Value::Undefined);
        self.key_of(&tag)
    }

    pub(crate) fn install_generators(&mut self) {
        let gp = self.plain();
        let agp = self.plain();
        self.rt.generator_proto = Some(gp.clone());
        self.rt.async_generator_proto = Some(agp.clone());
        for (proto, asyn) in [(&gp, false), (&agp, true)] {
            for (name, req) in [
                ("next", Req::Next),
                ("throw", Req::Throw),
                ("return", Req::Return),
            ] {
                self.set_native(proto, name, move |it, this, args| {
                    let Some(g) = gen_data(this) else {
                        return it.throw_error(
                            "TypeError",
                            &format!("{name} method called on incompatible receiver"),
                        );
                    };
                    let v = arg(args, 0);
                    if asyn {
                        let Value::Object(o) = this else {
                            unreachable!()
                        };
                        it.agen_enqueue(o, &g, req, v)
                    } else {
                        it.gen_resume(&g, req, v)
                    }
                });
            }
            let key = self.well_known_key(if asyn { "asyncIterator" } else { "iterator" });
            let me = self.native(|_, this, _| Ok(this.clone()));
            put_hidden(proto, &key, me);
            let tag = self.well_known_key("toStringTag");
            let name = if asyn { "AsyncGenerator" } else { "Generator" };
            put_hidden(proto, &tag, Value::str(name));
        }
    }

    pub(crate) fn make_generator(
        &mut self,
        f: &Value,
        def: &Rc<FuncDef>,
        scope: &Rc<Scope>,
        this: &Value,
        args: &[Value],
    ) -> R<Value> {
        let proto = match self.get(f, "prototype")? {
            Value::Object(p) => Some(p),
            _ if def.is_async => self.rt.async_generator_proto.clone(),
            _ => self.rt.generator_proto.clone(),
        };
        let data = Rc::new(GenData {
            status: Cell::new(GStatus::Start),
            start: RefCell::new(Some(GenStart {
                f: f.clone(),
                def: def.clone(),
                scope: scope.clone(),
                this: this.clone(),
                args: args.to_vec(),
            })),
            co: RefCell::new(None),
            queue: RefCell::new(VecDeque::new()),
        });
        Ok(Value::Object(new_obj(proto, Kind::Generator(data))))
    }

    // ----- sync generators -----

    pub(crate) fn gen_resume(&mut self, g: &Rc<GenData>, req: Req, v: Value) -> R<Value> {
        let status = g.status.get();
        if status == GStatus::Running {
            return self.throw_error("TypeError", "Generator is already running");
        }
        if status == GStatus::Start && !matches!(req, Req::Next) {
            g.status.set(GStatus::Done);
            g.start.borrow_mut().take();
        }
        if g.status.get() == GStatus::Done {
            return match req {
                Req::Next => Ok(self.iter_result(Value::Undefined, true)),
                Req::Return => Ok(self.iter_result(v, true)),
                Req::Throw => Err(Ctl::Throw(v)),
            };
        }
        let (co, snap) = if status == GStatus::Start {
            let st = g.start.borrow_mut().take().expect("generator start state");
            let promise = self.new_promise();
            let r = self.new_co(&st.f, &st.def, &st.scope, &st.this, &st.args, promise, None);
            match r {
                Ok((co, snap)) => {
                    *g.co.borrow_mut() = Some(co.clone());
                    (co, snap)
                }
                Err(e) => {
                    g.status.set(GStatus::Done);
                    return Err(e);
                }
            }
        } else {
            let co =
                g.co.borrow()
                    .clone()
                    .expect("suspended generator coroutine");
            self.rt.cur_co.push(co.ctx.clone());
            let snap = self.snapshot();
            let resume = match req {
                Req::Next => Resume::Value(v),
                Req::Throw => Resume::Throw(v),
                Req::Return => Resume::Return(v),
            };
            let _ = co.to_co.send(SendBox(resume));
            (co, snap)
        };
        g.status.set(GStatus::Running);
        let y = co.from_co.recv();
        self.restore(snap);
        self.rt.cur_co.pop();
        match y.map(SendBox::into_inner) {
            Ok(Yield::Yielded(v)) => {
                g.status.set(GStatus::Suspended);
                Ok(self.iter_result(v, false))
            }
            Ok(Yield::Done(r)) => {
                g.status.set(GStatus::Done);
                *g.co.borrow_mut() = None;
                Ok(self.iter_result(r?, true))
            }
            Ok(Yield::Await(_)) => {
                g.status.set(GStatus::Done);
                unsupported("await")
            }
            Err(_) => panic!("generator thread terminated unexpectedly"),
        }
    }

    // ----- async generators -----

    fn agen_enqueue(&mut self, gobj: &Obj, g: &Rc<GenData>, req: Req, v: Value) -> R<Value> {
        let p = self.new_promise();
        g.queue.borrow_mut().push_back((req, v, p.clone()));
        self.agen_drive(gobj, g)?;
        Ok(Value::Object(p))
    }

    fn agen_drive(&mut self, gobj: &Obj, g: &Rc<GenData>) -> R<()> {
        loop {
            if g.status.get() == GStatus::Running {
                return Ok(());
            }
            let front = g.queue.borrow().front().cloned();
            let Some((req, v, p)) = front else {
                return Ok(());
            };
            if g.status.get() == GStatus::Start && !matches!(req, Req::Next) {
                g.status.set(GStatus::Done);
                g.start.borrow_mut().take();
            }
            match g.status.get() {
                GStatus::Done => {
                    g.queue.borrow_mut().pop_front();
                    match req {
                        Req::Next => {
                            let r = self.iter_result(Value::Undefined, true);
                            self.resolve_promise(&p, r)?;
                        }
                        Req::Return => {
                            let r = self.iter_result(v, true);
                            self.resolve_promise(&p, r)?;
                        }
                        Req::Throw => self.reject_promise(&p, v),
                    }
                }
                GStatus::Start => {
                    let st = g.start.borrow_mut().take().expect("generator start state");
                    let promise = self.new_promise();
                    let r = self.new_co(
                        &st.f,
                        &st.def,
                        &st.scope,
                        &st.this,
                        &st.args,
                        promise,
                        Some(gobj.clone()),
                    );
                    let (co, snap) = match r {
                        Ok(x) => x,
                        Err(e) => {
                            g.status.set(GStatus::Done);
                            return Err(e);
                        }
                    };
                    *g.co.borrow_mut() = Some(co.clone());
                    g.status.set(GStatus::Running);
                    return self.co_wait(&co, snap);
                }
                _ => {
                    let co =
                        g.co.borrow()
                            .clone()
                            .expect("suspended generator coroutine");
                    g.status.set(GStatus::Running);
                    let resume = match req {
                        Req::Next => Resume::Value(v),
                        Req::Throw => Resume::Throw(v),
                        Req::Return => Resume::Return(v),
                    };
                    return self.co_step(&co, resume);
                }
            }
        }
    }

    pub(crate) fn agen_yielded(&mut self, co: &Rc<Co>, v: Value) -> R<()> {
        let gobj = co.gen.clone().expect("async generator");
        let g = gen_data(&Value::Object(gobj.clone())).expect("generator data");
        g.status.set(GStatus::Suspended);
        if let Some((_, _, p)) = g.queue.borrow_mut().pop_front() {
            let r = self.iter_result(v, false);
            self.resolve_promise(&p, r)?;
        }
        self.agen_drive(&gobj, &g)
    }

    pub(crate) fn agen_done(&mut self, co: &Rc<Co>, r: R<Value>) -> R<()> {
        let gobj = co.gen.clone().expect("async generator");
        let g = gen_data(&Value::Object(gobj.clone())).expect("generator data");
        g.status.set(GStatus::Done);
        *g.co.borrow_mut() = None;
        let front = g.queue.borrow_mut().pop_front();
        if let Some((_, _, p)) = front {
            match r {
                Ok(v) => {
                    let r = self.iter_result(v, true);
                    self.resolve_promise(&p, r)?;
                }
                Err(Ctl::Throw(e)) => self.reject_promise(&p, e),
                Err(other) => return Err(other),
            }
        }
        self.agen_drive(&gobj, &g)
    }

    // ----- inside the generator body -----

    fn yield_raw(&mut self, v: Value) -> R<Resume> {
        let Some(ctx) = self.rt.cur_co.last().cloned() else {
            return unsupported("yield");
        };
        let snap = self.snapshot();
        let _ = ctx.to_host.send(SendBox(Yield::Yielded(v)));
        let msg = ctx.from_host.recv();
        self.restore(snap);
        match msg.map(SendBox::into_inner) {
            Ok(r) => Ok(r),
            Err(_) => loop {
                // the generator was dropped while suspended: never touch shared state again
                std::thread::park();
            },
        }
    }

    pub(crate) fn yield_value(&mut self, v: Value) -> R<Value> {
        let agen = self.rt.cur_co.last().is_some_and(|c| c.agen);
        let v = if agen { self.await_value(v)? } else { v };
        match self.yield_raw(v)? {
            Resume::Value(v) => Ok(v),
            Resume::Throw(e) => Err(Ctl::Throw(e)),
            Resume::Return(v) => {
                let v = if agen { self.await_value(v)? } else { v };
                Err(Ctl::Return(v))
            }
        }
    }

    /// `(iterator, next)` of a `Symbol.asyncIterator` iterable; `None` if it has none.
    pub(crate) fn get_async_iterator(&mut self, iterable: &Value) -> R<Option<(Value, Value)>> {
        if !matches!(iterable, Value::Object(_)) {
            return Ok(None);
        }
        let key = self.well_known_key("asyncIterator");
        let method = self.get(iterable, &key)?;
        if !method.is_callable() {
            return Ok(None);
        }
        let iter = self.call(&method, iterable, &[])?;
        if !matches!(iter, Value::Object(_)) {
            return self.throw_error(
                "TypeError",
                "Result of the Symbol.asyncIterator method is not an object",
            );
        }
        let next = self.get(&iter, "next")?;
        Ok(Some((iter, next)))
    }

    pub(crate) fn yield_star(&mut self, iterable: Value) -> R<Value> {
        let agen = self.rt.cur_co.last().is_some_and(|c| c.agen);
        if !agen {
            if let Some(items) = self.iter_items(&iterable) {
                for item in items {
                    self.yield_value(item)?;
                }
                return Ok(Value::Undefined);
            }
        }
        let (iter, next, async_iter) = match if agen {
            self.get_async_iterator(&iterable)?
        } else {
            None
        } {
            Some((i, n)) => (i, n, true),
            None => match self.get_iterator(&iterable)? {
                Some((i, n)) => (i, n, false),
                None => return self.throw_error("TypeError", "yield* operand is not iterable"),
            },
        };
        let mut received = Resume::Value(Value::Undefined);
        loop {
            let r = match received {
                Resume::Value(v) => self.call(&next, &iter, &[v])?,
                Resume::Throw(e) => {
                    let t = self.get(&iter, "throw")?;
                    if !t.is_callable() {
                        self.close_iterator(&iter);
                        return self.throw_error(
                            "TypeError",
                            "The iterator does not provide a 'throw' method",
                        );
                    }
                    self.call(&t, &iter, &[e])?
                }
                Resume::Return(v) => {
                    let rf = self.get(&iter, "return")?;
                    if !rf.is_callable() {
                        return Err(Ctl::Return(v));
                    }
                    let r = self.call(&rf, &iter, &[v])?;
                    let r = if async_iter { self.await_value(r)? } else { r };
                    if !matches!(r, Value::Object(_)) {
                        return self.throw_error("TypeError", "Iterator result is not an object");
                    }
                    if self.get(&r, "done")?.truthy() {
                        let value = self.get(&r, "value")?;
                        return Err(Ctl::Return(value));
                    }
                    let value = self.get(&r, "value")?;
                    received = self.yield_raw(value)?;
                    continue;
                }
            };
            let r = if async_iter { self.await_value(r)? } else { r };
            if !matches!(r, Value::Object(_)) {
                return self.throw_error("TypeError", "Iterator result is not an object");
            }
            if self.get(&r, "done")?.truthy() {
                return self.get(&r, "value");
            }
            let value = self.get(&r, "value")?;
            received = self.yield_raw(value)?;
        }
    }
}
