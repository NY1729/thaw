//! One call of a compiled function: the frame, the `this` rule of `call_closure_ast`, and the body.

use std::rc::Rc;

use super::{unbox, CompiledFn, Flow, Frame};
use crate::{Ctl, Interp, Scope, Value, R};

/// Most empty slot buffers kept for reuse. Nested calls each hold one, so this bounds the pool, not the depth.
const FRAME_POOL_CAP: usize = 64;

/// Most empty argument buffers kept for reuse (`Interp::arg_pool`), as `FRAME_POOL_CAP`.
pub(super) const ARG_POOL_CAP: usize = 64;

/// Returns an argument buffer to the pool: its values are dropped now, and only the empty buffer is kept.
pub(super) fn give_arg_buf(it: &mut Interp, mut buf: Vec<Value>) {
    buf.clear();
    if it.arg_pool.len() < ARG_POOL_CAP {
        it.arg_pool.push(buf);
    }
}

impl CompiledFn {
    /// `run` for a caller holding the arguments as a shared slice: the values are copied into a pooled buffer
    /// first, so the caller's slice is left intact (`run` moves values out of its buffer).
    pub(crate) fn run_shared(
        &self,
        it: &mut Interp,
        callee: &Value,
        scope: &Rc<Scope>,
        this: &Value,
        args: &[Value],
    ) -> R<Value> {
        let mut owned = it.arg_pool.pop().unwrap_or_default();
        owned.extend_from_slice(args);
        let r = self.run(it, callee, scope.clone(), this, &mut owned);
        give_arg_buf(it, owned);
        r
    }

    /// One call. `args` is the caller's buffer: each parameter value is moved out of it (the caller clears the
    /// rest), so no argument is cloned. `scope` is the closure's scope, moved into the frame.
    pub(crate) fn run(
        &self,
        it: &mut Interp,
        callee: &Value,
        scope: Rc<Scope>,
        this: &Value,
        args: &mut [Value],
    ) -> R<Value> {
        // the same normalisation as `call_closure_ast`: sloppy undefined/null `this` is the global object
        let this = if self.arrow || !self.uses_this {
            Value::Undefined
        } else if !(self.strict || scope.strict.get())
            && matches!(this, Value::Undefined | Value::Null)
        {
            Value::Object(it.global_object.clone())
        } else {
            this.clone()
        };
        // a recycled buffer (`resize` fills it with Undefined); a nested call finds the pool empty or smaller
        // and takes its own buffer, so no live frame is ever shared
        let mut slots = it.frame_pool.pop().unwrap_or_default();
        slots.resize(self.nslots as usize, Value::Undefined);
        if let Some(k) = self.self_slot {
            slots[k as usize] = callee.clone();
        }
        // in argument order, so a duplicate parameter name ends with the last argument (as `insert` does)
        for (i, slot) in self.param_slots.iter().enumerate() {
            if let Some(k) = slot {
                slots[*k as usize] = match args.get_mut(i) {
                    Some(a) => std::mem::replace(a, Value::Undefined),
                    None => Value::Undefined,
                };
            }
        }
        // the call's own `Scope` (as `call_closure_ast` makes): the parameters, then the `var`s, which are not
        // slots because an inner closure names them. A function without inner closures keeps the closure scope.
        let scope = match &self.frame {
            None => scope,
            Some(init) => {
                it.cstats.scope_frames += 1;
                let s = Scope::new(Some(scope));
                if self.strict {
                    s.strict.set(true);
                }
                if init.set_this {
                    s.set_this(this.clone());
                }
                for (i, name) in &init.params {
                    let v = match args.get_mut(*i) {
                        Some(a) => std::mem::replace(a, Value::Undefined),
                        None => Value::Undefined,
                    };
                    s.declare_rc(name.clone(), v, true);
                }
                for name in &init.vars {
                    s.declare_if_absent(name);
                }
                s
            }
        };
        let mut frame = Frame {
            slots,
            scope,
            this,
            ret: Value::Undefined,
        };
        let r = self.body(it, &mut frame);
        // every exit path (return, throw, fallthrough) comes here: the values are dropped now, as the AST frame's
        // are at return, and only the empty buffer is kept
        let mut slots = std::mem::take(&mut frame.slots);
        slots.clear();
        if it.frame_pool.len() < FRAME_POOL_CAP {
            it.frame_pool.push(slots);
        }
        r
    }

    fn body(&self, it: &mut Interp, frame: &mut Frame) -> R<Value> {
        if let Some(e) = &self.expr_body {
            return unbox(e(it, frame));
        }
        for st in &self.body {
            match st(it, frame) {
                Ok(Flow::Next) => {}
                Ok(Flow::Return) => return Ok(std::mem::replace(&mut frame.ret, Value::Undefined)),
                // break/continue are compiled only inside a loop, so they cannot leave the body
                Ok(_) => return Ok(Value::Undefined),
                // an expression-level `Ctl::Return` (generator resumption) is a return value, as in the AST
                Err(b) => {
                    return match *b {
                        Ctl::Return(v) => Ok(v),
                        other => Err(other),
                    }
                }
            }
        }
        Ok(Value::Undefined)
    }
}
