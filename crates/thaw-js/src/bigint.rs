//! BigInt: arbitrary precision integers (`num-bigint`), operators, the `BigInt` global.

use crate::*;
use num_bigint::BigInt;
use num_traits::{FromPrimitive, One, Signed, ToPrimitive, Zero};

pub(crate) fn big(v: BigInt) -> Value {
    Value::BigInt(Rc::new(v))
}

/// StringToBigInt: `None` when the text is not an integer literal.
pub(crate) fn parse_bigint(s: &str) -> Option<BigInt> {
    let t = s.trim();
    if t.is_empty() {
        return Some(BigInt::zero());
    }
    let (neg, body, signed) = if let Some(r) = t.strip_prefix('-') {
        (true, r, true)
    } else if let Some(r) = t.strip_prefix('+') {
        (false, r, true)
    } else {
        (false, t, false)
    };
    let (radix, digits) = match body.get(..2) {
        Some("0x" | "0X") if !signed => (16, &body[2..]),
        Some("0o" | "0O") if !signed => (8, &body[2..]),
        Some("0b" | "0B") if !signed => (2, &body[2..]),
        _ => (10, body),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let v = BigInt::parse_bytes(digits.as_bytes(), radix)?;
    Some(if neg { -v } else { v })
}

fn as_big(v: &Value) -> Option<&BigInt> {
    match v {
        Value::BigInt(b) => Some(b),
        _ => None,
    }
}

/// Compare a BigInt with a Number exactly; `None` when the number is NaN.
pub(crate) fn cmp_big_num(b: &BigInt, n: f64) -> Option<std::cmp::Ordering> {
    if n.is_nan() {
        return None;
    }
    if n == f64::INFINITY {
        return Some(std::cmp::Ordering::Less);
    }
    if n == f64::NEG_INFINITY {
        return Some(std::cmp::Ordering::Greater);
    }
    let fl = BigInt::from_f64(n.floor())?;
    match b.cmp(&fl) {
        std::cmp::Ordering::Equal if n.fract() != 0.0 => Some(std::cmp::Ordering::Less),
        o => Some(o),
    }
}

fn grouped(s: &str) -> String {
    let (sign, digits) = match s.strip_prefix('-') {
        Some(d) => ("-", d),
        None => ("", s),
    };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

impl Interp {
    /// Operators with at least one BigInt operand (operands already primitive). `None` = not a
    /// BigInt operation (string concatenation, comparison with a string, ...).
    pub(crate) fn big_binary(&mut self, op: a::BinaryOp, l: &Value, r: &Value) -> R<Option<Value>> {
        use a::BinaryOp as B;
        use std::cmp::Ordering;
        let is_cmp = matches!(op, B::Lt | B::LtEq | B::Gt | B::GtEq);
        let arithmetic = matches!(
            op,
            B::Add
                | B::Sub
                | B::Mul
                | B::Div
                | B::Mod
                | B::Exp
                | B::BitAnd
                | B::BitOr
                | B::BitXor
                | B::LShift
                | B::RShift
                | B::ZeroFillRShift
        );
        if !is_cmp && !arithmetic {
            return Ok(None);
        }
        if matches!(op, B::Add) && (matches!(l, Value::Str(_)) || matches!(r, Value::Str(_))) {
            return Ok(None);
        }
        if is_cmp {
            let ord = match (l, r) {
                (Value::BigInt(x), Value::BigInt(y)) => Some(x.cmp(y)),
                (Value::BigInt(x), Value::Str(s)) => parse_bigint(s).map(|y| (**x).cmp(&y)),
                (Value::Str(s), Value::BigInt(y)) => parse_bigint(s).map(|x| x.cmp(y)),
                (Value::BigInt(x), o) => {
                    let n = self.num(o);
                    cmp_big_num(x, n)
                }
                (o, Value::BigInt(y)) => {
                    let n = self.num(o);
                    cmp_big_num(y, n).map(Ordering::reverse)
                }
                _ => None,
            };
            return Ok(Some(Value::Bool(match ord {
                None => false,
                Some(o) => match op {
                    B::Lt => o.is_lt(),
                    B::LtEq => o.is_le(),
                    B::Gt => o.is_gt(),
                    _ => o.is_ge(),
                },
            })));
        }
        let (Some(x), Some(y)) = (as_big(l), as_big(r)) else {
            return self
                .throw_error(
                    "TypeError",
                    "Cannot mix BigInt and other types, use explicit conversions",
                )
                .map(Some);
        };
        let v = match op {
            B::Add => x + y,
            B::Sub => x - y,
            B::Mul => x * y,
            B::Div | B::Mod => {
                if y.is_zero() {
                    return self.throw_error("RangeError", "Division by zero").map(Some);
                }
                if matches!(op, B::Div) {
                    x / y
                } else {
                    x % y
                }
            }
            B::Exp => {
                if y.is_negative() {
                    return self
                        .throw_error("RangeError", "Exponent must be non-negative")
                        .map(Some);
                }
                let Some(e) = y.to_u32() else {
                    return self
                        .throw_error("RangeError", "Maximum BigInt size exceeded")
                        .map(Some);
                };
                x.pow(e)
            }
            B::BitAnd => x & y,
            B::BitOr => x | y,
            B::BitXor => x ^ y,
            B::LShift | B::RShift => {
                let left = matches!(op, B::LShift) != y.is_negative();
                let Some(n) = y.abs().to_u64().filter(|n| *n < (1 << 32)) else {
                    return self
                        .throw_error("RangeError", "Maximum BigInt size exceeded")
                        .map(Some);
                };
                if left {
                    x << n as usize
                } else {
                    x >> n as usize
                }
            }
            _ => {
                return self
                    .throw_error(
                        "TypeError",
                        "BigInts have no unsigned right shift, use >> instead",
                    )
                    .map(Some)
            }
        };
        Ok(Some(big(v)))
    }

    /// `ToBigInt` as done by the `BigInt(...)` function.
    fn bigint_of(&mut self, v: &Value) -> R<BigInt> {
        let p = self.primitive_of(v, extras::Hint::Number)?;
        match &p {
            Value::BigInt(b) => Ok((**b).clone()),
            Value::Bool(b) => Ok(BigInt::from(*b as u8)),
            Value::Number(n) => {
                if n.is_finite() && n.fract() == 0.0 {
                    Ok(BigInt::from_f64(*n).unwrap_or_default())
                } else {
                    let shown = number_to_string(*n);
                    self.throw_error(
                        "RangeError",
                        &format!("The number {shown} cannot be converted to a BigInt because it is not an integer"),
                    )
                }
            }
            Value::Str(s) => match parse_bigint(s) {
                Some(b) => Ok(b),
                None => self.throw_error("SyntaxError", &format!("Cannot convert {s} to a BigInt")),
            },
            other => {
                let shown = self.to_str(other);
                self.throw_error("TypeError", &format!("Cannot convert {shown} to a BigInt"))
            }
        }
    }

    pub(crate) fn install_bigint(&mut self) {
        let proto = self.plain();
        self.rt.bigint_proto = Some(proto.clone());
        let ctor = self.native(|it, _, args| {
            if it.rt.constructing {
                return it.throw_error("TypeError", "BigInt is not a constructor");
            }
            Ok(big(it.bigint_of(&arg(args, 0))?))
        });
        let Value::Object(c) = &ctor else { return };
        put_hidden(c, "prototype", Value::Object(proto.clone()));
        put_hidden(&proto, "constructor", ctor.clone());
        for signed in [false, true] {
            let name = if signed { "asIntN" } else { "asUintN" };
            self.set_native(c, name, move |it, _, args| {
                let bits = it.num(&arg(args, 0));
                let Some(bits) = (bits.is_finite() && bits >= 0.0).then_some(bits as u64) else {
                    return it.throw_error(
                        "RangeError",
                        "Invalid value: not (convertible to) a safe integer",
                    );
                };
                let x = it.bigint_of(&arg(args, 1))?;
                if bits == 0 {
                    return Ok(big(BigInt::zero()));
                }
                let modulus = BigInt::one() << bits as usize;
                let mut m = x % &modulus;
                if m.is_negative() {
                    m += &modulus;
                }
                if signed && m >= (BigInt::one() << (bits as usize - 1)) {
                    m -= &modulus;
                }
                Ok(big(m))
            });
        }
        let this_big = |it: &mut Interp, this: &Value| -> R<Rc<BigInt>> {
            match this {
                Value::BigInt(b) => Ok(b.clone()),
                _ => it.throw_error(
                    "TypeError",
                    "BigInt.prototype method called on incompatible receiver",
                ),
            }
        };
        self.set_native(&proto, "toString", move |it, this, args| {
            let b = this_big(it, this)?;
            let radix = match arg(args, 0) {
                Value::Undefined => 10,
                v => it.num(&v) as u32,
            };
            if !(2..=36).contains(&radix) {
                return it.throw_error("RangeError", "toString() radix must be between 2 and 36");
            }
            Ok(Value::Str(b.to_str_radix(radix).into()))
        });
        self.set_native(&proto, "toLocaleString", move |it, this, _| {
            let b = this_big(it, this)?;
            Ok(Value::Str(grouped(&b.to_string()).into()))
        });
        self.set_native(&proto, "valueOf", move |it, this, _| {
            Ok(Value::BigInt(this_big(it, this)?))
        });
        let tag = self.well_known_key("toStringTag");
        put_hidden(&proto, &tag, Value::str("BigInt"));
        self.global.declare("BigInt", ctor, true);
    }
}
