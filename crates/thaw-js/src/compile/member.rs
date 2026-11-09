//! Member access and array/object literals (plan TJ150 section 7, PR4). Each node calls the helper the AST arm
//! calls (`get`, `get_index`, `set`, `set_index`, `key_of`, `put`, `array`, `plain`), in the same order: the
//! object, then the key, then the right-hand side (`member_loc`, `eval_assign`, `Expr::Update`).

use std::rc::Rc;

use thaw_parser::ast as a;

use super::elem;
use super::expr::{self, combine, mk_ex};
use super::gen::{reject, Gen, CG};
use super::num::{self, Side};
use super::{bx, Ex, Frame, CR};
use crate::{bigint, put, Interp, Value, R};

/// The property of a member expression or literal, decided at compile time.
pub(crate) enum PropKey {
    Name(Rc<str>),
    Computed(Ex),
    /// A computed key that is a Number by construction (a bitwise node): no `Value` is built for it.
    Num(num::Nm),
}

/// A located property: an integer index (kept numeric, so `arr[i]` builds no key) or a string key.
enum MLoc {
    Index(Value, usize),
    Prop(Value, Rc<str>),
}

pub(crate) fn prop_key(g: &mut Gen, p: &a::MemberProp) -> CG<PropKey> {
    match p {
        a::MemberProp::Ident(i) => Ok(PropKey::Name(Rc::from(i.sym.as_ref()))),
        a::MemberProp::Computed(c) => Ok(match num::operand(g, &c.expr)? {
            Side::N(n) => PropKey::Num(n),
            side => PropKey::Computed(num::side_ex(side)),
        }),
        a::MemberProp::PrivateName(_) => reject(),
    }
}

/// `member_loc`: the object is already evaluated, then the key. A non-negative integer Number stays numeric.
fn locate(it: &mut Interp, f: &mut Frame, obj: Value, key: &PropKey) -> CR<MLoc> {
    match key {
        PropKey::Name(n) => Ok(MLoc::Prop(obj, n.clone())),
        PropKey::Computed(e) => {
            let kv = e(it, f)?;
            if let Value::Number(n) = &kv {
                // range first (rejects NaN), then an exact-integer round trip
                if (0.0..4294967295.0).contains(n) && (*n as u32) as f64 == *n {
                    return Ok(MLoc::Index(obj, *n as usize));
                }
            }
            Ok(MLoc::Prop(obj, it.key_of(&kv)))
        }
        // the same decision as the `Value::Number` arm above
        PropKey::Num(e) => {
            let n = e(it, f)?;
            if (0.0..4294967295.0).contains(&n) && (n as u32) as f64 == n {
                return Ok(MLoc::Index(obj, n as usize));
            }
            Ok(MLoc::Prop(obj, it.key_of(&Value::Number(n))))
        }
    }
}

/// The key of a callee `o.p` / `o[e]` as the AST call arm builds it (`member_key`): a computed key goes through
/// `key_of` only, with no integer shortcut.
pub(crate) fn callee_key(it: &mut Interp, f: &mut Frame, key: &PropKey) -> CR<Rc<str>> {
    match key {
        PropKey::Name(n) => Ok(n.clone()),
        PropKey::Computed(e) => {
            let v = e(it, f)?;
            Ok(it.key_of(&v))
        }
        PropKey::Num(e) => {
            let n = e(it, f)?;
            Ok(it.key_of(&Value::Number(n)))
        }
    }
}

/// `read_loc`
fn load(it: &mut Interp, loc: &MLoc) -> R<Value> {
    match loc {
        MLoc::Index(o, i) => it.get_index(o, *i),
        MLoc::Prop(o, k) => it.get(o, k),
    }
}

/// `write_loc`
fn store(it: &mut Interp, loc: &MLoc, v: Value) -> R<()> {
    match loc {
        MLoc::Index(o, i) => it.set_index(o, *i, v),
        MLoc::Prop(o, k) => it.set(o, k, v),
    }
}

/// `o.p` / `o[e]` as a value.
pub(crate) fn read(g: &mut Gen, m: &a::MemberExpr) -> CG<Ex> {
    if let Some(el) = elem::compile(g, m)? {
        return Ok(elem::ex(el));
    }
    let obj = expr::compile(g, &m.obj)?;
    let key = prop_key(g, &m.prop)?;
    Ok(mk_ex(move |it, f| {
        let o = obj(it, f)?;
        let loc = locate(it, f, o, &key)?;
        bx(load(it, &loc))
    }))
}

/// The operator of a member assignment, decided at compile time.
#[derive(Clone, Copy)]
enum Rmw {
    Set,
    Logic(a::AssignOp),
    Arith(a::BinaryOp),
}

/// `o.p = e`, `o[i] op= e`, `o.p &&= e`. The value of the assignment is the stored value.
pub(crate) fn assign(g: &mut Gen, m: &a::MemberExpr, asg: &a::AssignExpr) -> CG<Ex> {
    use a::AssignOp as A;
    let rmw = match asg.op {
        A::Assign => Rmw::Set,
        op @ (A::AndAssign | A::OrAssign | A::NullishAssign) => Rmw::Logic(op),
        op => match crate::compound_binop(op) {
            Some(b) => Rmw::Arith(b),
            None => return reject(),
        },
    };
    let obj = expr::compile(g, &m.obj)?;
    let key = prop_key(g, &m.prop)?;
    let rhs = expr::compile(g, &asg.right)?;
    Ok(mk_ex(move |it, f| {
        let o = obj(it, f)?;
        let loc = locate(it, f, o, &key)?;
        let v = match rmw {
            Rmw::Set => rhs(it, f)?,
            Rmw::Logic(op) => {
                let cur = bx(load(it, &loc))?;
                let keep = match op {
                    A::AndAssign => !cur.truthy(),
                    A::OrAssign => cur.truthy(),
                    _ => !matches!(cur, Value::Undefined | Value::Null),
                };
                if keep {
                    return Ok(cur);
                }
                rhs(it, f)?
            }
            Rmw::Arith(bop) => {
                let cur = bx(load(it, &loc))?;
                let r = rhs(it, f)?;
                bx(combine(it, bop, &cur, &r))?
            }
        };
        bx(store(it, &loc, v.clone()))?;
        Ok(v)
    }))
}

/// `++o.p` / `o[i]--` and friends (the `Expr::Update` arm for a member target).
pub(crate) fn update(g: &mut Gen, m: &a::MemberExpr, inc: bool, prefix: bool) -> CG<Ex> {
    let obj = expr::compile(g, &m.obj)?;
    let key = prop_key(g, &m.prop)?;
    Ok(mk_ex(move |it, f| {
        let o = obj(it, f)?;
        let loc = locate(it, f, o, &key)?;
        let old = bx(load(it, &loc))?;
        if let Value::BigInt(b) = &old {
            let one = num_bigint::BigInt::from(1);
            let new = bigint::big(if inc { &**b + one } else { &**b - one });
            bx(store(it, &loc, new.clone()))?;
            return Ok(if prefix { new } else { old.clone() });
        }
        let n = it.num(&old);
        let new = if inc { n + 1.0 } else { n - 1.0 };
        bx(store(it, &loc, Value::Number(new)))?;
        Ok(Value::Number(if prefix { new } else { n }))
    }))
}

/// `[a, , b]`: no spread (the analyzer blocks it), holes become `Value::Hole`.
pub(crate) fn array(g: &mut Gen, arr: &a::ArrayLit) -> CG<Ex> {
    let mut items: Vec<Option<Ex>> = Vec::with_capacity(arr.elems.len());
    for el in &arr.elems {
        items.push(match el {
            Some(el) if el.spread.is_some() => return reject(),
            Some(el) => Some(expr::compile(g, &el.expr)?),
            None => None,
        });
    }
    Ok(mk_ex(move |it, f| {
        let mut vals = Vec::with_capacity(items.len());
        for x in &items {
            vals.push(match x {
                Some(x) => x(it, f)?,
                None => Value::Hole,
            });
        }
        Ok(it.array(vals))
    }))
}

fn prop_name(g: &mut Gen, k: &a::PropName) -> CG<PropKey> {
    Ok(match k {
        a::PropName::Ident(i) => PropKey::Name(Rc::from(i.sym.as_ref())),
        a::PropName::Str(s) => PropKey::Name(Rc::from(s.value.as_str().unwrap_or(""))),
        a::PropName::Num(n) => PropKey::Name(Rc::from(crate::number_to_string(n.value).as_str())),
        a::PropName::Computed(c) => PropKey::Computed(expr::compile(g, &c.expr)?),
        _ => return reject(),
    })
}

/// `{ k: v, s, [c]: v }`: key then value, each in source order, then `put` (no methods, accessors or spread).
pub(crate) fn object(g: &mut Gen, o: &a::ObjectLit) -> CG<Ex> {
    let mut fields: Vec<(PropKey, Ex)> = Vec::with_capacity(o.props.len());
    for p in &o.props {
        let a::PropOrSpread::Prop(p) = p else {
            return reject();
        };
        let field = match &**p {
            a::Prop::KeyValue(kv) => (prop_name(g, &kv.key)?, expr::compile(g, &kv.value)?),
            a::Prop::Shorthand(i) => (
                PropKey::Name(Rc::from(i.sym.as_ref())),
                expr::compile(g, &a::Expr::Ident(i.clone()))?,
            ),
            _ => return reject(),
        };
        fields.push(field);
    }
    Ok(mk_ex(move |it, f| {
        let obj = it.plain();
        for (k, v) in &fields {
            let k = match k {
                PropKey::Name(n) => n.clone(),
                PropKey::Computed(e) => {
                    let kv = e(it, f)?;
                    it.key_of(&kv)
                }
                PropKey::Num(e) => {
                    let n = e(it, f)?;
                    it.key_of(&Value::Number(n))
                }
            };
            let v = v(it, f)?;
            put(&obj, &k, v);
        }
        Ok(Value::Object(obj))
    }))
}
