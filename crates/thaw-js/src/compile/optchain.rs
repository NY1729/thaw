//! Optional chains (`a?.b`, `a?.[k]`, `a?.(x)`), compiled as `Interp::eval_opt` runs them: every part yields
//! `None` when the chain short-circuits, and the whole chain is then `undefined`.

use std::rc::Rc;

use thaw_parser::ast as a;

use super::call::{args_of, eval_args, Arg};
use super::expr::{self, mk_ex};
use super::gen::{reject, Gen, CG};
use super::member::{self, PropKey};
use super::run::give_arg_buf;
use super::{bx, Ex, Frame, CR};
use crate::{Interp, Value};

type Chain = Box<dyn Fn(&mut Interp, &mut Frame) -> CR<Option<Value>>>;

fn nullish(v: &Value) -> bool {
    matches!(v, Value::Undefined | Value::Null)
}

/// `Interp::eval_chain_part`: a nested chain, or any other expression (never short-circuits).
fn part(g: &mut Gen, e: &a::Expr) -> CG<Chain> {
    match e {
        a::Expr::OptChain(oc) => chain(g, oc),
        other => {
            let x = expr::compile(g, other)?;
            Ok(Box::new(move |it, f| Ok(Some(x(it, f)?))))
        }
    }
}

pub(crate) fn compile(g: &mut Gen, oc: &a::OptChainExpr) -> CG<Ex> {
    let c = chain(g, oc)?;
    Ok(mk_ex(
        move |it, f| Ok(c(it, f)?.unwrap_or(Value::Undefined)),
    ))
}

/// How the callee of a call in a chain yields `(function, this)`.
enum Callee {
    /// `o.m(...)` / `o?.m(...)`: `optional` is the call's own `?.`
    Member(Chain, PropKey),
    /// `a.b?.c(...)`: the callee is an optional member access; `a.b` stays `this`.
    InnerOptional(Chain, PropKey, bool),
    Other(Chain),
}

fn chain(g: &mut Gen, oc: &a::OptChainExpr) -> CG<Chain> {
    let optional = oc.optional;
    match &*oc.base {
        a::OptChainBase::Member(m) => {
            let obj = part(g, &m.obj)?;
            let key = member::prop_key(g, &m.prop)?;
            Ok(Box::new(move |it, f| {
                let Some(o) = obj(it, f)? else {
                    return Ok(None);
                };
                if optional && nullish(&o) {
                    return Ok(None);
                }
                let k = member::callee_key(it, f, &key)?;
                Ok(Some(bx(it.get(&o, &k))?))
            }))
        }
        a::OptChainBase::Call(c) => {
            let text: Rc<str> = Rc::from(crate::expr_text(&c.callee));
            let callee = match &*c.callee {
                a::Expr::Member(m) => {
                    Callee::Member(part(g, &m.obj)?, member::prop_key(g, &m.prop)?)
                }
                a::Expr::OptChain(inner) if matches!(&*inner.base, a::OptChainBase::Member(_)) => {
                    let a::OptChainBase::Member(m) = &*inner.base else {
                        return reject();
                    };
                    Callee::InnerOptional(
                        part(g, &m.obj)?,
                        member::prop_key(g, &m.prop)?,
                        inner.optional,
                    )
                }
                other => Callee::Other(part(g, other)?),
            };
            let args: Vec<Arg> = args_of(g, &c.args)?;
            Ok(Box::new(move |it, f| {
                let (fv, this) = match &callee {
                    Callee::Member(obj, key) => {
                        let Some(o) = obj(it, f)? else {
                            return Ok(None);
                        };
                        let k = member::callee_key(it, f, key)?;
                        let fv = if nullish(&o) && optional {
                            Value::Undefined
                        } else {
                            bx(it.get(&o, &k))?
                        };
                        (fv, o)
                    }
                    Callee::InnerOptional(obj, key, inner_optional) => {
                        let Some(o) = obj(it, f)? else {
                            return Ok(None);
                        };
                        if *inner_optional && nullish(&o) {
                            return Ok(None);
                        }
                        let k = member::callee_key(it, f, key)?;
                        (bx(it.get(&o, &k))?, o)
                    }
                    Callee::Other(p) => {
                        let Some(fv) = p(it, f)? else {
                            return Ok(None);
                        };
                        (fv, Value::Undefined)
                    }
                };
                if optional && nullish(&fv) {
                    return Ok(None);
                }
                let argv = eval_args(it, f, &args)?;
                let r = if fv.is_callable() {
                    bx(it.call(&fv, &this, &argv))
                } else {
                    bx(it.throw_error("TypeError", &format!("{text} is not a function")))
                };
                give_arg_buf(it, argv);
                Ok(Some(r?))
            }))
        }
    }
}
