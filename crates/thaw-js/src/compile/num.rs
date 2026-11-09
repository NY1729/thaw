//! Numeric nodes (plan TJ150 section 7, PR6). Per-operator closures for `number op number`, operands read without
//! a closure call when they are a slot or a literal, conditions that return `bool` (`Bl`), and number-valued
//! bitwise nodes (`Nm`). Every node keeps the interpreter helper (`combine`, `get_index`, `key_of`) on its slow
//! path, and evaluates each child exactly once, in source order. A Number-Number pair takes the inline expression
//! `number_binop` uses (the tests check that against `binary` over a corner table).

use thaw_parser::ast as a;

use super::elem;
use super::expr::{self, combine, mk_ex, Place};
use super::gen::{Gen, CG};
use super::{bx, Ex, Frame, CR};
use crate::{to_int32, Interp, Value};

type B = a::BinaryOp;

/// A condition: the truthiness of a test, without building a `Value::Bool`.
pub(crate) type Bl = Box<dyn Fn(&mut Interp, &mut Frame) -> CR<bool>>;
/// A number-valued operand. Its result is always a Number: a bitwise node with a Number-typed operand cannot yield
/// a BigInt (mixing throws in `binary`), and `>>>` never yields one either.
pub(crate) type Nm = Box<dyn Fn(&mut Interp, &mut Frame) -> CR<f64>>;

const _: () = assert!(std::mem::size_of::<CR<bool>>() <= 16);

/// An operand read without a closure call when it is a slot or a literal.
pub(crate) enum Leaf {
    Slot(u32),
    Const(Value),
    Ex(Ex),
    /// `slot[pure key]` (elem.rs).
    Elem(Box<elem::Elem>),
}

/// A compiled operand: `N` when its value is a Number by construction, otherwise a `Leaf`.
pub(crate) enum Side {
    N(Nm),
    L(Leaf),
}

/// An evaluated operand: `N` for a Number (no `Value` built), `V` for anything else. `V` never holds a Number.
pub(crate) enum Num2 {
    N(f64),
    V(Value),
}

pub(crate) fn num2_val(n: Num2) -> Value {
    match n {
        Num2::N(x) => Value::Number(x),
        Num2::V(v) => v,
    }
}

impl Leaf {
    /// Evaluates the operand once, without cloning a slot or literal that holds a Number.
    fn eval_n(&self, it: &mut Interp, f: &mut Frame) -> CR<Num2> {
        Ok(match self {
            Leaf::Slot(k) => match &f.slots[*k as usize] {
                Value::Number(x) => Num2::N(*x),
                v => Num2::V(v.clone()),
            },
            Leaf::Const(Value::Number(x)) => Num2::N(*x),
            Leaf::Const(v) => Num2::V(v.clone()),
            Leaf::Ex(e) => match e(it, f)? {
                Value::Number(x) => Num2::N(x),
                v => Num2::V(v),
            },
            Leaf::Elem(el) => el.eval_n(it, f)?,
        })
    }
}

impl Side {
    pub(crate) fn eval_n(&self, it: &mut Interp, f: &mut Frame) -> CR<Num2> {
        match self {
            Side::N(n) => Ok(Num2::N(n(it, f)?)),
            Side::L(l) => l.eval_n(it, f),
        }
    }
}

/// `x = v` on a slot, reusing the Number in place (the old value is dropped only when it is not a Number).
fn set_num(slot: &mut Value, v: f64) {
    match slot {
        Value::Number(s) => *s = v,
        o => *o = Value::Number(v),
    }
}

fn is_bit(op: B) -> bool {
    matches!(
        op,
        B::BitAnd | B::BitOr | B::BitXor | B::LShift | B::RShift | B::ZeroFillRShift
    )
}

/// Operators with a `number_binop`-style fast path (`**` and the logical operators stay generic).
fn is_spec(op: B) -> bool {
    matches!(
        op,
        B::Add
            | B::Sub
            | B::Mul
            | B::Div
            | B::Mod
            | B::Lt
            | B::LtEq
            | B::Gt
            | B::GtEq
            | B::EqEqEq
            | B::NotEqEq
            | B::EqEq
            | B::NotEq
    )
}

fn leaf_val(l: &Leaf, it: &mut Interp, f: &mut Frame) -> CR<Value> {
    match l {
        Leaf::Slot(k) => Ok(f.slots[*k as usize].clone()),
        Leaf::Const(v) => Ok(v.clone()),
        Leaf::Ex(e) => e(it, f),
        Leaf::Elem(el) => Ok(num2_val(el.eval_n(it, f)?)),
    }
}

fn side_val(s: &Side, it: &mut Interp, f: &mut Frame) -> CR<Value> {
    match s {
        Side::N(n) => Ok(Value::Number(n(it, f)?)),
        Side::L(l) => leaf_val(l, it, f),
    }
}

fn leaf_of(s: Side) -> Leaf {
    match s {
        Side::N(n) => Leaf::Ex(mk_ex(move |it, f| Ok(Value::Number(n(it, f)?)))),
        Side::L(l) => l,
    }
}

/// The `Ex` of an operand (a value-producing closure).
pub(crate) fn side_ex(s: Side) -> Ex {
    match leaf_of(s) {
        Leaf::Slot(k) => expr::slot_read(k),
        Leaf::Const(v) => expr::lit(v),
        Leaf::Ex(e) => e,
        Leaf::Elem(el) => elem::ex(*el),
    }
}

/// A compiled operand of any expression. Literals and slots stay leaves; a bitwise node becomes `N` when a Number
/// operand (a literal or an `N`) or `>>>` makes its result a Number.
pub(crate) fn operand(g: &mut Gen, e: &a::Expr) -> CG<Side> {
    match e {
        a::Expr::Paren(p) => operand(g, &p.expr),
        a::Expr::Bin(b) => match bin(g, b.op, &b.left, &b.right)? {
            Some(s) => Ok(s),
            None => Ok(Side::L(Leaf::Ex(expr::compile(g, e)?))),
        },
        a::Expr::Ident(i) => Ok(Side::L(match g.resolve(&i.sym) {
            Some(l) => Leaf::Slot(l.slot),
            None => Leaf::Ex(expr::compile(g, e)?),
        })),
        a::Expr::Lit(l) => Ok(Side::L(Leaf::Const(expr::lit_value(l)?))),
        a::Expr::Member(m) => match elem::compile(g, m)? {
            Some(el) => Ok(Side::L(Leaf::Elem(Box::new(el)))),
            None => Ok(Side::L(Leaf::Ex(expr::compile(g, e)?))),
        },
        _ => Ok(Side::L(Leaf::Ex(expr::compile(g, e)?))),
    }
}

fn is_num(s: &Side) -> bool {
    match s {
        Side::N(_) => true,
        Side::L(Leaf::Const(Value::Number(_))) => true,
        Side::L(_) => false,
    }
}

/// A binary expression, or `None` for the operators this module does not specialise (`&&`, `||`, `??`, `**`,
/// `in`, `instanceof`). Compiles the operands only when it returns `Some`.
pub(crate) fn bin(g: &mut Gen, op: B, left: &a::Expr, right: &a::Expr) -> CG<Option<Side>> {
    if is_bit(op) {
        let l = operand(g, left)?;
        let r = operand(g, right)?;
        return Ok(Some(
            if op == B::ZeroFillRShift || is_num(&l) || is_num(&r) {
                Side::N(bit_nm(op, l, r))
            } else {
                #[cfg(test)]
                {
                    g.bit_ex_built += 1;
                }
                Side::L(Leaf::Ex(bit_ex(op, l, r)))
            },
        ));
    }
    if !is_spec(op) {
        return Ok(None);
    }
    let l = leaf_of(operand(g, left)?);
    let r = leaf_of(operand(g, right)?);
    Ok(Some(Side::L(Leaf::Ex(spec(op, l, r)))))
}

/// `number op number` for `+ - * / %` and the comparisons. The closures are monomorphic per operator.
pub(crate) fn spec(op: B, l: Leaf, r: Leaf) -> Ex {
    match op {
        B::Add => fast_ex(op, l, r, |x, y| Value::Number(x + y)),
        B::Sub => fast_ex(op, l, r, |x, y| Value::Number(x - y)),
        B::Mul => fast_ex(op, l, r, |x, y| Value::Number(x * y)),
        B::Div => fast_ex(op, l, r, |x, y| Value::Number(x / y)),
        B::Mod => fast_ex(op, l, r, |x, y| Value::Number(x % y)),
        B::Lt => fast_ex(op, l, r, |x, y| Value::Bool(x < y)),
        B::LtEq => fast_ex(op, l, r, |x, y| Value::Bool(x <= y)),
        B::Gt => fast_ex(op, l, r, |x, y| Value::Bool(x > y)),
        B::GtEq => fast_ex(op, l, r, |x, y| Value::Bool(x >= y)),
        B::EqEqEq | B::EqEq => fast_ex(op, l, r, |x, y| Value::Bool(x == y)),
        B::NotEqEq | B::NotEq => fast_ex(op, l, r, |x, y| Value::Bool(x != y)),
        _ => slow_ex(op, Side::L(l), Side::L(r)),
    }
}

fn fast_ex<F>(op: B, l: Leaf, r: Leaf, fast: F) -> Ex
where
    F: Fn(f64, f64) -> Value + 'static,
{
    mk_ex(move |it, f| {
        let lv = leaf_val(&l, it, f)?;
        let rv = leaf_val(&r, it, f)?;
        if let (Value::Number(x), Value::Number(y)) = (&lv, &rv) {
            return Ok(fast(*x, *y));
        }
        bx(combine(it, op, &lv, &rv))
    })
}

/// The generic operator node: both operands, then `combine`.
fn slow_ex(op: B, l: Side, r: Side) -> Ex {
    mk_ex(move |it, f| {
        let lv = side_val(&l, it, f)?;
        let rv = side_val(&r, it, f)?;
        bx(combine(it, op, &lv, &rv))
    })
}

/// The Number result of a bitwise operator: `combine` gives a Number, or throws (see `Nm`).
fn num_of(it: &mut Interp, op: B, lv: &Value, rv: &Value) -> CR<f64> {
    match bx(combine(it, op, lv, rv))? {
        Value::Number(n) => Ok(n),
        // unreachable: the operands are a Number and a BigInt (mixing throws) or a BigInt-only `>>>` (throws)
        _ => bx(it.throw_error(
            "TypeError",
            "Cannot mix BigInt and other types, use explicit conversions",
        )),
    }
}

fn nm_op<F>(op: B, l: Side, r: Side, fast: F) -> Nm
where
    F: Fn(f64, f64) -> f64 + 'static,
{
    Box::new(move |it: &mut Interp, f: &mut Frame| {
        let lv = l.eval_n(it, f)?;
        let rv = r.eval_n(it, f)?;
        if let (Num2::N(x), Num2::N(y)) = (&lv, &rv) {
            it.cstats.note_nm_fast();
            return Ok(fast(*x, *y));
        }
        num_of(it, op, &num2_val(lv), &num2_val(rv))
    })
}

fn slow_nm(op: B, l: Side, r: Side) -> Nm {
    Box::new(move |it: &mut Interp, f: &mut Frame| {
        let lv = l.eval_n(it, f)?;
        let rv = r.eval_n(it, f)?;
        num_of(it, op, &num2_val(lv), &num2_val(rv))
    })
}

/// A bitwise operator whose operands are both non-literal: the Number-Number pair takes the `number_binop`
/// expression, anything else `combine` on the evaluated values (each child runs once, left first).
fn bit_ex(op: B, l: Side, r: Side) -> Ex {
    fn with<F>(op: B, l: Side, r: Side, fast: F) -> Ex
    where
        F: Fn(f64, f64) -> f64 + 'static,
    {
        mk_ex(move |it, f| {
            let lv = l.eval_n(it, f)?;
            let rv = r.eval_n(it, f)?;
            if let (Num2::N(x), Num2::N(y)) = (&lv, &rv) {
                it.cstats.note_nm_fast();
                return Ok(Value::Number(fast(*x, *y)));
            }
            bx(combine(it, op, &num2_val(lv), &num2_val(rv)))
        })
    }
    match op {
        B::BitAnd => with(op, l, r, |x, y| (to_int32(x) & to_int32(y)) as f64),
        B::BitOr => with(op, l, r, |x, y| (to_int32(x) | to_int32(y)) as f64),
        B::BitXor => with(op, l, r, |x, y| (to_int32(x) ^ to_int32(y)) as f64),
        B::LShift => with(op, l, r, |x, y| {
            to_int32(x).wrapping_shl(to_int32(y) as u32 & 31) as f64
        }),
        B::RShift => with(op, l, r, |x, y| {
            (to_int32(x) >> (to_int32(y) as u32 & 31)) as f64
        }),
        B::ZeroFillRShift => with(op, l, r, |x, y| {
            ((to_int32(x) as u32) >> (to_int32(y) as u32 & 31)) as f64
        }),
        _ => slow_ex(op, l, r),
    }
}

/// `x = e` on a mutable slot whose right-hand side is a Number by construction: the slot is written in place.
pub(crate) fn store_slot(k: u32, n: Nm) -> Ex {
    mk_ex(move |it, f| {
        let x = n(it, f)?;
        set_num(&mut f.slots[k as usize], x);
        Ok(Value::Number(x))
    })
}

/// A bitwise or shift operator as a number-valued node. The fast closures are the `number_binop` expressions.
pub(crate) fn bit_nm(op: B, l: Side, r: Side) -> Nm {
    match op {
        B::BitAnd => nm_op(op, l, r, |x, y| (to_int32(x) & to_int32(y)) as f64),
        B::BitOr => nm_op(op, l, r, |x, y| (to_int32(x) | to_int32(y)) as f64),
        B::BitXor => nm_op(op, l, r, |x, y| (to_int32(x) ^ to_int32(y)) as f64),
        B::LShift => nm_op(op, l, r, |x, y| {
            to_int32(x).wrapping_shl(to_int32(y) as u32 & 31) as f64
        }),
        B::RShift => nm_op(op, l, r, |x, y| {
            (to_int32(x) >> (to_int32(y) as u32 & 31)) as f64
        }),
        B::ZeroFillRShift => nm_op(op, l, r, |x, y| {
            ((to_int32(x) as u32) >> (to_int32(y) as u32 & 31)) as f64
        }),
        _ => slow_nm(op, l, r),
    }
}

/// A condition from a comparison, with the same fast path as `spec`.
fn fast_bl<F>(op: B, l: Leaf, r: Leaf, fast: F) -> Bl
where
    F: Fn(f64, f64) -> bool + 'static,
{
    Box::new(move |it: &mut Interp, f: &mut Frame| {
        let lv = leaf_val(&l, it, f)?;
        let rv = leaf_val(&r, it, f)?;
        if let (Value::Number(x), Value::Number(y)) = (&lv, &rv) {
            return Ok(fast(*x, *y));
        }
        Ok(bx(combine(it, op, &lv, &rv))?.truthy())
    })
}

fn cmp_bl(op: B, l: Leaf, r: Leaf) -> Bl {
    match op {
        B::Lt => fast_bl(op, l, r, |x, y| x < y),
        B::LtEq => fast_bl(op, l, r, |x, y| x <= y),
        B::Gt => fast_bl(op, l, r, |x, y| x > y),
        B::GtEq => fast_bl(op, l, r, |x, y| x >= y),
        B::EqEqEq | B::EqEq => fast_bl(op, l, r, |x, y| x == y),
        B::NotEqEq | B::NotEq => fast_bl(op, l, r, |x, y| x != y),
        _ => Box::new(move |it: &mut Interp, f: &mut Frame| {
            let lv = leaf_val(&l, it, f)?;
            let rv = leaf_val(&r, it, f)?;
            Ok(bx(combine(it, op, &lv, &rv))?.truthy())
        }),
    }
}

/// A condition (`if`, loops, `?:`). `!`, `&&` and `||` recurse; comparisons skip the `Value::Bool`.
pub(crate) fn test(g: &mut Gen, e: &a::Expr) -> CG<Bl> {
    match e {
        a::Expr::Paren(p) => test(g, &p.expr),
        a::Expr::Unary(u) if matches!(u.op, a::UnaryOp::Bang) => {
            let x = test(g, &u.arg)?;
            Ok(Box::new(move |it: &mut Interp, f: &mut Frame| {
                Ok(!x(it, f)?)
            }))
        }
        a::Expr::Bin(b) if matches!(b.op, B::LogicalAnd | B::LogicalOr) => {
            // `a && b` is true exactly when both are; the right side runs only when the left one does not decide
            let and = matches!(b.op, B::LogicalAnd);
            let l = test(g, &b.left)?;
            let r = test(g, &b.right)?;
            Ok(Box::new(move |it: &mut Interp, f: &mut Frame| {
                let lv = l(it, f)?;
                if lv == and {
                    r(it, f)
                } else {
                    Ok(lv)
                }
            }))
        }
        a::Expr::Bin(b) if is_cmp(b.op) => {
            let l = leaf_of(operand(g, &b.left)?);
            let r = leaf_of(operand(g, &b.right)?);
            Ok(cmp_bl(b.op, l, r))
        }
        _ => {
            let x = expr::compile(g, e)?;
            Ok(Box::new(move |it: &mut Interp, f: &mut Frame| {
                Ok(x(it, f)?.truthy())
            }))
        }
    }
}

fn is_cmp(op: B) -> bool {
    matches!(
        op,
        B::Lt | B::LtEq | B::Gt | B::GtEq | B::EqEqEq | B::NotEqEq | B::EqEq | B::NotEq
    )
}

/// An expression statement, or a `for` update: `x++` / `x--` on a mutable slot of Number value is done inline.
/// The value is discarded by both callers, so the fast path returns `undefined`.
pub(crate) fn discard(g: &mut Gen, e: &a::Expr) -> CG<Ex> {
    if let a::Expr::Update(u) = e {
        if let a::Expr::Ident(i) = &*u.arg {
            if let Some(l) = g.resolve(&i.sym).filter(|l| l.mutable) {
                let k = l.slot as usize;
                let inc = matches!(u.op, a::UpdateOp::PlusPlus);
                let slow = expr::compile(g, e)?;
                return Ok(mk_ex(move |it, f| match f.slots[k] {
                    Value::Number(n) => {
                        set_num(&mut f.slots[k], if inc { n + 1.0 } else { n - 1.0 });
                        Ok(Value::Undefined)
                    }
                    _ => slow(it, f),
                }));
            }
        }
    }
    expr::compile(g, e)
}

/// `x op= e` on a mutable slot. Numbers take the inline operator, everything else the shared `compound_finish`.
/// `cur` is read before the right-hand side, as in `compound`.
pub(crate) fn compound_slot(k: u32, op: B, rhs: Side) -> Ex {
    match op {
        B::Add => fused(op, k, rhs, |x, y| x + y),
        B::Sub => fused(op, k, rhs, |x, y| x - y),
        B::Mul => fused(op, k, rhs, |x, y| x * y),
        B::Div => fused(op, k, rhs, |x, y| x / y),
        B::Mod => fused(op, k, rhs, |x, y| x % y),
        B::BitAnd => fused(op, k, rhs, |x, y| (to_int32(x) & to_int32(y)) as f64),
        B::BitOr => fused(op, k, rhs, |x, y| (to_int32(x) | to_int32(y)) as f64),
        B::BitXor => fused(op, k, rhs, |x, y| (to_int32(x) ^ to_int32(y)) as f64),
        B::LShift => fused(op, k, rhs, |x, y| {
            to_int32(x).wrapping_shl(to_int32(y) as u32 & 31) as f64
        }),
        B::RShift => fused(op, k, rhs, |x, y| {
            (to_int32(x) >> (to_int32(y) as u32 & 31)) as f64
        }),
        B::ZeroFillRShift => fused(op, k, rhs, |x, y| {
            ((to_int32(x) as u32) >> (to_int32(y) as u32 & 31)) as f64
        }),
        _ => expr::compound(Place::Slot { k, mutable: true }, op, side_ex(rhs)),
    }
}

/// `x op= e` on a slot. The old value is read before the right-hand side (`n += (n = 5)` keeps the old `n`);
/// a Number old value with a Number right-hand side is written in place.
fn fused<F>(op: B, k: u32, rhs: Side, fast: F) -> Ex
where
    F: Fn(f64, f64) -> f64 + 'static,
{
    let p = Place::Slot { k, mutable: true };
    mk_ex(move |it, f| {
        if let Value::Number(x) = f.slots[k as usize] {
            let r = rhs.eval_n(it, f)?;
            if let Num2::N(y) = r {
                it.cstats.note_nm_fast();
                let v = fast(x, y);
                set_num(&mut f.slots[k as usize], v);
                return Ok(Value::Number(v));
            }
            return expr::compound_finish(it, f, &p, op, Value::Number(x), num2_val(r));
        }
        let cur = f.slots[k as usize].clone();
        let r = rhs.eval_n(it, f)?;
        expr::compound_finish(it, f, &p, op, cur, num2_val(r))
    })
}
