//! Expression nodes (plan TJ150 section 4, PR1 subset: literals, identifiers, `this`, parentheses, sequences
//! and `=` to an identifier). Every node calls the same interpreter helper the AST arm calls.

use thaw_parser::ast as a;

use super::gen::{reject, Gen, CG};
use super::{bx, Ex, Frame, CR};
use crate::{builtins, Interp, Value};

/// Wraps a closure as an `Ex`; the bound gives the closure its `&mut Interp` / `&mut Frame` signature.
pub(crate) fn mk_ex<F>(f: F) -> Ex
where
    F: Fn(&mut Interp, &mut Frame) -> CR<Value> + 'static,
{
    Box::new(f)
}

pub(crate) fn lit(v: Value) -> Ex {
    mk_ex(move |_, _| Ok(v.clone()))
}

pub(crate) fn slot_read(k: u32) -> Ex {
    mk_ex(move |_, f| Ok(f.slots[k as usize].clone()))
}

/// A free name: the same `eval_ident` the AST uses, against the closure's scope.
fn free_read(id: a::Ident) -> Ex {
    mk_ex(move |it, f| bx(it.eval_ident(&id, &f.scope)))
}

fn frame_this() -> Ex {
    mk_ex(|_, f| Ok(f.this.clone()))
}

/// `this` in an arrow: the enclosing non-arrow frame's binding, found through the closure scope.
fn arrow_this() -> Ex {
    mk_ex(|it, f| Ok(it.this_of(&f.scope)))
}

/// `slots[k] = value`, yielding the value.
pub(crate) fn slot_store(k: u32, rhs: Ex) -> Ex {
    mk_ex(move |it, f| {
        let v = rhs(it, f)?;
        f.slots[k as usize] = v.clone();
        Ok(v)
    })
}

/// A write to a `const` slot: the right-hand side still runs first, then the interpreter's TypeError.
fn const_store(rhs: Ex) -> Ex {
    mk_ex(move |it, f| {
        rhs(it, f)?;
        bx(it.throw_error::<Value>("TypeError", "Assignment to constant variable."))
    })
}

/// A write to a free name: `assign_var` against the closure's scope (sloppy implicit globals included).
fn free_store(name: String, rhs: Ex) -> Ex {
    mk_ex(move |it, f| {
        let v = rhs(it, f)?;
        bx(it.assign_var(&f.scope, &name, v.clone()))?;
        Ok(v)
    })
}

/// Comma operator: every operand, value of the last.
pub(crate) fn sequence(list: Vec<Ex>) -> Ex {
    mk_ex(move |it, f| {
        let mut last = Value::Undefined;
        for x in &list {
            last = x(it, f)?;
        }
        Ok(last)
    })
}

/// The value of a literal, computed once at compile time (the same arms as `Interp::eval`).
fn lit_value(l: &a::Lit) -> CG<Value> {
    Ok(match l {
        a::Lit::Num(n) => Value::Number(n.value),
        a::Lit::Str(s) => match s.value.as_str() {
            Some(x) => Value::str(x),
            None => Value::Str(
                builtins::units_str(&s.value.to_ill_formed_utf16().collect::<Vec<u16>>()).into(),
            ),
        },
        a::Lit::Bool(b) => Value::Bool(b.value),
        a::Lit::Null(_) => Value::Null,
        a::Lit::BigInt(b) => crate::bigint::big((*b.value).clone()),
        _ => return reject(),
    })
}

pub(crate) fn compile(g: &mut Gen, e: &a::Expr) -> CG<Ex> {
    match e {
        a::Expr::Lit(l) => Ok(lit(lit_value(l)?)),
        a::Expr::Ident(i) => Ok(match g.resolve(&i.sym) {
            Some(l) => slot_read(l.slot),
            None => free_read(i.clone()),
        }),
        a::Expr::This(_) => Ok(if g.arrow { arrow_this() } else { frame_this() }),
        a::Expr::Paren(p) => compile(g, &p.expr),
        a::Expr::Seq(s) => {
            let mut list = Vec::with_capacity(s.exprs.len());
            for x in &s.exprs {
                list.push(compile(g, x)?);
            }
            Ok(sequence(list))
        }
        a::Expr::Assign(asg) => assign(g, asg),
        _ => reject(),
    }
}

/// `x = e` with an identifier target. Other operators and targets are C2 PR2/PR4.
fn assign(g: &mut Gen, asg: &a::AssignExpr) -> CG<Ex> {
    if !matches!(asg.op, a::AssignOp::Assign) {
        return reject();
    }
    let a::AssignTarget::Simple(a::SimpleAssignTarget::Ident(id)) = &asg.left else {
        return reject();
    };
    let rhs = compile(g, &asg.right)?;
    let name = id.id.sym.to_string();
    Ok(match g.resolve(&name) {
        Some(l) if l.mutable => slot_store(l.slot, rhs),
        Some(_) => const_store(rhs),
        None => free_store(name, rhs),
    })
}
