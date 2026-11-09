//! Element reads `s[k]` where `s` is a slot and `k` is a pure key (plan TJ153 section 3.2, C4 PR2). The key is
//! evaluated first; the base slot is read after it, which is the interpreter order because a pure key cannot write
//! a slot of this frame. A Number element of an Array or a typed array (an in-range integer index) is returned
//! without cloning the base. Every other case calls the interpreter helper the AST arm calls (`get_index`, `get`,
//! `key_of`), with the same key decision as `member::locate`.

use thaw_parser::ast as a;

use super::expr::mk_ex;
use super::gen::{Gen, CG};
use super::num::{self, num2_val, Num2, Side};
use super::{bx, Ex, Frame, CR};
use crate::{Interp, Kind, Value};

/// `s[k]` with `s` in a slot and a pure key.
pub(crate) struct Elem {
    base: u32,
    key: Side,
}

/// The gate: `Some` only for `slot[pure key]`. Nothing is compiled when it returns `None`, so the caller can try
/// another shape (or fall back to `expr::compile`).
pub(crate) fn compile(g: &mut Gen, m: &a::MemberExpr) -> CG<Option<Elem>> {
    let a::MemberProp::Computed(c) = &m.prop else {
        return Ok(None);
    };
    let mut obj: &a::Expr = &m.obj;
    while let a::Expr::Paren(p) = obj {
        obj = &p.expr;
    }
    let a::Expr::Ident(i) = obj else {
        return Ok(None);
    };
    let Some(l) = g.resolve(&i.sym) else {
        return Ok(None);
    };
    if !pure(&c.expr) {
        return Ok(None);
    }
    #[cfg(test)]
    {
        g.elem_built += 1;
    }
    let key = num::operand(g, &c.expr)?;
    Ok(Some(Elem { base: l.slot, key }))
}

/// Default-deny whitelist of keys that cannot write a slot of this frame. Assignments, updates, calls, `new`,
/// sequences and templates are rejected. Inner functions and `arguments` are rejected by the analyzer, so no
/// other code reaches the frame's slots.
fn pure(e: &a::Expr) -> bool {
    match e {
        a::Expr::Ident(_) | a::Expr::Lit(_) => true,
        a::Expr::Paren(p) => pure(&p.expr),
        a::Expr::Bin(b) => pure(&b.left) && pure(&b.right),
        a::Expr::Unary(u) => pure(&u.arg),
        a::Expr::Member(m) => {
            pure(&m.obj)
                && match &m.prop {
                    a::MemberProp::Ident(_) => true,
                    a::MemberProp::Computed(c) => pure(&c.expr),
                    a::MemberProp::PrivateName(_) => false,
                }
        }
        _ => false,
    }
}

/// The same range test `member::locate` uses: NaN fails it, `-0` passes it (index 0).
const LIMIT: f64 = 4294967295.0;

impl Elem {
    /// The operand: `N` for a Number element, otherwise the value the interpreter helper gives.
    pub(crate) fn eval_n(&self, it: &mut Interp, f: &mut Frame) -> CR<Num2> {
        let key = self.key.eval_n(it, f)?;
        if let Num2::N(n) = key {
            if let Some(x) = fast(f, self.base, n) {
                it.cstats.note_elem_fast();
                return Ok(Num2::N(x));
            }
        }
        Ok(match slow(it, f, self.base, key)? {
            Value::Number(x) => Num2::N(x),
            v => Num2::V(v),
        })
    }
}

/// The read as a plain member expression.
pub(crate) fn ex(e: Elem) -> Ex {
    mk_ex(move |it, f| Ok(num2_val(e.eval_n(it, f)?)))
}

/// A Number element of an Array, or an in-range element of a typed array. A hole, a non-Number element, an
/// out-of-range index or a non-index key returns `None` and takes the fallback. The borrow ends before return.
#[inline]
fn fast(f: &Frame, base: u32, n: f64) -> Option<f64> {
    if !((0.0..LIMIT).contains(&n) && (n as u32) as f64 == n) {
        return None;
    }
    let i = n as usize;
    let Value::Object(o) = &f.slots[base as usize] else {
        return None;
    };
    let x = match &*o.kind.borrow() {
        Kind::Array(items) => match items.get(i) {
            Some(Value::Number(x)) => Some(*x),
            _ => None,
        },
        Kind::Typed(t) => t.get(i),
        _ => None,
    };
    x
}

/// The fallback: the helper `member::load` calls for the same key decision as `member::locate`, on the base read
/// after the key. Cold, so the fast path stays small.
#[cold]
#[inline(never)]
fn slow(it: &mut Interp, f: &mut Frame, base: u32, key: Num2) -> CR<Value> {
    it.cstats.note_elem_slow();
    let o = f.slots[base as usize].clone();
    match key {
        Num2::N(n) if (0.0..LIMIT).contains(&n) && (n as u32) as f64 == n => {
            bx(it.get_index(&o, n as usize))
        }
        k => {
            let kv = num2_val(k);
            let key = it.key_of(&kv);
            bx(it.get(&o, &key))
        }
    }
}
