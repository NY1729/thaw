//! Binding patterns (declarations and parameters), compiled. Each node does what `Interp::bind_pattern` does for
//! the same pattern, in the same order: the value is checked, the items or properties are read, and the targets
//! are bound left to right (a default runs when the read value is `undefined`).

use std::rc::Rc;

use thaw_parser::ast as a;

use super::expr;
use super::gen::{reject, Gen, CG};
use super::{bx, Ex, Frame, CR};
use crate::{put, Interp, Value};

/// A compiled binder: stores the value into the pattern's targets.
pub(crate) type Bind = Box<dyn Fn(&mut Interp, &mut Frame, Value) -> CR<()>>;

/// What the pattern's names are: block `let`/`const` (a fresh slot each), hoisted `var` (assigned) or a
/// parameter (declared by the prologue, then assigned).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Let,
    Const,
    Var,
    Param,
}

pub(crate) fn target(g: &mut Gen, name: &str, mode: Mode) -> CG<Bind> {
    match mode {
        Mode::Let | Mode::Const => {
            if g.is_scoped(name) {
                // a captured `let`/`const` at the top of the body: declared in the call's scope, as the
                // interpreter does when the declaration runs
                if !g.at_fn_top() {
                    return reject();
                }
                let name: Rc<str> = name.into();
                let mutable = mode == Mode::Let;
                return Ok(Box::new(move |it, f, v| {
                    let n = it.intern(&name);
                    f.scope.declare_rc(n, v, mutable);
                    Ok(())
                }));
            }
            let k = g.declare_block(name, mode == Mode::Let);
            Ok(Box::new(move |_, f, v| {
                f.slots[k as usize] = v;
                Ok(())
            }))
        }
        Mode::Var | Mode::Param => match g.var_local(name) {
            Some(l) => {
                let k = l.slot;
                Ok(Box::new(move |_, f, v| {
                    f.slots[k as usize] = v;
                    Ok(())
                }))
            }
            None if g.is_scoped(name) => {
                let name: Rc<str> = name.into();
                let var = mode == Mode::Var;
                Ok(Box::new(move |it, f, v| {
                    let strict = var && f.scope.strict.get();
                    bx(it.assign_var(&f.scope, &name, v, strict))
                }))
            }
            None => reject(),
        },
    }
}

/// `default` applied when the value is `undefined`, then `inner`.
fn with_default(default: Ex, inner: Bind) -> Bind {
    Box::new(move |it, f, v| {
        let v = if matches!(v, Value::Undefined) {
            default(it, f)?
        } else {
            v
        };
        inner(it, f, v)
    })
}

enum Key {
    Name(Rc<str>),
    Computed(Ex),
}

pub(crate) fn compile(g: &mut Gen, p: &a::Pat, mode: Mode) -> CG<Bind> {
    match p {
        a::Pat::Ident(b) => target(g, &b.id.sym, mode),
        a::Pat::Assign(d) => {
            let default = expr::compile(g, &d.right)?;
            let default = match &*d.left {
                a::Pat::Ident(id) => expr::named(&d.right, &id.id.sym, default),
                _ => default,
            };
            let inner = compile(g, &d.left, mode)?;
            Ok(with_default(default, inner))
        }
        a::Pat::Array(ap) => array(g, ap, mode),
        a::Pat::Object(op) => object(g, op, mode),
        _ => reject(),
    }
}

fn array(g: &mut Gen, ap: &a::ArrayPat, mode: Mode) -> CG<Bind> {
    let has_rest = ap.elems.iter().any(|e| matches!(e, Some(a::Pat::Rest(_))));
    let wanted = if has_rest { None } else { Some(ap.elems.len()) };
    // (is rest, binder) per element; a hole binds nothing
    let mut elems: Vec<Option<(bool, Bind)>> = Vec::with_capacity(ap.elems.len());
    for el in &ap.elems {
        elems.push(match el {
            Some(a::Pat::Rest(r)) => Some((true, compile(g, &r.arg, mode)?)),
            Some(p) => Some((false, compile(g, p, mode)?)),
            None => None,
        });
    }
    Ok(Box::new(move |it, f, value| {
        if !matches!(value, Value::Object(_) | Value::Str(_)) {
            return bx(it.throw_error("TypeError", "value is not iterable"));
        }
        let Some(items) = bx(it.iter_protocol_limit(&value, wanted))? else {
            return bx(it.throw_error("TypeError", "value is not iterable"));
        };
        for (i, el) in elems.iter().enumerate() {
            match el {
                Some((true, b)) => {
                    let rest = it.array(items.get(i..).unwrap_or(&[]).to_vec());
                    b(it, f, rest)?;
                }
                Some((false, b)) => {
                    b(it, f, items.get(i).cloned().unwrap_or(Value::Undefined))?;
                }
                None => {}
            }
        }
        Ok(())
    }))
}

enum Prop {
    /// `key: pattern` (the pattern may carry a default), or a shorthand `key = default` already wrapped
    Get(Key, Option<Ex>, Bind),
    Rest(Bind),
}

fn object(g: &mut Gen, op: &a::ObjectPat, mode: Mode) -> CG<Bind> {
    let mut props: Vec<Prop> = Vec::with_capacity(op.props.len());
    let mut has_rest = false;
    for p in &op.props {
        props.push(match p {
            a::ObjectPatProp::KeyValue(kv) => {
                let key = match &kv.key {
                    a::PropName::Ident(i) => Key::Name(Rc::from(i.sym.as_ref())),
                    a::PropName::Str(s) => {
                        Key::Name(Rc::from(crate::builtins::wtf8_to_str(&s.value)))
                    }
                    a::PropName::Num(n) => {
                        Key::Name(Rc::from(crate::number_to_string(n.value).as_str()))
                    }
                    a::PropName::Computed(c) => Key::Computed(expr::compile(g, &c.expr)?),
                    _ => return reject(),
                };
                Prop::Get(key, None, compile(g, &kv.value, mode)?)
            }
            a::ObjectPatProp::Assign(ap) => {
                let k: Rc<str> = Rc::from(ap.key.id.sym.as_ref());
                let default = match &ap.value {
                    Some(d) => Some(expr::named(d, &k, expr::compile(g, d)?)),
                    None => None,
                };
                Prop::Get(Key::Name(k.clone()), default, target(g, &k, mode)?)
            }
            a::ObjectPatProp::Rest(r) => {
                has_rest = true;
                Prop::Rest(compile(g, &r.arg, mode)?)
            }
        });
    }
    Ok(Box::new(move |it, f, value| {
        if matches!(value, Value::Undefined | Value::Null) {
            return bx(it.throw_error("TypeError", "Cannot destructure a nullish value"));
        }
        let mut used: Vec<Rc<str>> = Vec::new();
        for p in &props {
            match p {
                Prop::Get(key, default, bind) => {
                    let k = match key {
                        Key::Name(n) => n.clone(),
                        Key::Computed(e) => {
                            let kv = e(it, f)?;
                            it.key_of(&kv)
                        }
                    };
                    let mut v = bx(it.get(&value, &k))?;
                    if let (Some(d), Value::Undefined) = (default, &v) {
                        v = d(it, f)?;
                    }
                    if has_rest {
                        used.push(k);
                    }
                    bind(it, f, v)?;
                }
                Prop::Rest(bind) => {
                    let rest = it.plain();
                    let keys = it
                        .own_keys(&value)
                        .into_iter()
                        .chain(it.own_enumerable_symbol_keys(&value));
                    for k in keys {
                        if !used.contains(&k) {
                            let v = bx(it.get(&value, &k))?;
                            put(&rest, &k, v);
                        }
                    }
                    bind(it, f, Value::Object(rest))?;
                }
            }
        }
        Ok(())
    }))
}
