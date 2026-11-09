//! One call of a compiled function: the frame, the `this` rule of `call_closure_ast`, and the body.

use std::rc::Rc;

use super::{unbox, CompiledFn, Flow, Frame};
use crate::{Ctl, Interp, Scope, Value, R};

/// Most empty slot buffers kept for reuse. Nested calls each hold one, so this bounds the pool, not the depth.
const FRAME_POOL_CAP: usize = 64;

impl CompiledFn {
    pub(crate) fn run(
        &self,
        it: &mut Interp,
        scope: &Rc<Scope>,
        this: &Value,
        args: &[Value],
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
        // in argument order, so a duplicate parameter name ends with the last argument (as `insert` does)
        for (i, slot) in self.param_slots.iter().enumerate() {
            if let Some(k) = slot {
                slots[*k as usize] = args.get(i).cloned().unwrap_or(Value::Undefined);
            }
        }
        let mut frame = Frame {
            slots,
            scope: scope.clone(),
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
