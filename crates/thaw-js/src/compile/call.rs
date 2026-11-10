//! Calls and `new` (plan TJ151 section 5, PR1-PR2). Every call goes through `Interp::call_direct` (PR2), which is
//! `Interp::call` for everything except a compiled closure, and `Interp::construct` for `new`, the same helpers the
//! AST arms of `Interp::eval` use, in the same order: callee (or object and key, then the getter), the arguments
//! left to right, the callable check, then the call. The check comes after the arguments.

use std::rc::Rc;

use thaw_parser::ast as a;

use super::check::applies;
use super::expr::{self, mk_ex};
use super::gen::{reject, Gen, CG};
use super::member::{self, PropKey};
use super::run::give_arg_buf;
use super::{bx, CState, Ex, Frame, CR};
use crate::{Interp, Kind, Value, R};

/// The callee of a call: a plain expression (`this` undefined), or a property of an object (`this` the object).
enum Target {
    Plain(Ex),
    Method(Ex, PropKey),
}

/// The argument expressions, in order. Spread arguments are blocked by the analyzer; this is the second check.
fn args_of(g: &mut Gen, args: &[a::ExprOrSpread]) -> CG<Vec<Ex>> {
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        if arg.spread.is_some() {
            return reject();
        }
        out.push(expr::compile(g, &arg.expr)?);
    }
    Ok(out)
}

/// The argument values, left to right, in a buffer from `Interp::arg_pool`. A throw part way returns the buffer
/// (emptied) to the pool before it propagates. The caller gives the buffer back after the call.
fn eval_args(it: &mut Interp, f: &mut Frame, args: &[Ex]) -> CR<Vec<Value>> {
    if args.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = it.arg_pool.pop().unwrap_or_default();
    #[cfg(test)]
    if out.capacity() > 0 {
        it.cstats.note_arg_pool_hit();
    }
    for x in args {
        match x(it, f) {
            Ok(v) => out.push(v),
            Err(e) => {
                give_arg_buf(it, out);
                return Err(e);
            }
        }
    }
    Ok(out)
}

/// `f(args)` and `o.m(args)` / `o[k](args)`. `(o.m)(args)` is a plain call here too (`this` undefined), as in
/// `Interp::eval`, whose `Member` shape test looks at the callee node itself, not through parentheses.
pub(crate) fn call(g: &mut Gen, c: &a::CallExpr) -> CG<Ex> {
    let a::Callee::Expr(callee) = &c.callee else {
        return reject();
    };
    let text: Rc<str> = Rc::from(crate::expr_text(callee));
    let target = match &**callee {
        a::Expr::SuperProp(_) => return reject(),
        a::Expr::Member(m) => {
            let obj = expr::compile(g, &m.obj)?;
            let key = member::prop_key(g, &m.prop)?;
            Target::Method(obj, key)
        }
        other => Target::Plain(expr::compile(g, other)?),
    };
    let args = args_of(g, &c.args)?;
    Ok(mk_ex(move |it, f| {
        let (fv, this) = match &target {
            Target::Plain(e) => (e(it, f)?, Value::Undefined),
            Target::Method(obj, key) => {
                let o = obj(it, f)?;
                let k = member::callee_key(it, f, key)?;
                (bx(it.get(&o, &k))?, o)
            }
        };
        let mut argv = eval_args(it, f, &args)?;
        let r = if fv.is_callable() {
            bx(it.call_direct_owned(&fv, &this, &mut argv))
        } else {
            bx(it.throw_error("TypeError", &format!("{text} is not a function")))
        };
        give_arg_buf(it, argv);
        r
    }))
}

/// `new C(args)` and `new C` (no parentheses): the callee, the arguments, the constructor check, then construct.
pub(crate) fn new_expr(g: &mut Gen, n: &a::NewExpr) -> CG<Ex> {
    let text: Rc<str> = Rc::from(crate::expr_text(&n.callee));
    let callee = expr::compile(g, &n.callee)?;
    let args = match &n.args {
        Some(list) => args_of(g, list)?,
        None => Vec::new(),
    };
    Ok(mk_ex(move |it, f| {
        let fv = callee(it, f)?;
        let argv = eval_args(it, f, &args)?;
        let r = if fv.is_callable() {
            bx(it.construct(&fv, &argv))
        } else {
            bx(it.throw_error("TypeError", &format!("{text} is not a constructor")))
        };
        give_arg_buf(it, argv);
        r
    }))
}

impl Interp {
    /// `Interp::call` with the compiled-to-compiled shortcut (PR2). A closure whose state is `Compiled` runs here
    /// with the steps `call` -> `call_closure_sync` -> `call_closure_body` -> `try_compiled` take for it: the depth
    /// and deadline checks, `depth`, the pending `new.target`, the `run_id` swap and restore, and the call counters.
    /// Everything else (natives, classes, generators, async, uncompiled closures, check-mode pure kernels) goes
    /// through `call`, unchanged.
    /// `call_direct_owned` for a caller holding the arguments as a shared slice (tests).
    #[cfg(test)]
    pub(crate) fn call_direct(&mut self, f: &Value, this: &Value, args: &[Value]) -> R<Value> {
        let mut owned = args.to_vec();
        self.call_direct_owned(f, this, &mut owned)
    }

    /// `args` is the caller's argument buffer; a compiled callee moves the values out of it (the caller then
    /// gives the buffer back), every other callee sees it as a shared slice.
    pub(crate) fn call_direct_owned(
        &mut self,
        f: &Value,
        this: &Value,
        args: &mut [Value],
    ) -> R<Value> {
        let Value::Object(fo) = f else {
            return self.call(f, this, args);
        };
        let hit = match &*fo.kind.borrow() {
            Kind::Closure { def, scope }
                if def.class.is_none() && !def.is_gen && !def.is_async && !def.uses_new_target =>
            {
                match &*def.cstate.borrow() {
                    CState::Compiled(c) if !applies(self.compile_mode, c) => {
                        Some((def.src.0, c.clone(), scope.clone()))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let Some((src_run, c, scope)) = hit else {
            return self.call(f, this, args);
        };
        if self.too_deep() {
            return self.throw_error("RangeError", "Maximum call stack size exceeded");
        }
        self.check_deadline()?;
        self.depth += 1;
        // `call_closure_body` takes the pending new.target before it dispatches
        self.rt.pending_nt.take();
        self.cstats.calls += 1;
        self.cstats.calls_eligible += 1;
        self.cstats.compiled_calls += 1;
        self.cstats.note_direct_call();
        let caller_run = std::mem::replace(&mut self.run_id, src_run);
        let r = c.run(self, f, scope, this, args);
        self.run_id = caller_run;
        self.depth -= 1;
        r
    }
}
