//! One call of a compiled function: the frame, the `this` rule of `call_closure_ast`, and the body.

use std::rc::Rc;

use super::{unbox, CompiledFn, Flow, Frame};
use crate::{Ctl, Interp, Scope, Value, R};

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
        let mut slots = vec![Value::Undefined; self.nslots as usize];
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
        if let Some(e) = &self.expr_body {
            return unbox(e(it, &mut frame));
        }
        for st in &self.body {
            match st(it, &mut frame) {
                Ok(Flow::Next) => {}
                Ok(Flow::Return) => return Ok(std::mem::replace(&mut frame.ret, Value::Undefined)),
                // break/continue cannot leave a function body (labels and loops are C2 PR3)
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
