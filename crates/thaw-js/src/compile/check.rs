//! `THAW_JS_COMPILE=check` oracle (plan TJ150 section 6). A pure compiled kernel runs twice: once on copies of its
//! arguments through the AST, once compiled. Results and post-call argument state must agree; otherwise the call
//! throws an `Error` naming the function (`mismatch_message`).

use std::rc::Rc;

use super::{CompileMode, CompiledFn};
use crate::{Ctl, FuncDef, Interp, Kind, Scope, Value, R};

/// The thrown message for a compiled-versus-AST mismatch: `function` is the name, `src` the position.
pub(crate) fn mismatch_message(name: &str, src: &str, detail: &str) -> String {
    format!("thaw-js compile check mismatch in {name} ({src}): {detail}")
}

// Test-only: when set, the compiled result is replaced by a constant, so the oracle must report a mismatch.
#[cfg(test)]
thread_local! {
    pub(crate) static CHECK_FAILPOINT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn failpoint(r: R<Value>) -> R<Value> {
    if CHECK_FAILPOINT.with(|c| c.get()) {
        r.map(|_| Value::Number(12345.0))
    } else {
        r
    }
}

/// A copy of an argument the check can compare: primitives as they are, arrays of primitives and typed arrays
/// copied. `None` for anything else (objects, functions, frozen or non-extensible objects): such calls are not checked.
fn snapshot(it: &mut Interp, v: &Value) -> Option<Value> {
    let Value::Object(o) = v else {
        return Some(v.clone());
    };
    // a frozen or non-extensible object (or one with a read-only length) behaves differently from a fresh copy
    if o.frozen.get() || o.ext.get() != 0 || o.len_ro.get() {
        return None;
    }
    if let Some(t) = Interp::typed_of(v) {
        let copy = it.typed_with_len(t.ty, t.len);
        let ct = Interp::typed_of(&copy)?;
        let n = t.len * t.ty.size();
        ct.data.borrow_mut()[..n].copy_from_slice(&t.data.borrow()[t.off..t.off + n]);
        return Some(copy);
    }
    let items = match &*o.kind.borrow() {
        Kind::Array(items) => items.clone(),
        _ => return None,
    };
    if items.iter().any(|x| matches!(x, Value::Object(_))) {
        return None;
    }
    Some(it.array(items))
}

/// The elements of an array or typed-array value (typed elements as numbers), `None` for anything else.
fn elems(v: &Value) -> Option<Vec<Value>> {
    let Value::Object(o) = v else {
        return None;
    };
    if let Some(t) = Interp::typed_of(v) {
        return Some(t.to_vec().into_iter().map(Value::Number).collect());
    }
    match &*o.kind.borrow() {
        Kind::Array(items) => Some(items.clone()),
        _ => None,
    }
}

/// 0 for a non-array object, 1 for an array, 2 for a typed array.
fn object_tag(v: &Value) -> u8 {
    if Interp::typed_of(v).is_some() {
        2
    } else if elems(v).is_some() {
        1
    } else {
        0
    }
}

/// `SameValue` for the values a snapshot can hold; objects compare by kind and elements.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            (x.is_nan() && y.is_nan()) || (x == y && x.is_sign_negative() == y.is_sign_negative())
        }
        (Value::Hole, Value::Hole) => true,
        (Value::Object(_), Value::Object(_)) => {
            let tag = object_tag(a);
            if tag != object_tag(b) {
                return false;
            }
            if tag == 0 {
                return true;
            }
            match (elems(a), elems(b)) {
                (Some(x), Some(y)) => {
                    x.len() == y.len() && x.iter().zip(&y).all(|(p, q)| same_value(p, q))
                }
                _ => false,
            }
        }
        _ => crate::strict_equals(a, b),
    }
}

/// The first element where a snapshot and the real argument differ, as `index j` or `length`.
fn arg_diff(snap: &Value, real: &Value) -> Option<String> {
    let (x, y) = (elems(snap)?, elems(real)?);
    if x.len() != y.len() {
        return Some("length".into());
    }
    x.iter()
        .zip(&y)
        .position(|(p, q)| !same_value(p, q))
        .map(|j| format!("index {j}"))
}

impl Interp {
    /// `check` mode for a pure kernel: the AST run on the argument copies and the compiled run on the real
    /// arguments must agree on the outcome and on each argument's final state.
    pub(crate) fn run_checked(
        &mut self,
        f: &Value,
        def: &Rc<FuncDef>,
        scope: &Rc<Scope>,
        this: &Value,
        args: &[Value],
        c: &CompiledFn,
    ) -> R<Value> {
        let copies: Option<Vec<Value>> = args.iter().map(|a| snapshot(self, a)).collect();
        let Some(copies) = copies else {
            return c.run(self, f, scope, this, args);
        };
        let ast = self.call_closure_ast(f, def, scope, this, &copies, None);
        let real = c.run(self, f, scope, this, args);
        #[cfg(test)]
        let real = failpoint(real);
        if self.dl_hit {
            // a deadline stopped the AST run part way: the two runs cannot be compared
            return real;
        }
        let detail = self.diff_detail(&ast, &real, args, &copies);
        match detail {
            None => real,
            Some(detail) => {
                let name = def.name.as_deref().unwrap_or("<anonymous>");
                let msg = mismatch_message(name, &format!("{}:{}", def.src.0, def.src.1), &detail);
                self.throw_error("Error", &msg)
            }
        }
    }

    /// What differs between the AST outcome and the compiled one, or `None` when nothing does.
    fn diff_detail(
        &mut self,
        ast: &R<Value>,
        real: &R<Value>,
        args: &[Value],
        copies: &[Value],
    ) -> Option<String> {
        let outcome = match (ast, real) {
            (Ok(a), Ok(b)) if same_value(a, b) => None,
            (Ok(_), Ok(_)) => Some("result".to_string()),
            (Err(Ctl::Throw(a)), Err(Ctl::Throw(b))) => {
                // Error objects compare by `name: message`; thrown primitives (`throw 1`) compare by value.
                let differs = match (a, b) {
                    (Value::Object(_), Value::Object(_)) => {
                        self.error_text(a) != self.error_text(b)
                    }
                    _ => !same_value(a, b),
                };
                differs.then(|| "error".to_string())
            }
            (Err(a), Err(b)) if std::mem::discriminant(a) == std::mem::discriminant(b) => None,
            _ => Some("error".to_string()),
        };
        if outcome.is_some() {
            return outcome;
        }
        args.iter()
            .zip(copies)
            .enumerate()
            .find_map(|(i, (real, snap))| arg_diff(snap, real).map(|d| format!("arg {i} {d}")))
    }

    /// `name: message` of a thrown error value, for comparing two thrown errors.
    fn error_text(&mut self, v: &Value) -> String {
        let name = self.get(v, "name").unwrap_or(Value::Undefined);
        let msg = self.get(v, "message").unwrap_or(Value::Undefined);
        format!("{}: {}", self.to_str(&name), self.to_str(&msg))
    }
}

/// The mode check: `real` is only compared when the function is a pure kernel.
pub(crate) fn applies(mode: CompileMode, c: &CompiledFn) -> bool {
    mode == CompileMode::Check && c.pure_kernel
}

#[cfg(test)]
mod thrown_value_tests {
    use super::*;

    #[test]
    fn thrown_primitives_are_compared_by_value() {
        let mut it = Interp::new();
        let throw = |v: Value| -> R<Value> { Err(Ctl::Throw(v)) };
        let num = |n: f64| Value::Number(n);
        assert!(it
            .diff_detail(&throw(num(1.0)), &throw(num(1.0)), &[], &[])
            .is_none());
        assert!(it
            .diff_detail(&throw(num(1.0)), &throw(num(2.0)), &[], &[])
            .is_some());
        assert!(it
            .diff_detail(&throw(num(f64::NAN)), &throw(num(f64::NAN)), &[], &[])
            .is_none());
        assert!(it
            .diff_detail(&throw(num(0.0)), &throw(num(-0.0)), &[], &[])
            .is_some());
        assert!(it
            .diff_detail(&throw(Value::Undefined), &throw(Value::Null), &[], &[])
            .is_some());
    }
}
