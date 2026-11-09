//! Calls and `new` (plan TJ151 section 5, PR1). Every call goes through `Interp::call` / `Interp::construct`, the
//! same helpers the AST arms of `Interp::eval` use, in the same order: callee (or object and key, then the
//! getter), the arguments left to right, the callable check, then the call. The check comes after the arguments.

use std::rc::Rc;

use thaw_parser::ast as a;

use super::expr::{self, mk_ex};
use super::gen::{reject, Gen, CG};
use super::member::{self, PropKey};
use super::{bx, Ex, Frame, CR};
use crate::{Interp, Value};

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

fn eval_args(it: &mut Interp, f: &mut Frame, args: &[Ex]) -> CR<Vec<Value>> {
    let mut out = Vec::with_capacity(args.len());
    for x in args {
        out.push(x(it, f)?);
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
        let argv = eval_args(it, f, &args)?;
        if !fv.is_callable() {
            return bx(it.throw_error("TypeError", &format!("{text} is not a function")));
        }
        bx(it.call(&fv, &this, &argv))
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
        if !fv.is_callable() {
            return bx(it.throw_error("TypeError", &format!("{text} is not a constructor")));
        }
        bx(it.construct(&fv, &argv))
    }))
}
