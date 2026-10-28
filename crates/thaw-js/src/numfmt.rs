//! `Number.prototype.toFixed` (exact, ties away from zero like V8), `toExponential`, and the
//! `en-US` subset of `toLocaleString`.

use crate::*;

/// Increment the decimal digit string `s` (digits with an optional `.`) by one unit in its last place.
fn bump(s: &str) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    let mut i = chars.len();
    loop {
        if i == 0 {
            chars.insert(0, '1');
            break;
        }
        i -= 1;
        match chars[i] {
            '.' => continue,
            '9' => chars[i] = '0',
            c => {
                chars[i] = char::from_digit(c.to_digit(10).unwrap_or(0) + 1, 10).unwrap_or('0');
                break;
            }
        }
    }
    chars.into_iter().collect()
}

/// `x.toFixed(d)` for |x| < 1e21.
pub(crate) fn to_fixed(x: f64, d: usize) -> String {
    let neg = x < 0.0;
    let abs = x.abs();
    let mut s = format!("{abs:.d$}");
    // `format!` rounds ties to even; V8 rounds exact ties up
    let ext = format!("{abs:.prec$}", prec = d + 1);
    if ext.ends_with('5') {
        let long = format!("{abs:.prec$}", prec = d + 60);
        if long[ext.len()..].bytes().all(|b| b == b'0') {
            s = bump(ext[..ext.len() - 1].trim_end_matches('.'));
        }
    }
    if neg {
        format!("-{s}")
    } else {
        s
    }
}

/// Round the plain decimal string `digits` (no sign, no exponent) to `frac` fractional digits, half up.
fn round_decimal(digits: &str, frac: usize) -> String {
    let (int, fr) = digits.split_once('.').unwrap_or((digits, ""));
    if fr.len() <= frac {
        return digits.to_string();
    }
    let keep = format!("{int}.{}", &fr[..frac]);
    let keep = keep.trim_end_matches('.').to_string();
    if fr.as_bytes()[frac] >= b'5' {
        bump(&keep)
    } else {
        keep
    }
}

/// Expand `number_to_string` output (which may use an exponent) into a plain decimal string.
fn plain_decimal(n: f64) -> String {
    let s = number_to_string(n.abs());
    let Some((mant, exp)) = s.split_once('e') else {
        return s;
    };
    let exp: i32 = exp.trim_start_matches('+').parse().unwrap_or(0);
    let (int, fr) = mant.split_once('.').unwrap_or((mant, ""));
    let digits = format!("{int}{fr}");
    let point = int.len() as i32 + exp;
    if point <= 0 {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else if point as usize >= digits.len() {
        format!("{digits}{}", "0".repeat(point as usize - digits.len()))
    } else {
        format!(
            "{}.{}",
            &digits[..point as usize],
            &digits[point as usize..]
        )
    }
}

fn group(int: &str) -> String {
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Interp {
    pub(crate) fn install_numfmt(&mut self) {
        let np = self.number_proto.clone();
        self.set_native(&np, "toFixed", |it, this, args| {
            let n = it.num(this);
            let d = it.num(&arg(args, 0));
            let d = if d.is_nan() { 0.0 } else { d.trunc() };
            if !(0.0..=100.0).contains(&d) {
                return it.throw_error(
                    "RangeError",
                    "toFixed() digits argument must be between 0 and 100",
                );
            }
            if !n.is_finite() || n.abs() >= 1e21 {
                return Ok(Value::Str(number_to_string(n).into()));
            }
            Ok(Value::Str(to_fixed(n, d as usize).into()))
        });
        self.set_native(&np, "toExponential", |it, this, args| {
            let n = it.num(this);
            if !n.is_finite() {
                return Ok(Value::Str(number_to_string(n).into()));
            }
            let s = match arg(args, 0) {
                Value::Undefined => format!("{n:e}"),
                v => {
                    let d = it.num(&v).trunc();
                    if !(0.0..=100.0).contains(&d) {
                        return it.throw_error(
                            "RangeError",
                            "toExponential() argument must be between 0 and 100",
                        );
                    }
                    format!("{:.*e}", d as usize, n)
                }
            };
            let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
            let sign = if exp.starts_with('-') { "-" } else { "+" };
            Ok(Value::Str(
                format!("{mant}e{sign}{}", exp.trim_start_matches('-')).into(),
            ))
        });
        self.set_native(&np, "toLocaleString", |it, this, args| {
            let n = it.num(this);
            let locale = arg(args, 0);
            if !matches!(&locale, Value::Undefined)
                && !matches!(&locale, Value::Str(s) if s.starts_with("en"))
            {
                return unsupported("Intl");
            }
            let (mut min_f, mut max_f) = (0usize, 3usize);
            let opts = arg(args, 1);
            if let Value::Object(_) = &opts {
                let style = it.get(&opts, "style")?;
                if !matches!(style, Value::Undefined)
                    && !matches!(&style, Value::Str(s) if &**s == "decimal")
                {
                    return unsupported("Intl");
                }
                let lo = it.get(&opts, "minimumFractionDigits")?;
                let hi = it.get(&opts, "maximumFractionDigits")?;
                if !matches!(lo, Value::Undefined) {
                    min_f = it.num(&lo) as usize;
                    max_f = max_f.max(min_f);
                }
                if !matches!(hi, Value::Undefined) {
                    max_f = it.num(&hi) as usize;
                    min_f = min_f.min(max_f);
                }
                let grouping = it.get(&opts, "useGrouping")?;
                if matches!(grouping, Value::Bool(false)) {
                    let s = round_decimal(&plain_decimal(n), max_f);
                    return Ok(Value::Str(finish(n, &s, min_f, false).into()));
                }
            }
            if n.is_nan() {
                return Ok(Value::str("NaN"));
            }
            if n.is_infinite() {
                return Ok(Value::str(if n < 0.0 { "-∞" } else { "∞" }));
            }
            let s = round_decimal(&plain_decimal(n), max_f);
            Ok(Value::Str(finish(n, &s, min_f, true).into()))
        });
    }
}

fn finish(n: f64, rounded: &str, min_f: usize, grouping: bool) -> String {
    let (int, fr) = rounded.split_once('.').unwrap_or((rounded, ""));
    let mut fr = fr.trim_end_matches('0').to_string();
    while fr.len() < min_f {
        fr.push('0');
    }
    let int = if grouping {
        group(int)
    } else {
        int.to_string()
    };
    let sign = if n.is_sign_negative() { "-" } else { "" };
    if fr.is_empty() {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{fr}")
    }
}
