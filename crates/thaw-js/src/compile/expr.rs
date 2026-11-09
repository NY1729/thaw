//! Expression nodes (plan TJ150 section 4, PR1 literals/identifiers/`this`/sequences; PR2 operators and
//! assignment on slots and free names). Every node calls the same interpreter helper the AST arm calls,
//! in the same order: `combine` is `number_binop` then `binary`, the way `Interp::eval` does it.

use thaw_parser::ast as a;

use super::gen::{reject, Gen, CG};
use super::member;
use super::{bx, Ex, Frame, CR};
use crate::{bigint, builtins, to_int32, Interp, JsStr, Kind, Value, R};

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
        a::Lit::BigInt(b) => bigint::big((*b.value).clone()),
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
        a::Expr::Tpl(t) => template(g, t),
        a::Expr::Cond(c) => {
            let test = compile(g, &c.test)?;
            let cons = compile(g, &c.cons)?;
            let alt = compile(g, &c.alt)?;
            Ok(cond(test, cons, alt))
        }
        a::Expr::Bin(b) => bin(g, b.op, &b.left, &b.right),
        a::Expr::Unary(u) => unary(g, u.op, &u.arg),
        a::Expr::Update(u) => update(g, &u.arg, matches!(u.op, a::UpdateOp::PlusPlus), u.prefix),
        a::Expr::Assign(asg) => assign(g, asg),
        a::Expr::Member(m) => member::read(g, m),
        a::Expr::Array(arr) => member::array(g, arr),
        a::Expr::Object(o) => member::object(g, o),
        _ => reject(),
    }
}

/// A name that an assignment or `++`/`--` writes: a slot of this frame, or a free name (closure scope).
#[derive(Clone)]
enum Place {
    Slot { k: u32, mutable: bool },
    Free(a::Ident),
}

impl Place {
    fn of(g: &Gen, id: &a::Ident) -> Place {
        match g.resolve(&id.sym) {
            Some(l) => Place::Slot {
                k: l.slot,
                mutable: l.mutable,
            },
            None => Place::Free(id.clone()),
        }
    }

    /// Reads the name as `read_loc` does for an identifier.
    fn read(&self, it: &mut Interp, f: &mut Frame) -> CR<Value> {
        match self {
            Place::Slot { k, .. } => Ok(f.slots[*k as usize].clone()),
            Place::Free(id) => bx(it.eval_ident(id, &f.scope)),
        }
    }

    /// Writes the name as `write_loc` does: a `const` throws, after the value was computed.
    fn write(&self, it: &mut Interp, f: &mut Frame, v: Value) -> CR<()> {
        match self {
            Place::Slot { k, mutable: true } => {
                f.slots[*k as usize] = v;
                Ok(())
            }
            Place::Slot { mutable: false, .. } => {
                bx(it.throw_error::<()>("TypeError", "Assignment to constant variable."))
            }
            Place::Free(id) => bx(it.assign_var(&f.scope, &id.sym, v)),
        }
    }
}

/// `x = e`: the right-hand side, then the write; the value of the assignment is the stored value.
fn store(p: Place, rhs: Ex) -> Ex {
    mk_ex(move |it, f| {
        let v = rhs(it, f)?;
        p.write(it, f, v.clone())?;
        Ok(v)
    })
}

/// `x &&= e`, `x ||= e`, `x ??= e`: the right-hand side only when the current value does not keep.
fn logical_assign(p: Place, op: a::AssignOp, rhs: Ex) -> Ex {
    use a::AssignOp as A;
    mk_ex(move |it, f| {
        let cur = p.read(it, f)?;
        let keep = match op {
            A::AndAssign => !cur.truthy(),
            A::OrAssign => cur.truthy(),
            _ => !matches!(cur, Value::Undefined | Value::Null),
        };
        if keep {
            return Ok(cur);
        }
        let v = rhs(it, f)?;
        p.write(it, f, v.clone())?;
        Ok(v)
    })
}

/// `x op= e` for the operators behind `compound_binop`: the old value is read before the right-hand side.
fn compound(p: Place, op: a::BinaryOp, rhs: Ex) -> Ex {
    mk_ex(move |it, f| {
        let cur = p.read(it, f)?;
        let r = rhs(it, f)?;
        // `s += x` on a mutable slot holding a string, `x` primitive: appended in place
        let cur = match &p {
            Place::Slot { k, mutable: true }
                if matches!(op, a::BinaryOp::Add) && is_str_append(&cur, &r) =>
            {
                let rs = it.to_str(&r);
                match append_slot(&mut f.slots, *k as usize, cur, &rs) {
                    Ok(v) => return Ok(v),
                    Err(back) => back,
                }
            }
            _ => cur,
        };
        let v = bx(combine(it, op, &cur, &r))?;
        p.write(it, f, v.clone())?;
        Ok(v)
    })
}

/// Only `+=` of a string with a primitive can append in place (no user code runs in ToPrimitive).
fn is_str_append(cur: &Value, rhs: &Value) -> bool {
    matches!(cur, Value::Str(_))
        && matches!(
            rhs,
            Value::Str(_)
                | Value::Number(_)
                | Value::Bool(_)
                | Value::Null
                | Value::Undefined
                | Value::BigInt(_)
        )
}

/// Appends `rs` to the string in `slots[k]` if that slot still holds the snapshot `cur` (the interpreter's
/// `append_in_place`). The snapshot is released first, so a uniquely owned buffer grows without a copy, while
/// a shared or literal string is copied by `push_joined`. Otherwise `cur` comes back and the slot is untouched.
fn append_slot(slots: &mut [Value], k: usize, cur: Value, rs: &JsStr) -> Result<Value, Value> {
    let stored = std::mem::replace(&mut slots[k], Value::Undefined);
    match (stored, cur) {
        (Value::Str(mut js), Value::Str(snap)) if JsStr::ptr_eq(&js, &snap) => {
            drop(snap);
            js.push_joined(rs);
            let v = Value::Str(js);
            slots[k] = v.clone();
            Ok(v)
        }
        (stored, cur) => {
            slots[k] = stored;
            Err(cur)
        }
    }
}

/// The interpreter's operator step: two Numbers take `number_binop`, everything else `binary`.
pub(crate) fn combine(it: &mut Interp, op: a::BinaryOp, l: &Value, r: &Value) -> R<Value> {
    if let (Value::Number(x), Value::Number(y)) = (l, r) {
        if let Some(v) = crate::number_binop(op, *x, *y) {
            return Ok(v);
        }
    }
    it.binary(op, l, r)
}

/// `x = e` / `x op= e` / `x &&= e` on an identifier, or a member target (`member::assign`).
fn assign(g: &mut Gen, asg: &a::AssignExpr) -> CG<Ex> {
    use a::AssignOp as A;
    let id = match &asg.left {
        a::AssignTarget::Simple(a::SimpleAssignTarget::Ident(id)) => &id.id,
        a::AssignTarget::Simple(a::SimpleAssignTarget::Member(m)) => {
            return member::assign(g, m, asg)
        }
        _ => return reject(),
    };
    let p = Place::of(g, id);
    let rhs = compile(g, &asg.right)?;
    match asg.op {
        A::Assign => Ok(store(p, rhs)),
        A::AndAssign | A::OrAssign | A::NullishAssign => Ok(logical_assign(p, asg.op, rhs)),
        op => match crate::compound_binop(op) {
            Some(b) => Ok(compound(p, b, rhs)),
            None => reject(),
        },
    }
}

/// `++x` / `x++` / `--x` / `x--` on an identifier or a member (`member::update`).
fn update(g: &mut Gen, arg: &a::Expr, inc: bool, prefix: bool) -> CG<Ex> {
    let i = match arg {
        a::Expr::Ident(i) => i,
        a::Expr::Member(m) => return member::update(g, m, inc, prefix),
        _ => return reject(),
    };
    let p = Place::of(g, i);
    Ok(mk_ex(move |it, f| {
        let old = p.read(it, f)?;
        if let Value::BigInt(b) = &old {
            let one = num_bigint::BigInt::from(1);
            let new = bigint::big(if inc { &**b + one } else { &**b - one });
            p.write(it, f, new.clone())?;
            return Ok(if prefix { new } else { old.clone() });
        }
        let n = it.num(&old);
        let new = if inc { n + 1.0 } else { n - 1.0 };
        p.write(it, f, Value::Number(new))?;
        Ok(Value::Number(if prefix { new } else { n }))
    }))
}

fn cond(test: Ex, cons: Ex, alt: Ex) -> Ex {
    mk_ex(move |it, f| {
        if test(it, f)?.truthy() {
            cons(it, f)
        } else {
            alt(it, f)
        }
    })
}

/// Binary operators. `in` and `instanceof` stay on the AST (the analyzer blocks them too).
fn bin(g: &mut Gen, op: a::BinaryOp, left: &a::Expr, right: &a::Expr) -> CG<Ex> {
    use a::BinaryOp as B;
    if matches!(op, B::In | B::InstanceOf) {
        return reject();
    }
    let l = compile(g, left)?;
    let r = compile(g, right)?;
    Ok(match op {
        B::LogicalAnd | B::LogicalOr | B::NullishCoalescing => logical(op, l, r),
        _ => mk_ex(move |it, f| {
            let lv = l(it, f)?;
            let rv = r(it, f)?;
            bx(combine(it, op, &lv, &rv))
        }),
    })
}

/// `&&`, `||`, `??`: the operand value, the right side only when the left one does not decide.
fn logical(op: a::BinaryOp, l: Ex, r: Ex) -> Ex {
    use a::BinaryOp as B;
    mk_ex(move |it, f| {
        let lv = l(it, f)?;
        let take_right = match op {
            B::LogicalAnd => lv.truthy(),
            B::LogicalOr => !lv.truthy(),
            _ => matches!(lv, Value::Undefined | Value::Null),
        };
        if take_right {
            r(it, f)
        } else {
            Ok(lv)
        }
    })
}

/// `typeof`, `!`, `-`, `+`, `~`, `void`. `delete` stays on the AST.
fn unary(g: &mut Gen, op: a::UnaryOp, arg: &a::Expr) -> CG<Ex> {
    use a::UnaryOp as U;
    if let (U::TypeOf, a::Expr::Ident(i)) = (op, arg) {
        // an undeclared name is "undefined", not a ReferenceError (as `eval_unary`)
        let p = Place::of(g, i);
        return Ok(mk_ex(move |it, f| {
            let v = match &p {
                Place::Slot { k, .. } => f.slots[*k as usize].clone(),
                Place::Free(id) => it.lookup_ident(&f.scope, id).unwrap_or(Value::Undefined),
            };
            Ok(typeof_value(&v))
        }));
    }
    if !matches!(
        op,
        U::TypeOf | U::Bang | U::Minus | U::Plus | U::Tilde | U::Void
    ) {
        return reject();
    }
    let x = compile(g, arg)?;
    Ok(mk_ex(move |it, f| {
        let v = x(it, f)?;
        match op {
            U::TypeOf => Ok(typeof_value(&v)),
            _ => bx(unary_value(it, op, &v)),
        }
    }))
}

/// The `typeof` names of `eval_unary`.
fn typeof_value(v: &Value) -> Value {
    Value::str(match v {
        Value::Undefined | Value::Hole => "undefined",
        Value::Null => "object",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::Str(_) => "string",
        Value::BigInt(_) => "bigint",
        Value::Object(_) if v.is_callable() => "function",
        Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Symbol { .. }) => "symbol",
        Value::Object(_) => "object",
    })
}

/// The unary operators of `eval_unary` on a computed operand (the same arms, the same order).
fn unary_value(it: &mut Interp, op: a::UnaryOp, v: &Value) -> R<Value> {
    use a::UnaryOp as U;
    Ok(match op {
        U::Bang => Value::Bool(!v.truthy()),
        U::Minus => match v {
            Value::BigInt(b) => bigint::big(-(**b).clone()),
            _ => Value::Number(-it.num(v)),
        },
        U::Plus if matches!(v, Value::BigInt(_)) => {
            return it.throw_error("TypeError", "Cannot convert a BigInt value to a number")
        }
        U::Plus => Value::Number(it.num(v)),
        U::Tilde => match v {
            Value::BigInt(b) => bigint::big(-(**b).clone() - 1),
            _ => Value::Number(!to_int32(it.num(v)) as f64),
        },
        // `void`: the only other operator `unary` lets through
        _ => Value::Undefined,
    })
}

/// A template literal without tag: the cooked text of each quasi, then the `ToString` of each substitution.
fn template(g: &mut Gen, t: &a::Tpl) -> CG<Ex> {
    let mut parts: Vec<(String, Option<Ex>)> = Vec::with_capacity(t.quasis.len());
    for (i, q) in t.quasis.iter().enumerate() {
        let text = q
            .cooked
            .as_ref()
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let e = match t.exprs.get(i) {
            Some(e) => Some(compile(g, e)?),
            None => None,
        };
        parts.push((text, e));
    }
    Ok(mk_ex(move |it, f| {
        let mut s = String::new();
        for (text, e) in &parts {
            builtins::push_joined(&mut s, text);
            if let Some(e) = e {
                let v = e(it, f)?;
                builtins::push_joined(&mut s, &it.to_str(&v));
            }
        }
        Ok(Value::Str(s.into()))
    }))
}
