//! Element reads and writes `s[k]` where `s` is a slot and `k` is a pure key (plan TJ153 sections 3.2 and 3.4, C4 PR2
//! and PR3). The key is evaluated first; the base slot is read after it, which is the interpreter order because a pure
//! key cannot write a slot of this frame. A Number element of an Array or a typed array (an in-range integer index) is
//! read without cloning the base. Every other case calls the interpreter helper the AST arm calls (`get_index`, `get`,
//! `set_index`, `set`, `key_of`), with the same key decision as `member::locate`. Writes capture the base before the
//! key and the right-hand side, as `member::assign` does.

use std::rc::Rc;

use thaw_parser::ast as a;

use super::expr::{combine, mk_ex};
use super::gen::{Gen, CG};
use super::member::Rmw;
use super::num::{self, num2_val, Num2, Side};
use super::{bx, Ex, Frame, CR};
use crate::{bigint, Interp, Kind, Value};

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

/// The index of an in-range integer key, decided as `member::locate` decides it.
fn index(n: f64) -> Option<usize> {
    if (0.0..LIMIT).contains(&n) && (n as u32) as f64 == n {
        Some(n as usize)
    } else {
        None
    }
}

/// A located property, decided as `member::locate` does: an index stays numeric, any other key is converted by
/// `key_of` at once (before the right-hand side of a write runs, as in the interpreter).
enum Loc {
    Idx(usize),
    Key(Rc<str>),
}

fn to_loc(it: &mut Interp, key: Num2) -> Loc {
    match key {
        Num2::N(n) => match index(n) {
            Some(i) => Loc::Idx(i),
            None => Loc::Key(it.key_of(&Value::Number(n))),
        },
        Num2::V(v) => Loc::Key(it.key_of(&v)),
    }
}

impl Elem {
    /// The read: `N` for a Number element, otherwise the value the interpreter helper gives.
    pub(crate) fn eval_n(&self, it: &mut Interp, f: &mut Frame) -> CR<Num2> {
        let key = self.key.eval_n(it, f)?;
        if let Num2::N(n) = key {
            if let Some(x) = index(n).and_then(|i| fast_get(&f.slots[self.base as usize], i)) {
                it.cstats.note_elem_fast();
                return Ok(Num2::N(x));
            }
        }
        let base = f.slots[self.base as usize].clone();
        Ok(match read_slow(it, &base, key)? {
            Value::Number(x) => Num2::N(x),
            v => Num2::V(v),
        })
    }
}

/// The read as a plain member expression.
pub(crate) fn ex(e: Elem) -> Ex {
    mk_ex(move |it, f| Ok(num2_val(e.eval_n(it, f)?)))
}

/// A Number element of an Array, or an in-range element of a typed array. A hole, a non-Number element or an
/// out-of-range index returns `None` and takes the fallback. The borrow ends before return.
#[inline]
fn fast_get(v: &Value, i: usize) -> Option<f64> {
    let Value::Object(o) = v else {
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

/// The write `set_index` makes as a plain store: an existing non-hole element of an extensible, unfrozen,
/// length-writable Array, or an in-range element of an unfrozen typed array (`TypedArr::set` does the conversion).
/// Returns `false` for everything else, which takes `set_index` / `set`. `proxy_set` is a no-op for both kinds
/// (only `Kind::Proxy` objects have a trap), so skipping it changes nothing.
#[inline]
fn fast_set(v: &Value, i: usize, x: f64) -> bool {
    let Value::Object(o) = v else {
        return false;
    };
    if o.frozen.get() {
        return false;
    }
    let mut kind = o.kind.borrow_mut();
    match &mut *kind {
        Kind::Array(items) => {
            if o.ext.get() != 0 || o.len_ro.get() {
                return false;
            }
            match items.get_mut(i) {
                Some(slot) if !matches!(slot, Value::Hole) => {
                    *slot = Value::Number(x);
                    true
                }
                _ => false,
            }
        }
        Kind::Typed(t) if i < t.len => {
            t.set(i, x);
            true
        }
        _ => false,
    }
}

/// The read fallback: the helper `member::load` calls for the key decision of `member::locate`. Cold, so the fast
/// path stays small.
#[cold]
#[inline(never)]
fn read_slow(it: &mut Interp, base: &Value, key: Num2) -> CR<Value> {
    it.cstats.note_elem_slow();
    let loc = to_loc(it, key);
    get_at(it, base, &loc)
}

/// `read_loc` on a located property.
fn get_at(it: &mut Interp, base: &Value, loc: &Loc) -> CR<Value> {
    match loc {
        Loc::Idx(i) => bx(it.get_index(base, *i)),
        Loc::Key(k) => bx(it.get(base, k)),
    }
}

/// The current value of a located property, with the plain-element read first (compound and update targets).
fn get_loc(it: &mut Interp, base: &Value, loc: &Loc) -> CR<Value> {
    if let Loc::Idx(i) = loc {
        if let Some(x) = fast_get(base, *i) {
            return Ok(Value::Number(x));
        }
    }
    get_at(it, base, loc)
}

/// `write_loc`: a Number into a plain element takes `fast_set`; everything else is the helper the AST arm calls.
fn store(it: &mut Interp, base: &Value, loc: &Loc, v: Value) -> CR<()> {
    if let (Loc::Idx(i), Value::Number(x)) = (loc, &v) {
        if fast_set(base, *i, *x) {
            it.cstats.note_elem_store_fast();
            return Ok(());
        }
    }
    it.cstats.note_elem_store_slow();
    match loc {
        Loc::Idx(i) => bx(it.set_index(base, *i, v)),
        Loc::Key(k) => bx(it.set(base, k, v)),
    }
}

/// `s[k] = e`, `s[k] op= e`, `s[k] &&= e`, with the order of `member::assign`: the base, the key, then the
/// current value (compound), the right-hand side, the store. The value of the assignment is the stored value.
pub(crate) fn assign(el: Elem, rmw: Rmw, rhs: Ex) -> Ex {
    mk_ex(move |it, f| {
        let base = f.slots[el.base as usize].clone();
        let key = el.key.eval_n(it, f)?;
        let loc = to_loc(it, key);
        let v = match rmw {
            Rmw::Set => rhs(it, f)?,
            Rmw::Logic(op) => {
                let cur = get_loc(it, &base, &loc)?;
                let keep = match op {
                    a::AssignOp::AndAssign => !cur.truthy(),
                    a::AssignOp::OrAssign => cur.truthy(),
                    _ => !matches!(cur, Value::Undefined | Value::Null),
                };
                if keep {
                    return Ok(cur);
                }
                rhs(it, f)?
            }
            Rmw::Arith(bop) => {
                let cur = get_loc(it, &base, &loc)?;
                let r = rhs(it, f)?;
                bx(combine(it, bop, &cur, &r))?
            }
        };
        store(it, &base, &loc, v.clone())?;
        Ok(v)
    })
}

/// `s[k]++` / `++s[k]` / `s[k]--` / `--s[k]`, the same steps as `member::update`.
pub(crate) fn update(el: Elem, inc: bool, prefix: bool) -> Ex {
    mk_ex(move |it, f| {
        let base = f.slots[el.base as usize].clone();
        let key = el.key.eval_n(it, f)?;
        let loc = to_loc(it, key);
        let old = get_loc(it, &base, &loc)?;
        if let Value::BigInt(b) = &old {
            let one = num_bigint::BigInt::from(1);
            let new = bigint::big(if inc { &**b + one } else { &**b - one });
            store(it, &base, &loc, new.clone())?;
            return Ok(if prefix { new } else { old.clone() });
        }
        let n = it.num(&old);
        let new = if inc { n + 1.0 } else { n - 1.0 };
        store(it, &base, &loc, Value::Number(new))?;
        Ok(Value::Number(if prefix { new } else { n }))
    })
}
