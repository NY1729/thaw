//! Builtin objects: natives written in Rust plus a JS prelude for the callback-heavy ones.

use std::rc::Rc;

use crate::*;

/// Self-hosted standard library: written in the subset the evaluator itself runs.
pub(crate) const PRELUDE: &str = r#"
(function () {
  function def(o, name, f) {
    Object.defineProperty(o, name, { value: f, writable: true, enumerable: false, configurable: true });
  }
  function mkError(name, Parent) {
    var C = function (message) {
      if (!(this instanceof C)) return new C(message);
      if (message !== undefined) def(this, 'message', String(message));
      def(this, 'stack', name + (message !== undefined ? ': ' + String(message) : ''));
    };
    def(C, 'name', name);
    C.prototype = Parent ? Object.create(Parent.prototype) : {};
    def(C.prototype, 'constructor', C);
    def(C.prototype, 'name', name);
    def(C.prototype, 'message', '');
    return C;
  }
  var E = mkError('Error', null);
  def(E.prototype, 'toString', function () {
    var m = String(this.message);
    return m ? this.name + ': ' + m : String(this.name);
  });
  def(E, 'captureStackTrace', function () {});
  Error = E;
  TypeError = mkError('TypeError', E);
  RangeError = mkError('RangeError', E);
  ReferenceError = mkError('ReferenceError', E);
  SyntaxError = mkError('SyntaxError', E);
  EvalError = mkError('EvalError', E);
  URIError = mkError('URIError', E);

  var AP = Array.prototype;
  def(AP, 'forEach', function (f, t) { for (var i = 0; i < this.length; i++) f.call(t, this[i], i, this); });
  def(AP, 'map', function (f, t) { var r = []; for (var i = 0; i < this.length; i++) r.push(f.call(t, this[i], i, this)); return r; });
  def(AP, 'filter', function (f, t) { var r = []; for (var i = 0; i < this.length; i++) if (f.call(t, this[i], i, this)) r.push(this[i]); return r; });
  def(AP, 'some', function (f, t) { for (var i = 0; i < this.length; i++) if (f.call(t, this[i], i, this)) return true; return false; });
  def(AP, 'every', function (f, t) { for (var i = 0; i < this.length; i++) if (!f.call(t, this[i], i, this)) return false; return true; });
  def(AP, 'find', function (f, t) { for (var i = 0; i < this.length; i++) if (f.call(t, this[i], i, this)) return this[i]; });
  def(AP, 'findIndex', function (f, t) { for (var i = 0; i < this.length; i++) if (f.call(t, this[i], i, this)) return i; return -1; });
  def(AP, 'reduce', function (f, init) {
    var i = 0, acc = init;
    if (arguments.length < 2) { if (this.length === 0) throw new TypeError('Reduce of empty array with no initial value'); acc = this[0]; i = 1; }
    for (; i < this.length; i++) acc = f(acc, this[i], i, this);
    return acc;
  });
  def(AP, 'reduceRight', function (f, init) {
    var i = this.length - 1, acc = init;
    if (arguments.length < 2) { if (this.length === 0) throw new TypeError('Reduce of empty array with no initial value'); acc = this[i]; i--; }
    for (; i >= 0; i--) acc = f(acc, this[i], i, this);
    return acc;
  });
  def(AP, 'fill', function (v, s, e) {
    var n = this.length; s = s === undefined ? 0 : (s < 0 ? Math.max(n + s, 0) : Math.min(s, n));
    e = e === undefined ? n : (e < 0 ? Math.max(n + e, 0) : Math.min(e, n));
    for (var i = s; i < e; i++) this[i] = v; return this;
  });
  def(AP, 'sort', function (cmp) {
    var a = this, defined = [], undef = 0;
    for (var i = 0; i < a.length; i++) { if (a[i] === undefined) undef++; else defined.push(a[i]); }
    var f = cmp === undefined ? function (x, y) { x = String(x); y = String(y); return x < y ? -1 : x > y ? 1 : 0; } : cmp;
    var n = defined.length, src = defined, dst = new Array(n);
    for (var w = 1; w < n; w *= 2) {
      for (var lo = 0; lo < n; lo += 2 * w) {
        var mid = Math.min(lo + w, n), hi = Math.min(lo + 2 * w, n), i1 = lo, j = mid, k = lo;
        while (i1 < mid && j < hi) dst[k++] = f(src[j], src[i1]) < 0 ? src[j++] : src[i1++];
        while (i1 < mid) dst[k++] = src[i1++];
        while (j < hi) dst[k++] = src[j++];
      }
      var t = src; src = dst; dst = t;
    }
    for (var m = 0; m < n; m++) a[m] = src[m];
    for (var u = 0; u < undef; u++) a[n + u] = undefined;
    return a;
  });
  def(Object, 'fromEntries', function (entries) { var o = {}; for (var i = 0; i < entries.length; i++) o[entries[i][0]] = entries[i][1]; return o; });
})();
"#;

pub(crate) fn now_ms() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `Date.UTC`-style arguments: year, month (0-based), day, hours, minutes, seconds, ms.
pub(crate) fn civil_to_ms(n: &[f64]) -> f64 {
    if n.iter().any(|x| x.is_nan()) || n.is_empty() {
        return f64::NAN;
    }
    let get = |i: usize, d: f64| n.get(i).copied().unwrap_or(d);
    let (y, mo) = (
        get(0, 1970.0) as i64 + (get(1, 0.0) as i64).div_euclid(12),
        (get(1, 0.0) as i64).rem_euclid(12),
    );
    let days = days_from_civil(y, mo + 1, 1) as f64 + get(2, 1.0) - 1.0;
    days * 86_400_000.0
        + get(3, 0.0) * 3_600_000.0
        + get(4, 0.0) * 60_000.0
        + get(5, 0.0) * 1000.0
        + get(6, 0.0)
}

fn char_to_byte(s: &str, ci: usize) -> usize {
    if s.is_ascii() {
        return ci.min(s.len());
    }
    s.char_indices().nth(ci).map_or(s.len(), |(b, _)| b)
}

fn byte_to_char(s: &str, bi: usize) -> usize {
    if s.is_ascii() {
        return bi;
    }
    s[..bi].chars().count()
}

fn this_str(it: &mut Interp, this: &Value) -> Rc<str> {
    it.to_str(this)
}

impl Interp {
    pub(crate) fn new_regexp(&mut self, source: &str, flags: &str) -> R<Value> {
        let engine_flags: String = flags.chars().filter(|c| "imsuv".contains(*c)).collect();
        let key = (source.to_string(), engine_flags.clone());
        let re = match self.regex_cache.get(&key) {
            Some(re) => re.clone(),
            None => {
                let compiled = match regress::Regex::with_flags(source, engine_flags.as_str()) {
                    Ok(r) => Rc::new(r),
                    Err(e) => {
                        return self.throw_error(
                            "SyntaxError",
                            &format!("Invalid regular expression: /{source}/: {e}"),
                        )
                    }
                };
                self.regex_cache.insert(key, compiled.clone());
                compiled
            }
        };
        let o = new_obj(Some(self.regexp_proto.clone()), Kind::RegExp(re));
        put_hidden(&o, "source", Value::str(source));
        put_hidden(&o, "flags", Value::str(flags));
        put_hidden(&o, "global", Value::Bool(flags.contains('g')));
        put_hidden(&o, "ignoreCase", Value::Bool(flags.contains('i')));
        put_hidden(&o, "multiline", Value::Bool(flags.contains('m')));
        put_hidden(&o, "lastIndex", Value::Number(0.0));
        Ok(Value::Object(o))
    }

    pub(crate) fn date_target(&self, v: &Value) -> bool {
        matches!(v, Value::Object(o) if find_own(o, "__time").is_some())
    }

    pub(crate) fn date_time(&mut self, v: &Value) -> R<f64> {
        match v {
            Value::Object(o) => match find_own(o, "__time") {
                Some((Slot::Data(Value::Number(t)), _)) => Ok(t),
                _ => self.throw_error("TypeError", "this is not a Date object."),
            },
            _ => self.throw_error("TypeError", "this is not a Date object."),
        }
    }

    fn regexp_of(&self, v: &Value) -> Option<(Obj, Rc<regress::Regex>)> {
        if let Value::Object(o) = v {
            if let Kind::RegExp(r) = &*o.kind.borrow() {
                return Some((o.clone(), r.clone()));
            }
        }
        None
    }

    /// One `exec`: returns the match array (with `index`/`input`) and updates `lastIndex`.
    fn regexp_exec(&mut self, re_obj: &Obj, re: &regress::Regex, s: &Rc<str>) -> R<Value> {
        let flags = match find_own(re_obj, "flags") {
            Some((Slot::Data(Value::Str(f)), _)) => f,
            _ => "".into(),
        };
        let (global, sticky) = (flags.contains('g'), flags.contains('y'));
        let last = if global || sticky {
            let v = self.get(&Value::Object(re_obj.clone()), "lastIndex")?;
            self.num(&v).max(0.0) as usize
        } else {
            0
        };
        let nchars = s.chars().count();
        if last > nchars {
            put_hidden(re_obj, "lastIndex", Value::Number(0.0));
            return Ok(Value::Null);
        }
        let start = char_to_byte(s, last);
        let found = re
            .find_from(s, start)
            .next()
            .filter(|m| !sticky || m.start() == start);
        let Some(m) = found else {
            if global || sticky {
                put_hidden(re_obj, "lastIndex", Value::Number(0.0));
            }
            return Ok(Value::Null);
        };
        if global || sticky {
            put_hidden(
                re_obj,
                "lastIndex",
                Value::Number(byte_to_char(s, m.end()) as f64),
            );
        }
        let mut items = vec![Value::str(&s[m.range()])];
        for g in m.groups().skip(1) {
            items.push(g.map_or(Value::Undefined, |r| Value::str(&s[r])));
        }
        let arr = self.array(items);
        if let Value::Object(ao) = &arr {
            put(
                ao,
                "index",
                Value::Number(byte_to_char(s, m.start()) as f64),
            );
            put(ao, "input", Value::Str(s.clone()));
            let named: Vec<(String, Option<std::ops::Range<usize>>)> =
                m.named_groups().map(|(n, r)| (n.to_string(), r)).collect();
            if named.is_empty() {
                put(ao, "groups", Value::Undefined);
            } else {
                let g = self.plain();
                for (n, r) in named {
                    put(&g, &n, r.map_or(Value::Undefined, |r| Value::str(&s[r])));
                }
                put(ao, "groups", Value::Object(g));
            }
        }
        Ok(arr)
    }

    /// All matches of a (global) regexp over `s` as match arrays.
    fn regexp_all(&mut self, re_obj: &Obj, re: &regress::Regex, s: &Rc<str>) -> R<Vec<Value>> {
        put_hidden(re_obj, "lastIndex", Value::Number(0.0));
        let mut out = Vec::new();
        loop {
            let m = self.regexp_exec(re_obj, re, s)?;
            let Value::Object(mo) = &m else { break };
            // advance past empty matches
            let matched = self.get(&m, "0")?;
            if matches!(&matched, Value::Str(x) if x.is_empty()) {
                let li = self.get(&Value::Object(re_obj.clone()), "lastIndex")?;
                let li = self.num(&li);
                put_hidden(re_obj, "lastIndex", Value::Number(li + 1.0));
            }
            let _ = mo;
            out.push(m);
        }
        Ok(out)
    }

    fn expand_replacement(
        &mut self,
        template: &str,
        m: &[Value],
        index: usize,
        input: &str,
    ) -> String {
        let mut out = String::new();
        let chars: Vec<char> = template.chars().collect();
        let matched = m
            .first()
            .map(|v| self.to_str(v))
            .unwrap_or_else(|| "".into());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '$' && i + 1 < chars.len() {
                let c = chars[i + 1];
                match c {
                    '$' => {
                        out.push('$');
                        i += 2;
                        continue;
                    }
                    '&' => {
                        out.push_str(&matched);
                        i += 2;
                        continue;
                    }
                    '`' => {
                        out.extend(input.chars().take(index));
                        i += 2;
                        continue;
                    }
                    '\'' => {
                        out.extend(input.chars().skip(index + matched.chars().count()));
                        i += 2;
                        continue;
                    }
                    d if d.is_ascii_digit() => {
                        let mut n = d.to_digit(10).unwrap() as usize;
                        let mut used = 2;
                        if let Some(d2) = chars.get(i + 2).and_then(|c| c.to_digit(10)) {
                            if n * 10 + (d2 as usize) < m.len() {
                                n = n * 10 + d2 as usize;
                                used = 3;
                            }
                        }
                        if n >= 1 && n < m.len() {
                            if !matches!(m[n], Value::Undefined) {
                                out.push_str(&self.to_str(&m[n].clone()));
                            }
                            i += used;
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    fn string_replace(&mut self, this: &Value, args: &[Value], all: bool) -> R<Value> {
        let s = this_str(self, this);
        let pattern = arg(args, 0);
        let repl = arg(args, 1);
        // (char index, match-array-like values) of the matches to replace
        let mut matches: Vec<(usize, Vec<Value>)> = Vec::new();
        if let Some((ro, re)) = self.regexp_of(&pattern) {
            let global = matches!(
                find_own(&ro, "global"),
                Some((Slot::Data(Value::Bool(true)), _))
            );
            let found = if global {
                self.regexp_all(&ro, &re, &s)?
            } else {
                match self.regexp_exec(&ro, &re, &s)? {
                    Value::Null => vec![],
                    m => vec![m],
                }
            };
            for m in found {
                let idx = self.get(&m, "index")?;
                let idx = self.num(&idx) as usize;
                let Value::Object(mo) = &m else { continue };
                let items = match &*mo.kind.borrow() {
                    Kind::Array(v) => v.clone(),
                    _ => vec![],
                };
                matches.push((idx, items));
            }
        } else {
            let needle = self.to_str(&pattern);
            let mut from = 0;
            while let Some(pos) = s[from..].find(&*needle) {
                let b = from + pos;
                matches.push((byte_to_char(&s, b), vec![Value::str(&needle)]));
                if !all {
                    break;
                }
                from = b + needle.len().max(1);
                if from > s.len() {
                    break;
                }
            }
        }
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut pos = 0usize;
        for (idx, groups) in matches {
            if idx < pos {
                continue;
            }
            out.extend(&chars[pos..idx]);
            let matched_len = groups.first().map_or(0, |v| {
                if let Value::Str(x) = v {
                    x.chars().count()
                } else {
                    0
                }
            });
            let text = if repl.is_callable() {
                let mut call_args = groups.clone();
                call_args.push(Value::Number(idx as f64));
                call_args.push(Value::Str(s.clone()));
                let r = self.call(&repl, &Value::Undefined, &call_args)?;
                self.to_str(&r).to_string()
            } else {
                let t = self.to_str(&repl);
                self.expand_replacement(&t, &groups, idx, &s)
            };
            out.push_str(&text);
            pos = idx + matched_len;
        }
        out.extend(&chars[pos.min(chars.len())..]);
        Ok(Value::Str(out.into()))
    }

    fn define_property(&mut self, target: &Value, key: &str, desc: &Value) -> R<()> {
        let Value::Object(o) = target else {
            return self.throw_error("TypeError", "Object.defineProperty called on non-object");
        };
        let has = |it: &mut Interp, k: &str| -> R<Option<Value>> {
            if let Value::Object(d) = desc {
                if find_in_chain(d, k).is_some() {
                    return Ok(Some(it.get(desc, k)?));
                }
            }
            Ok(None)
        };
        let (value, writable, enumerable, getter, setter) = (
            has(self, "value")?,
            has(self, "writable")?,
            has(self, "enumerable")?,
            has(self, "get")?,
            has(self, "set")?,
        );
        let existing = o.props.borrow().iter().position(|p| &*p.key == key);
        // array index / length writes go through the element storage
        if value.is_some() {
            if let Kind::Array(_) = &*o.kind.borrow() {
                if key.parse::<usize>().is_ok() || key == "length" {
                    drop(self.set(target, key, value.clone().unwrap()));
                    return Ok(());
                }
            }
        }
        let flag = |v: &Option<Value>, default: bool| v.as_ref().map_or(default, |v| v.truthy());
        let mut props = o.props.borrow_mut();
        let slot = if getter.is_some() || setter.is_some() {
            let (mut g, mut s) = match existing.map(|i| props[i].slot.clone()) {
                Some(Slot::Accessor { get, set }) => (get, set),
                _ => (None, None),
            };
            if let Some(v) = getter.filter(|v| v.is_callable()) {
                g = Some(v);
            }
            if let Some(v) = setter.filter(|v| v.is_callable()) {
                s = Some(v);
            }
            Slot::Accessor { get: g, set: s }
        } else {
            match (&value, existing.map(|i| props[i].slot.clone())) {
                (Some(v), _) => Slot::Data(v.clone()),
                (None, Some(old)) => old,
                (None, None) => Slot::Data(Value::Undefined),
            }
        };
        match existing {
            Some(i) => {
                let p = &mut props[i];
                p.slot = slot;
                p.enumerable = flag(&enumerable, p.enumerable);
                p.writable = flag(&writable, p.writable);
            }
            None => props.push(Prop {
                key: key.into(),
                slot,
                enumerable: flag(&enumerable, false),
                writable: flag(&writable, false),
            }),
        }
        Ok(())
    }

    fn range(&mut self, args: &[Value], len: usize) -> (usize, usize) {
        let rel = |it: &mut Interp, v: Value, default: usize| -> usize {
            match v {
                Value::Undefined => default,
                v => {
                    let n = it.num(&v);
                    let n = if n.is_nan() { 0.0 } else { n.trunc() };
                    if n < 0.0 {
                        (len as f64 + n).max(0.0) as usize
                    } else {
                        (n.min(len as f64)) as usize
                    }
                }
            }
        };
        let from = rel(self, arg(args, 0), 0);
        let to = rel(self, arg(args, 1), len);
        (from, to.max(from))
    }

    pub(crate) fn array_items(&self, v: &Value) -> Vec<Value> {
        match v {
            Value::Object(o) => match &*o.kind.borrow() {
                Kind::Array(items) => items.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    pub(crate) fn install_builtins(&mut self) {
        let g = self.global.clone();
        let op = self.object_proto.clone();
        let fp = self.function_proto.clone();

        // ----- console -----
        let console = self.plain();
        self.set_native(&console, "log", |it, _, args| {
            let line = args
                .iter()
                .map(|v| it.inspect(v, true))
                .collect::<Vec<_>>()
                .join(" ");
            it.emit_out(&line, true);
            Ok(Value::Undefined)
        });
        for name in ["error", "warn"] {
            self.set_native(&console, name, |it, _, args| {
                let line = args
                    .iter()
                    .map(|v| it.inspect(v, true))
                    .collect::<Vec<_>>()
                    .join(" ");
                it.emit_err(&line);
                Ok(Value::Undefined)
            });
        }
        g.declare("console", Value::Object(console), true);

        // ----- Math -----
        let math = self.plain();
        macro_rules! math1 {
            ($name:literal, $f:expr) => {
                self.set_native(&math, $name, |it, _, args| {
                    Ok(Value::Number($f(
                        it.num(args.first().unwrap_or(&Value::Undefined))
                    )))
                });
            };
        }
        math1!("floor", f64::floor);
        math1!("ceil", f64::ceil);
        math1!("abs", f64::abs);
        math1!("sqrt", f64::sqrt);
        math1!("cbrt", f64::cbrt);
        math1!("trunc", f64::trunc);
        math1!("sin", f64::sin);
        math1!("cos", f64::cos);
        math1!("tan", f64::tan);
        math1!("atan", f64::atan);
        math1!("asin", f64::asin);
        math1!("acos", f64::acos);
        math1!("exp", f64::exp);
        math1!("log", f64::ln);
        math1!("log2", f64::log2);
        math1!("log10", f64::log10);
        math1!("round", |x: f64| (x + 0.5).floor());
        math1!("sign", |x: f64| if x.is_nan() || x == 0.0 {
            x
        } else {
            x.signum()
        });
        self.set_native(&math, "pow", |it, _, args| {
            Ok(Value::Number(
                it.num(&arg(args, 0)).powf(it.num(&arg(args, 1))),
            ))
        });
        self.set_native(&math, "atan2", |it, _, args| {
            Ok(Value::Number(
                it.num(&arg(args, 0)).atan2(it.num(&arg(args, 1))),
            ))
        });
        self.set_native(&math, "imul", |it, _, args| {
            Ok(Value::Number(
                to_int32(it.num(&arg(args, 0))).wrapping_mul(to_int32(it.num(&arg(args, 1))))
                    as f64,
            ))
        });
        self.set_native(&math, "random", |_, _, _| {
            use std::time::{SystemTime, UNIX_EPOCH};
            let n = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(1, |d| d.subsec_nanos()) as f64;
            Ok(Value::Number((n * 0.618_033_988_749_895).fract()))
        });
        self.set_native(&math, "max", |it, _, args| {
            Ok(Value::Number(args.iter().fold(
                f64::NEG_INFINITY,
                |m, v| {
                    let n = it.num(v);
                    if n.is_nan() || m.is_nan() {
                        f64::NAN
                    } else {
                        m.max(n)
                    }
                },
            )))
        });
        self.set_native(&math, "min", |it, _, args| {
            Ok(Value::Number(args.iter().fold(f64::INFINITY, |m, v| {
                let n = it.num(v);
                if n.is_nan() || m.is_nan() {
                    f64::NAN
                } else {
                    m.min(n)
                }
            })))
        });
        put_hidden(&math, "PI", Value::Number(std::f64::consts::PI));
        put_hidden(&math, "E", Value::Number(std::f64::consts::E));
        g.declare("Math", Value::Object(math), true);

        // ----- Object -----
        let object = self.native(|it, _, args| {
            Ok(match arg(args, 0) {
                v @ Value::Object(_) => v,
                _ => Value::Object(it.plain()),
            })
        });
        if let Value::Object(o) = &object {
            put_hidden(o, "prototype", Value::Object(op.clone()));
            put_hidden(&op, "constructor", object.clone());
            self.set_native(o, "keys", |it, _, args| {
                let keys = it.own_keys(&arg(args, 0));
                Ok(it.array(keys.into_iter().map(Value::Str).collect()))
            });
            self.set_native(o, "values", |it, _, args| {
                let t = arg(args, 0);
                let mut out = Vec::new();
                for k in it.own_keys(&t) {
                    out.push(it.get(&t, &k)?);
                }
                Ok(it.array(out))
            });
            self.set_native(o, "entries", |it, _, args| {
                let t = arg(args, 0);
                let mut out = Vec::new();
                for k in it.own_keys(&t) {
                    let v = it.get(&t, &k)?;
                    out.push(it.array(vec![Value::Str(k), v]));
                }
                Ok(it.array(out))
            });
            self.set_native(o, "assign", |it, _, args| {
                let target = arg(args, 0);
                for src in args.iter().skip(1) {
                    for k in it.own_keys(src) {
                        let v = it.get(src, &k)?;
                        it.set(&target, &k, v)?;
                    }
                }
                Ok(target)
            });
            self.set_native(o, "create", |it, _, args| {
                let proto = match arg(args, 0) {
                    Value::Object(p) => Some(p),
                    _ => None,
                };
                let obj = Value::Object(new_obj(proto, Kind::Plain));
                if let Value::Object(props) = arg(args, 1) {
                    for k in it.own_keys(&Value::Object(props.clone())) {
                        let d = it.get(&Value::Object(props.clone()), &k)?;
                        it.define_property(&obj, &k, &d)?;
                    }
                }
                Ok(obj)
            });
            self.set_native(o, "defineProperty", |it, _, args| {
                let key = it.to_str(&arg(args, 1));
                it.define_property(&arg(args, 0), &key, &arg(args, 2))?;
                Ok(arg(args, 0))
            });
            self.set_native(o, "defineProperties", |it, _, args| {
                let props = arg(args, 1);
                for k in it.own_keys(&props) {
                    let d = it.get(&props, &k)?;
                    it.define_property(&arg(args, 0), &k, &d)?;
                }
                Ok(arg(args, 0))
            });
            self.set_native(o, "getPrototypeOf", |it, _, args| match arg(args, 0) {
                Value::Object(o) => Ok(o.proto.borrow().clone().map_or(Value::Null, Value::Object)),
                Value::Str(_) => Ok(Value::Object(it.string_proto.clone())),
                Value::Undefined | Value::Null => {
                    it.throw_error("TypeError", "Cannot convert undefined or null to object")
                }
                _ => Ok(Value::Object(it.object_proto.clone())),
            });
            self.set_native(o, "setPrototypeOf", |_, _, args| {
                if let Value::Object(o) = arg(args, 0) {
                    *o.proto.borrow_mut() = match arg(args, 1) {
                        Value::Object(p) => Some(p),
                        _ => None,
                    };
                }
                Ok(arg(args, 0))
            });
            self.set_native(o, "getOwnPropertyNames", |it, _, args| {
                let t = arg(args, 0);
                let mut names: Vec<Value> = it.own_keys(&t).into_iter().map(Value::Str).collect();
                if let Value::Object(o) = &t {
                    names.extend(
                        o.props
                            .borrow()
                            .iter()
                            .filter(|p| !p.enumerable && !p.key.starts_with("@@sym:"))
                            .map(|p| Value::Str(p.key.clone())),
                    );
                    if matches!(&*o.kind.borrow(), Kind::Array(_)) {
                        names.push(Value::str("length"));
                    }
                }
                Ok(it.array(names))
            });
            self.set_native(o, "getOwnPropertySymbols", |it, _, args| {
                let Value::Object(o) = arg(args, 0) else {
                    return Ok(it.array(vec![]));
                };
                let found: Vec<Value> = o
                    .props
                    .borrow()
                    .iter()
                    .filter_map(|p| p.key.strip_prefix("@@sym:")?.parse::<u64>().ok())
                    .filter_map(|id| it.rt.symbols.get(&id).cloned().map(Value::Object))
                    .collect();
                Ok(it.array(found))
            });
            self.set_native(o, "getOwnPropertyDescriptor", |it, _, args| {
                let key = it.to_str(&arg(args, 1));
                let Value::Object(o) = arg(args, 0) else {
                    return Ok(Value::Undefined);
                };
                let Some(p) = o.props.borrow().iter().find(|p| *p.key == *key).cloned() else {
                    let v = it.get(&arg(args, 0), &key)?;
                    if matches!(&*o.kind.borrow(), Kind::Array(_)) && !matches!(v, Value::Undefined)
                    {
                        let d = it.plain();
                        put(&d, "value", v);
                        put(&d, "writable", Value::Bool(true));
                        put(&d, "enumerable", Value::Bool(key.as_ref() != "length"));
                        put(&d, "configurable", Value::Bool(true));
                        return Ok(Value::Object(d));
                    }
                    return Ok(Value::Undefined);
                };
                let d = it.plain();
                match p.slot {
                    Slot::Data(v) => {
                        put(&d, "value", v);
                        put(&d, "writable", Value::Bool(p.writable && !o.frozen.get()));
                    }
                    Slot::Accessor { get, set } => {
                        put(&d, "get", get.unwrap_or(Value::Undefined));
                        put(&d, "set", set.unwrap_or(Value::Undefined));
                    }
                }
                put(&d, "enumerable", Value::Bool(p.enumerable));
                put(&d, "configurable", Value::Bool(!o.frozen.get()));
                Ok(Value::Object(d))
            });
            self.set_native(o, "freeze", |_, _, args| {
                if let Value::Object(o) = arg(args, 0) {
                    o.frozen.set(true);
                }
                Ok(arg(args, 0))
            });
            self.set_native(o, "isFrozen", |_, _, args| {
                Ok(Value::Bool(match arg(args, 0) {
                    Value::Object(o) => o.frozen.get(),
                    _ => true,
                }))
            });
            self.set_native(o, "preventExtensions", |_, _, args| Ok(arg(args, 0)));
            self.set_native(o, "seal", |_, _, args| Ok(arg(args, 0)));
            self.set_native(o, "isExtensible", |_, _, _| Ok(Value::Bool(true)));
        }
        g.declare("Object", object, true);
        self.set_native(&op, "hasOwnProperty", |it, this, args| {
            let key = it.to_str(&arg(args, 0));
            Ok(Value::Bool(match this {
                Value::Object(o) => find_own(o, &key).is_some() || matches!(&*o.kind.borrow(), Kind::Array(items) if key.parse::<usize>().is_ok_and(|i| i < items.len()) || *key == *"length"),
                Value::Str(s) => key.parse::<usize>().is_ok_and(|i| i < s.chars().count()),
                _ => false,
            }))
        });
        self.set_native(&op, "isPrototypeOf", |_, this, args| {
            let (Value::Object(p), Value::Object(o)) = (this, arg(args, 0)) else {
                return Ok(Value::Bool(false));
            };
            let mut cur = o.proto.borrow().clone();
            while let Some(c) = cur {
                if Rc::ptr_eq(&c, p) {
                    return Ok(Value::Bool(true));
                }
                cur = c.proto.borrow().clone();
            }
            Ok(Value::Bool(false))
        });
        self.set_native(&op, "propertyIsEnumerable", |it, this, args| {
            let key = it.to_str(&arg(args, 0));
            Ok(Value::Bool(match this {
                Value::Object(o) => find_own(o, &key).is_some_and(|(_, e)| e) || matches!(&*o.kind.borrow(), Kind::Array(items) if key.parse::<usize>().is_ok_and(|i| i < items.len())),
                _ => false,
            }))
        });
        self.set_native(&op, "valueOf", |_, this, _| Ok(this.clone()));
        self.set_native(&op, "toString", |_, this, _| {
            Ok(Value::str(match this {
                Value::Undefined => "[object Undefined]",
                Value::Null => "[object Null]",
                Value::Number(_) => "[object Number]",
                Value::Bool(_) => "[object Boolean]",
                Value::Str(_) => "[object String]",
                Value::Object(o) => match &*o.kind.borrow() {
                    Kind::Array(_) => "[object Array]",
                    Kind::Closure { .. } | Kind::Native(_) => "[object Function]",
                    Kind::RegExp(_) => "[object RegExp]",
                    Kind::Map(_) => "[object Map]",
                    Kind::Set(_) => "[object Set]",
                    Kind::Symbol { .. } => "[object Symbol]",
                    Kind::Promise(_) => "[object Promise]",
                    Kind::ArrayBuffer(_) => "[object ArrayBuffer]",
                    Kind::Typed(t) => t.ty.tag(),
                    Kind::Plain if find_in_chain(o, "__time").is_some() => "[object Date]",
                    Kind::Plain | Kind::Global => "[object Object]",
                },
            }))
        });

        // ----- Function.prototype -----
        self.set_native(&fp, "call", |it, this, args| {
            it.call(this, &arg(args, 0), args.get(1..).unwrap_or(&[]))
        });
        self.set_native(&fp, "apply", |it, this, args| {
            let list = match arg(args, 1) {
                Value::Undefined | Value::Null => Vec::new(),
                v => it.array_items(&v),
            };
            it.call(this, &arg(args, 0), &list)
        });
        self.set_native(&fp, "bind", |it, this, args| {
            let (target, bound_this, bound_args) = (
                this.clone(),
                arg(args, 0),
                args.get(1..).unwrap_or(&[]).to_vec(),
            );
            Ok(it.native(move |it, _, call_args| {
                let mut all = bound_args.clone();
                all.extend_from_slice(call_args);
                it.call(&target, &bound_this, &all)
            }))
        });
        self.set_native(&fp, "toString", |_, this, _| {
            Ok(Value::str(if this.is_callable() {
                "function () { [native code] }"
            } else {
                ""
            }))
        });
        let function = self.native(|it, _, args| {
            let (body, params) = match args.split_last() {
                Some((b, p)) => (
                    it.to_str(b),
                    p.iter()
                        .map(|v| it.to_str(v).to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                ),
                None => ("".into(), String::new()),
            };
            it.eval_function(&format!("function anonymous({params}\n) {{\n{body}\n}}"))
        });
        g.declare(
            "globalThis",
            Value::Object(self.global_object.clone()),
            true,
        );
        g.declare("global", Value::Object(self.global_object.clone()), true);
        if let Value::Object(f) = &function {
            put_hidden(f, "prototype", Value::Object(fp.clone()));
            put_hidden(&fp, "constructor", function.clone());
        }
        g.declare("Function", function, true);

        // ----- Array -----
        let ap = self.array_proto.clone();
        let array = self.native(|it, _, args| {
            Ok(match args {
                [Value::Number(n)] => it.array(vec![Value::Undefined; *n as usize]),
                _ => it.array(args.to_vec()),
            })
        });
        if let Value::Object(o) = &array {
            put_hidden(o, "prototype", Value::Object(ap.clone()));
            put_hidden(&ap, "constructor", array.clone());
            self.set_native(o, "isArray", |_, _, args| Ok(Value::Bool(matches!(arg(args, 0), Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_))))));
            self.set_native(o, "of", |it, _, args| Ok(it.array(args.to_vec())));
            self.set_native(o, "from", |it, _, args| {
                let src = arg(args, 0);
                let mut items = match &src {
                    Value::Str(s) => s
                        .chars()
                        .map(|c| Value::Str(c.to_string().into()))
                        .collect(),
                    Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_)) => {
                        it.array_items(&src)
                    }
                    Value::Object(_) => match it.iter_protocol(&src)? {
                        Some(items) => items,
                        None => {
                            let n = it.get(&src, "length")?;
                            let n = it.num(&n);
                            let mut v = Vec::new();
                            for i in 0..(if n.is_nan() { 0 } else { n as usize }) {
                                v.push(it.get(&src, &i.to_string())?);
                            }
                            v
                        }
                    },
                    _ => Vec::new(),
                };
                if arg(args, 1).is_callable() {
                    for (i, item) in items.iter_mut().enumerate() {
                        *item = it.call(
                            &arg(args, 1),
                            &Value::Undefined,
                            &[item.clone(), Value::Number(i as f64)],
                        )?;
                    }
                }
                Ok(it.array(items))
            });
        }
        g.declare("Array", array, true);
        self.set_native(&ap, "push", |it, this, args| {
            let Value::Object(o) = this else {
                return it.throw_error("TypeError", "push on non-array");
            };
            let mut kind = o.kind.borrow_mut();
            let Kind::Array(items) = &mut *kind else {
                drop(kind);
                return it.throw_error("TypeError", "push on non-array");
            };
            if o.frozen.get() {
                return Ok(Value::Number(items.len() as f64));
            }
            items.extend(args.iter().cloned());
            Ok(Value::Number(items.len() as f64))
        });
        self.set_native(&ap, "pop", |_, this, _| {
            if let Value::Object(o) = this {
                if let Kind::Array(items) = &mut *o.kind.borrow_mut() {
                    return Ok(items.pop().unwrap_or(Value::Undefined));
                }
            }
            Ok(Value::Undefined)
        });
        self.set_native(&ap, "shift", |_, this, _| {
            if let Value::Object(o) = this {
                if let Kind::Array(items) = &mut *o.kind.borrow_mut() {
                    return Ok(if items.is_empty() {
                        Value::Undefined
                    } else {
                        items.remove(0)
                    });
                }
            }
            Ok(Value::Undefined)
        });
        self.set_native(&ap, "unshift", |_, this, args| {
            if let Value::Object(o) = this {
                if let Kind::Array(items) = &mut *o.kind.borrow_mut() {
                    items.splice(0..0, args.iter().cloned());
                    return Ok(Value::Number(items.len() as f64));
                }
            }
            Ok(Value::Undefined)
        });
        self.set_native(&ap, "reverse", |_, this, _| {
            if let Value::Object(o) = this {
                if let Kind::Array(items) = &mut *o.kind.borrow_mut() {
                    items.reverse();
                }
            }
            Ok(this.clone())
        });
        self.set_native(&ap, "splice", |it, this, args| {
            let Value::Object(o) = this else {
                return Ok(Value::Undefined);
            };
            let len = it.array_items(this).len();
            let start = match arg(args, 0) {
                Value::Undefined => 0,
                v => {
                    let n = it.num(&v);
                    if n < 0.0 {
                        (len as f64 + n).max(0.0) as usize
                    } else {
                        (n as usize).min(len)
                    }
                }
            };
            let del = if args.len() < 2 {
                len - start
            } else {
                (it.num(&args[1]).max(0.0) as usize).min(len - start)
            };
            let removed: Vec<Value> = match &mut *o.kind.borrow_mut() {
                Kind::Array(items) => items
                    .splice(start..start + del, args.iter().skip(2).cloned())
                    .collect(),
                _ => Vec::new(),
            };
            Ok(it.array(removed))
        });
        self.set_native(&ap, "join", |it, this, args| {
            let sep = match arg(args, 0) {
                Value::Undefined => ",".into(),
                v => it.to_str(&v),
            };
            Ok(Value::Str(it.join(this, &sep)))
        });
        for (name, last) in [("indexOf", false), ("lastIndexOf", true)] {
            self.set_native(&ap, name, move |it, this, args| {
                let needle = arg(args, 0);
                let items = it.array_items(this);
                let pos = if last {
                    items.iter().rposition(|v| strict_equals(v, &needle))
                } else {
                    items.iter().position(|v| strict_equals(v, &needle))
                };
                Ok(Value::Number(pos.map_or(-1.0, |i| i as f64)))
            });
        }
        self.set_native(&ap, "includes", |it, this, args| {
            let needle = arg(args, 0);
            let same_zero = |v: &Value| strict_equals(v, &needle) || matches!((v, &needle), (Value::Number(x), Value::Number(y)) if x.is_nan() && y.is_nan());
            Ok(Value::Bool(it.array_items(this).iter().any(same_zero)))
        });
        self.set_native(&ap, "slice", |it, this, args| {
            let items = it.array_items(this);
            let (from, to) = it.range(args, items.len());
            Ok(it.array(items[from..to].to_vec()))
        });
        self.set_native(&ap, "concat", |it, this, args| {
            let mut items = it.array_items(this);
            for v in args {
                match v {
                    Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_)) => {
                        items.extend(it.array_items(v))
                    }
                    other => items.push(other.clone()),
                }
            }
            Ok(it.array(items))
        });

        // ----- String -----
        let sp = self.string_proto.clone();
        let string = self.native(|it, _, args| {
            Ok(Value::Str(if args.is_empty() {
                "".into()
            } else {
                it.to_str(&args[0])
            }))
        });
        if let Value::Object(o) = &string {
            put_hidden(o, "prototype", Value::Object(sp.clone()));
            put_hidden(&sp, "constructor", string.clone());
            self.set_native(o, "fromCharCode", |it, _, args| {
                let s: String = args
                    .iter()
                    .map(|v| char::from_u32(it.num(v) as u32 & 0xFFFF).unwrap_or('\u{FFFD}'))
                    .collect();
                Ok(Value::Str(s.into()))
            });
        }
        g.declare("String", string, true);
        self.set_native(&sp, "toString", |it, this, _| {
            Ok(Value::Str(this_str(it, this)))
        });
        self.set_native(&sp, "valueOf", |it, this, _| {
            Ok(Value::Str(this_str(it, this)))
        });
        self.set_native(&sp, "charAt", |it, this, args| {
            let s = this_str(it, this);
            let i = it.num(&arg(args, 0));
            Ok(Value::Str(if i >= 0.0 {
                s.chars()
                    .nth(i as usize)
                    .map(String::from)
                    .unwrap_or_default()
                    .into()
            } else {
                "".into()
            }))
        });
        self.set_native(&sp, "charCodeAt", |it, this, args| {
            let s = this_str(it, this);
            let i = it.num(&arg(args, 0));
            Ok(Value::Number(if i >= 0.0 {
                s.chars()
                    .nth(i as usize)
                    .map_or(f64::NAN, |c| c as u32 as f64)
            } else {
                f64::NAN
            }))
        });
        self.set_native(&sp, "indexOf", |it, this, args| {
            let (s, n) = (this_str(it, this), it.to_str(&arg(args, 0)));
            let from = match arg(args, 1) {
                Value::Undefined => 0,
                v => char_to_byte(&s, it.num(&v).max(0.0) as usize),
            };
            Ok(Value::Number(
                s[from..]
                    .find(&*n)
                    .map_or(-1.0, |b| byte_to_char(&s, from + b) as f64),
            ))
        });
        self.set_native(&sp, "lastIndexOf", |it, this, args| {
            let (s, n) = (this_str(it, this), it.to_str(&arg(args, 0)));
            Ok(Value::Number(
                s.rfind(&*n).map_or(-1.0, |b| byte_to_char(&s, b) as f64),
            ))
        });
        self.set_native(&sp, "includes", |it, this, args| {
            let (s, n) = (this_str(it, this), it.to_str(&arg(args, 0)));
            Ok(Value::Bool(s.contains(&*n)))
        });
        self.set_native(&sp, "startsWith", |it, this, args| {
            let (s, n) = (this_str(it, this), it.to_str(&arg(args, 0)));
            let from = char_to_byte(
                &s,
                it.num(&arg(args, 1)).max(0.0) as usize
                    * (!matches!(arg(args, 1), Value::Undefined) as usize),
            );
            Ok(Value::Bool(s[from..].starts_with(&*n)))
        });
        self.set_native(&sp, "endsWith", |it, this, args| {
            let (s, n) = (this_str(it, this), it.to_str(&arg(args, 0)));
            Ok(Value::Bool(s.ends_with(&*n)))
        });
        self.set_native(&sp, "toUpperCase", |it, this, _| {
            Ok(Value::Str(this_str(it, this).to_uppercase().into()))
        });
        self.set_native(&sp, "toLowerCase", |it, this, _| {
            Ok(Value::Str(this_str(it, this).to_lowercase().into()))
        });
        self.set_native(&sp, "trim", |it, this, _| {
            Ok(Value::Str(this_str(it, this).trim().into()))
        });
        self.set_native(&sp, "trimStart", |it, this, _| {
            Ok(Value::Str(this_str(it, this).trim_start().into()))
        });
        self.set_native(&sp, "trimEnd", |it, this, _| {
            Ok(Value::Str(this_str(it, this).trim_end().into()))
        });
        self.set_native(&sp, "repeat", |it, this, args| {
            let n = it.num(&arg(args, 0));
            if !(0.0..1e9).contains(&n) {
                return it.throw_error(
                    "RangeError",
                    &format!("Invalid count value: {}", number_to_string(n)),
                );
            }
            Ok(Value::Str(this_str(it, this).repeat(n as usize).into()))
        });
        for (name, start) in [("padStart", true), ("padEnd", false)] {
            self.set_native(&sp, name, move |it, this, args| {
                let s = this_str(it, this);
                let target = it.num(&arg(args, 0)).max(0.0) as usize;
                let fill: Rc<str> = match arg(args, 1) {
                    Value::Undefined => " ".into(),
                    v => it.to_str(&v),
                };
                let len = s.chars().count();
                if target <= len || fill.is_empty() {
                    return Ok(Value::Str(s));
                }
                let pad: String = fill.chars().cycle().take(target - len).collect();
                Ok(Value::Str(
                    if start {
                        format!("{pad}{s}")
                    } else {
                        format!("{s}{pad}")
                    }
                    .into(),
                ))
            });
        }
        self.set_native(&sp, "slice", |it, this, args| {
            let chars: Vec<char> = this_str(it, this).chars().collect();
            let (from, to) = it.range(args, chars.len());
            Ok(Value::Str(
                chars[from..to].iter().collect::<String>().into(),
            ))
        });
        self.set_native(&sp, "substring", |it, this, args| {
            let chars: Vec<char> = this_str(it, this).chars().collect();
            let clamp = |it: &mut Interp, v: Value, d: usize| match v {
                Value::Undefined => d,
                v => {
                    let n = it.num(&v);
                    if n.is_nan() || n < 0.0 {
                        0
                    } else {
                        (n as usize).min(chars.len())
                    }
                }
            };
            let (a, b) = (
                clamp(it, arg(args, 0), 0),
                clamp(it, arg(args, 1), chars.len()),
            );
            let (from, to) = (a.min(b), a.max(b));
            Ok(Value::Str(
                chars[from..to].iter().collect::<String>().into(),
            ))
        });
        self.set_native(&sp, "substr", |it, this, args| {
            let chars: Vec<char> = this_str(it, this).chars().collect();
            let n = it.num(&arg(args, 0));
            let from = if n < 0.0 {
                (chars.len() as f64 + n).max(0.0) as usize
            } else {
                (n as usize).min(chars.len())
            };
            let len = match arg(args, 1) {
                Value::Undefined => chars.len() - from,
                v => (it.num(&v).max(0.0) as usize).min(chars.len() - from),
            };
            Ok(Value::Str(
                chars[from..from + len].iter().collect::<String>().into(),
            ))
        });
        self.set_native(&sp, "concat", |it, this, args| {
            let mut s = this_str(it, this).to_string();
            for v in args {
                s.push_str(&it.to_str(v));
            }
            Ok(Value::Str(s.into()))
        });
        self.set_native(&sp, "localeCompare", |it, this, args| {
            let (a, b) = (this_str(it, this), it.to_str(&arg(args, 0)));
            Ok(Value::Number(match a.cmp(&b) {
                std::cmp::Ordering::Less => -1.0,
                std::cmp::Ordering::Equal => 0.0,
                std::cmp::Ordering::Greater => 1.0,
            })) // ponytail: code point order, not ICU collation
        });
        self.set_native(&sp, "replace", |it, this, args| {
            it.string_replace(this, args, false)
        });
        self.set_native(&sp, "replaceAll", |it, this, args| {
            it.string_replace(this, args, true)
        });
        self.set_native(&sp, "match", |it, this, args| {
            let s = this_str(it, this);
            let pat = arg(args, 0);
            let (ro, re) = match it.regexp_of(&pat) {
                Some(x) => x,
                None => {
                    let src = if matches!(pat, Value::Undefined) {
                        "(?:)".into()
                    } else {
                        it.to_str(&pat)
                    };
                    let r = it.new_regexp(&src, "")?;
                    it.regexp_of(&r).unwrap()
                }
            };
            if matches!(
                find_own(&ro, "global"),
                Some((Slot::Data(Value::Bool(true)), _))
            ) {
                let all = it.regexp_all(&ro, &re, &s)?;
                if all.is_empty() {
                    return Ok(Value::Null);
                }
                let mut out = Vec::new();
                for m in all {
                    out.push(it.get(&m, "0")?);
                }
                return Ok(it.array(out));
            }
            it.regexp_exec(&ro, &re, &s)
        });
        self.set_native(&sp, "search", |it, this, args| {
            let s = this_str(it, this);
            let pat = arg(args, 0);
            let (_, re) = match it.regexp_of(&pat) {
                Some(x) => x,
                None => {
                    let src = it.to_str(&pat);
                    let r = it.new_regexp(&src, "")?;
                    it.regexp_of(&r).unwrap()
                }
            };
            Ok(Value::Number(
                re.find(&s)
                    .map_or(-1.0, |m| byte_to_char(&s, m.start()) as f64),
            ))
        });
        self.set_native(&sp, "split", |it, this, args| {
            let s = this_str(it, this);
            let limit = match arg(args, 1) {
                Value::Undefined => usize::MAX,
                v => it.num(&v) as usize,
            };
            let sep = arg(args, 0);
            let mut parts: Vec<Value> = Vec::new();
            if let Some((_, re)) = it.regexp_of(&sep) {
                if s.is_empty() {
                    if re.find(&s).is_none() {
                        parts.push(Value::Str(s.clone()));
                    }
                } else {
                    let (mut p, mut q) = (0usize, 0usize);
                    while q < s.len() {
                        let Some(m) = re.find_from(&s, q).next() else {
                            break;
                        };
                        let (st, e) = (m.start(), m.end());
                        if st >= s.len() {
                            break;
                        }
                        if e == p {
                            q = st.max(q) + s[st.max(q)..].chars().next().map_or(1, char::len_utf8);
                            continue;
                        }
                        parts.push(Value::str(&s[p..st]));
                        for gr in m.groups().skip(1) {
                            parts.push(gr.map_or(Value::Undefined, |r| Value::str(&s[r])));
                        }
                        p = e;
                        q = e;
                    }
                    let last = p;
                    parts.push(Value::str(&s[last..]));
                }
            } else if matches!(sep, Value::Undefined) {
                parts.push(Value::Str(s.clone()));
            } else {
                let sep = it.to_str(&sep);
                if sep.is_empty() {
                    parts.extend(s.chars().map(|c| Value::Str(c.to_string().into())));
                } else {
                    parts.extend(s.split(&*sep).map(Value::str));
                }
            }
            parts.truncate(limit);
            Ok(it.array(parts))
        });

        // ----- RegExp -----
        let rp = self.regexp_proto.clone();
        let regexp = self.native(|it, _, args| {
            let src = match arg(args, 0) {
                v @ Value::Object(_) if it.regexp_of(&v).is_some() => it.get(&v, "source")?,
                v => v,
            };
            let src = if matches!(src, Value::Undefined) {
                "(?:)".into()
            } else {
                it.to_str(&src)
            };
            let flags = match arg(args, 1) {
                Value::Undefined => "".into(),
                v => it.to_str(&v),
            };
            it.new_regexp(&src, &flags)
        });
        if let Value::Object(o) = &regexp {
            put_hidden(o, "prototype", Value::Object(rp.clone()));
            put_hidden(&rp, "constructor", regexp.clone());
        }
        g.declare("RegExp", regexp, true);
        self.set_native(&rp, "exec", |it, this, args| {
            let s = it.to_str(&arg(args, 0));
            let Some((ro, re)) = it.regexp_of(this) else {
                return it.throw_error("TypeError", "exec called on incompatible receiver");
            };
            it.regexp_exec(&ro, &re, &s)
        });
        self.set_native(&rp, "test", |it, this, args| {
            let s = it.to_str(&arg(args, 0));
            let Some((ro, re)) = it.regexp_of(this) else {
                return it.throw_error("TypeError", "test called on incompatible receiver");
            };
            Ok(Value::Bool(!matches!(
                it.regexp_exec(&ro, &re, &s)?,
                Value::Null
            )))
        });
        self.set_native(&rp, "toString", |it, this, _| {
            let (s, f) = (it.get(this, "source")?, it.get(this, "flags")?);
            Ok(Value::Str(
                format!("/{}/{}", it.to_str(&s), it.to_str(&f)).into(),
            ))
        });

        // ----- Number / Boolean / globals -----
        let number = self.native(|it, _, args| {
            Ok(Value::Number(if args.is_empty() {
                0.0
            } else {
                it.num(&args[0])
            }))
        });
        if let Value::Object(o) = &number {
            put_hidden(o, "MAX_SAFE_INTEGER", Value::Number(9007199254740991.0));
            put_hidden(o, "MIN_SAFE_INTEGER", Value::Number(-9007199254740991.0));
            put_hidden(o, "EPSILON", Value::Number(f64::EPSILON));
            put_hidden(o, "MAX_VALUE", Value::Number(f64::MAX));
            put_hidden(o, "POSITIVE_INFINITY", Value::Number(f64::INFINITY));
            put_hidden(o, "NEGATIVE_INFINITY", Value::Number(f64::NEG_INFINITY));
            put_hidden(o, "NaN", Value::Number(f64::NAN));
            self.set_native(o, "isInteger", |_, _, args| {
                Ok(Value::Bool(
                    matches!(arg(args, 0), Value::Number(n) if n.is_finite() && n.fract() == 0.0),
                ))
            });
            self.set_native(o, "isSafeInteger", |_, _, args| Ok(Value::Bool(matches!(arg(args, 0), Value::Number(n) if n.is_finite() && n.fract() == 0.0 && n.abs() <= 9007199254740991.0))));
            self.set_native(o, "isFinite", |_, _, args| {
                Ok(Value::Bool(
                    matches!(arg(args, 0), Value::Number(n) if n.is_finite()),
                ))
            });
            self.set_native(o, "isNaN", |_, _, args| {
                Ok(Value::Bool(
                    matches!(arg(args, 0), Value::Number(n) if n.is_nan()),
                ))
            });
        }
        g.declare("Number", number, true);
        self.set_native(&self.number_proto.clone(), "toString", |it, this, args| {
            let n = it.num(this);
            let radix = match arg(args, 0) {
                Value::Undefined => 10,
                v => it.num(&v) as u32,
            };
            if radix == 10 || !n.is_finite() || n.fract() != 0.0 {
                return Ok(Value::Str(number_to_string(n).into()));
            }
            let (neg, mut v) = (n < 0.0, n.abs() as u64);
            let mut digits = Vec::new();
            while v > 0 {
                digits.push(std::char::from_digit((v % radix as u64) as u32, radix).unwrap_or('0'));
                v /= radix as u64;
            }
            if digits.is_empty() {
                digits.push('0');
            }
            if neg {
                digits.push('-');
            }
            Ok(Value::Str(digits.iter().rev().collect::<String>().into()))
        });
        self.set_native(&self.number_proto.clone(), "toFixed", |it, this, args| {
            let n = it.num(this);
            let d = it.num(&arg(args, 0)).max(0.0) as usize;
            Ok(Value::Str(
                if n.abs() >= 1e21 {
                    number_to_string(n)
                } else {
                    format!("{n:.d$}")
                }
                .into(),
            ))
        });
        self.set_native(&self.number_proto.clone(), "valueOf", |_, this, _| {
            Ok(this.clone())
        });
        self.set_native(&self.bool_proto.clone(), "toString", |it, this, _| {
            Ok(Value::Str(it.to_str(this)))
        });
        g.declare(
            "Boolean",
            self.native(|_, _, args| Ok(Value::Bool(arg(args, 0).truthy()))),
            true,
        );
        g.declare(
            "isNaN",
            self.native(|it, _, args| Ok(Value::Bool(it.num(&arg(args, 0)).is_nan()))),
            true,
        );
        g.declare(
            "isFinite",
            self.native(|it, _, args| Ok(Value::Bool(it.num(&arg(args, 0)).is_finite()))),
            true,
        );
        let parse_float = self.native(|it, _, args| {
            let s = it.to_str(&arg(args, 0));
            let t = s.trim_start();
            let mut end = 0;
            let mut best = f64::NAN;
            for (i, c) in t.char_indices() {
                end = i + c.len_utf8();
                if let Ok(v) = t[..end].parse::<f64>() {
                    if !t[..end].contains(|c: char| c.is_alphabetic() && c != 'e' && c != 'E') {
                        best = v;
                    }
                }
            }
            let _ = end;
            if best.is_nan() {
                for (word, v) in [
                    ("Infinity", f64::INFINITY),
                    ("+Infinity", f64::INFINITY),
                    ("-Infinity", f64::NEG_INFINITY),
                ] {
                    if t.starts_with(word) {
                        return Ok(Value::Number(v));
                    }
                }
            }
            Ok(Value::Number(best))
        });
        g.declare("parseFloat", parse_float.clone(), true);
        if let Value::Object(n) = g
            .vars
            .borrow()
            .get("Number")
            .map(|b| b.value.clone())
            .unwrap_or(Value::Undefined)
        {
            put_hidden(&n, "parseFloat", parse_float);
        }
        let parse_int = self.native(|it, _, args| {
            let s = it.to_str(&arg(args, 0));
            let mut radix = match arg(args, 1) {
                Value::Undefined => 0,
                v => it.num(&v) as u32,
            };
            let t = s.trim();
            let (neg, t) = match t.strip_prefix('-') {
                Some(r) => (true, r),
                None => (false, t.strip_prefix('+').unwrap_or(t)),
            };
            let mut t = t;
            if (radix == 0 || radix == 16) && (t.starts_with("0x") || t.starts_with("0X")) {
                t = &t[2..];
                radix = 16;
            }
            let radix = if radix == 0 { 10 } else { radix };
            if !(2..=36).contains(&radix) {
                return Ok(Value::Number(f64::NAN));
            }
            let digits: String = t.chars().take_while(|c| c.is_digit(radix)).collect();
            if digits.is_empty() {
                return Ok(Value::Number(f64::NAN));
            }
            let v = digits.chars().fold(0f64, |acc, c| {
                acc * radix as f64 + c.to_digit(radix).unwrap() as f64
            });
            Ok(Value::Number(if neg { -v } else { v }))
        });
        g.declare("parseInt", parse_int.clone(), true);
        if let Value::Object(n) = g
            .vars
            .borrow()
            .get("Number")
            .map(|b| b.value.clone())
            .unwrap_or(Value::Undefined)
        {
            put_hidden(&n, "parseInt", parse_int);
        }
        g.declare("undefined", Value::Undefined, false);
        g.declare("NaN", Value::Number(f64::NAN), false);
        g.declare("Infinity", Value::Number(f64::INFINITY), false);

        // ----- Date (UTC only: no time zone support yet) -----
        let dp = self.plain();
        let date = self.native({
            let dp = dp.clone();
            move |it, this, args| {
                // `Date()` without `new` is the current time as a string
                if !std::mem::take(&mut it.rt.constructing) && matches!(this, Value::Undefined) {
                    let o = new_obj(Some(dp.clone()), Kind::Plain);
                    put_hidden(&o, "__time", Value::Number(now_ms()));
                    let f = it.get(&Value::Object(o.clone()), "toString")?;
                    return it.call(&f, &Value::Object(o), &[]);
                }
                let t = match args {
                    [] => now_ms(),
                    [Value::Str(s)] => crate::date::parse_date(s),
                    [Value::Null] => 0.0,
                    [Value::Number(n)] => crate::date::clip(*n),
                    [Value::Object(_)] => match it.get(&args[0], "getTime")? {
                        f if f.is_callable() => {
                            let v = it.call(&f, &args[0], &[])?;
                            it.num(&v)
                        }
                        _ => f64::NAN,
                    },
                    parts => {
                        let n: Vec<f64> = parts.iter().map(|v| it.num(v)).collect();
                        crate::date::local_from_parts(&n)
                    }
                };
                if matches!(this, Value::Object(_)) && it.date_target(this) {
                    put_hidden(
                        match this {
                            Value::Object(o) => o,
                            _ => unreachable!(),
                        },
                        "__time",
                        Value::Number(t),
                    );
                    return Ok(this.clone());
                }
                let o = new_obj(Some(dp.clone()), Kind::Plain);
                put_hidden(&o, "__time", Value::Number(t));
                Ok(Value::Object(o))
            }
        });
        if let Value::Object(d) = &date {
            put_hidden(d, "prototype", Value::Object(dp.clone()));
            put_hidden(&dp, "constructor", date.clone());
            self.set_native(d, "now", |_, _, _| Ok(Value::Number(now_ms())));
            self.set_native(d, "parse", |it, _, args| {
                Ok(Value::Number(crate::date::parse_date(
                    &it.to_str(&arg(args, 0)),
                )))
            });
            self.set_native(d, "UTC", |it, _, args| {
                let n: Vec<f64> = args.iter().map(|v| it.num(v)).collect();
                Ok(Value::Number(crate::date::utc_from_parts(&n)))
            });
        }
        g.declare("Date", date, true);
        for (name, f) in [
            ("getTime", 0u8),
            ("valueOf", 0),
            ("getUTCFullYear", 1),
            ("getUTCMonth", 2),
            ("getUTCDate", 3),
            ("getUTCDay", 4),
            ("getUTCHours", 5),
            ("getUTCMinutes", 6),
            ("getUTCSeconds", 7),
            ("getUTCMilliseconds", 8),
        ] {
            self.set_native(&dp, name, move |it, this, _| {
                let t = it.date_time(this)?;
                if t.is_nan() {
                    return Ok(Value::Number(f64::NAN));
                }
                let days = (t / 86_400_000.0).floor();
                let ms_in_day = t - days * 86_400_000.0;
                let (y, m, d) = civil_from_days(days as i64);
                Ok(Value::Number(match f {
                    0 => t,
                    1 => y as f64,
                    2 => (m - 1) as f64,
                    3 => d as f64,
                    4 => ((days as i64 + 4).rem_euclid(7)) as f64,
                    5 => (ms_in_day / 3_600_000.0).floor(),
                    6 => ((ms_in_day / 60_000.0).floor()) % 60.0,
                    7 => ((ms_in_day / 1000.0).floor()) % 60.0,
                    _ => ms_in_day % 1000.0,
                }))
            });
        }
        self.set_native(&dp, "toISOString", |it, this, _| {
            let t = it.date_time(this)?;
            if t.is_nan() {
                return it.throw_error("RangeError", "Invalid time value");
            }
            let days = (t / 86_400_000.0).floor();
            let r = t - days * 86_400_000.0;
            let (y, m, d) = civil_from_days(days as i64);
            Ok(Value::Str(
                format!(
                    "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
                    (r / 3_600_000.0).floor(),
                    ((r / 60_000.0).floor()) % 60.0,
                    ((r / 1000.0).floor()) % 60.0,
                    r % 1000.0
                )
                .into(),
            ))
        });
        self.install_date_extras(&dp);
        self.set_native(&dp, "toJSON", |it, this, _| {
            let f = it.get(this, "toISOString")?;
            it.call(&f, this, &[])
        });

        // ----- JSON -----
        let json = self.plain();
        self.set_native(&json, "stringify", |it, _, args| {
            let indent = match arg(args, 2) {
                Value::Number(n) => " ".repeat(n.clamp(0.0, 10.0) as usize),
                Value::Str(s) => s.chars().take(10).collect(),
                _ => String::new(),
            };
            let mut out = String::new();
            let wrote = it.json_write(&arg(args, 0), &indent, "", &mut out)?;
            Ok(if wrote {
                Value::Str(out.into())
            } else {
                Value::Undefined
            })
        });
        self.set_native(&json, "parse", |it, _, args| {
            let text = it.to_str(&arg(args, 0));
            let mut p = JsonParser {
                s: text.as_bytes(),
                i: 0,
            };
            let v = p.value(it);
            match v {
                Some(v)
                    if {
                        p.ws();
                        p.i == p.s.len()
                    } =>
                {
                    Ok(v)
                }
                _ => {
                    let at = p.i.min(text.len());
                    let tok =
                        text[at..]
                            .chars()
                            .next()
                            .map_or("end of JSON input".to_string(), |c| {
                                format!(
                                    "token '{c}', \"{}\" is not valid JSON",
                                    &text[..text.len().min(at + 10)]
                                )
                            });
                    it.throw_error("SyntaxError", &format!("Unexpected {tok}"))
                }
            }
        });
        g.declare("JSON", Value::Object(json), true);
    }

    /// Serialize per JSON.stringify; returns false when the value has no JSON form (undefined/function).
    fn json_write(&mut self, v: &Value, indent: &str, cur: &str, out: &mut String) -> R<bool> {
        match v {
            Value::Undefined => Ok(false),
            Value::Null => {
                out.push_str("null");
                Ok(true)
            }
            Value::Bool(b) => {
                out.push_str(if *b { "true" } else { "false" });
                Ok(true)
            }
            Value::Number(n) => {
                out.push_str(&if n.is_finite() {
                    number_to_string(*n)
                } else {
                    "null".into()
                });
                Ok(true)
            }
            Value::Str(s) => {
                json_quote(s, out);
                Ok(true)
            }
            Value::Object(o) => {
                if v.is_callable() {
                    return Ok(false);
                }
                if let Ok(f) = self.get(v, "toJSON") {
                    if f.is_callable() {
                        let r = self.call(&f, v, &[])?;
                        return self.json_write(&r, indent, cur, out);
                    }
                }
                let inner = format!("{cur}{indent}");
                let (open_nl, sep, close_nl) = if indent.is_empty() {
                    (String::new(), ",".to_string(), String::new())
                } else {
                    (
                        format!("\n{inner}"),
                        format!(",\n{inner}"),
                        format!("\n{cur}"),
                    )
                };
                if matches!(&*o.kind.borrow(), Kind::Array(_)) {
                    let items = self.array_items(v);
                    if items.is_empty() {
                        out.push_str("[]");
                        return Ok(true);
                    }
                    out.push('[');
                    out.push_str(&open_nl);
                    for (i, item) in items.iter().enumerate() {
                        if i > 0 {
                            out.push_str(&sep);
                        }
                        if !self.json_write(item, indent, &inner, out)? {
                            out.push_str("null");
                        }
                    }
                    out.push_str(&close_nl);
                    out.push(']');
                    return Ok(true);
                }
                let mut first = true;
                let mut body = String::new();
                for k in self.own_keys(v) {
                    let val = self.get(v, &k)?;
                    let mut piece = String::new();
                    if self.json_write(&val, indent, &inner, &mut piece)? {
                        if !first {
                            body.push_str(&sep);
                        }
                        first = false;
                        json_quote(&k, &mut body);
                        body.push(':');
                        if !indent.is_empty() {
                            body.push(' ');
                        }
                        body.push_str(&piece);
                    }
                }
                if first {
                    out.push_str("{}");
                } else {
                    out.push('{');
                    out.push_str(&open_nl);
                    out.push_str(&body);
                    out.push_str(&close_nl);
                    out.push('}');
                }
                Ok(true)
            }
        }
    }
}

pub(crate) fn json_quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct JsonParser<'a> {
    s: &'a [u8],
    i: usize,
}

impl JsonParser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn lit(&mut self, word: &str) -> bool {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            true
        } else {
            false
        }
    }

    fn string(&mut self) -> Option<String> {
        self.i += 1; // opening quote
        let mut out: Vec<u8> = Vec::new();
        while self.i < self.s.len() {
            match self.s[self.i] {
                b'"' => {
                    self.i += 1;
                    return String::from_utf8(out).ok();
                }
                b'\\' => {
                    self.i += 1;
                    let c = *self.s.get(self.i)?;
                    self.i += 1;
                    match c {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'/' | b'\\' | b'"' => out.push(c),
                        b'u' => {
                            let hex = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                            let code = u32::from_str_radix(hex, 16).ok()?;
                            self.i += 4;
                            let ch = char::from_u32(code).unwrap_or('\u{FFFD}'); // ponytail: lone surrogates become U+FFFD
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return None,
                    }
                }
                c if c < 0x20 => return None,
                c => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
        None
    }

    fn value(&mut self, it: &mut Interp) -> Option<Value> {
        self.ws();
        match *self.s.get(self.i)? {
            b'{' => {
                self.i += 1;
                let obj = it.plain();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Some(Value::Object(obj));
                }
                loop {
                    self.ws();
                    if *self.s.get(self.i)? != b'"' {
                        return None;
                    }
                    let k = self.string()?;
                    self.ws();
                    if *self.s.get(self.i)? != b':' {
                        return None;
                    }
                    self.i += 1;
                    let v = self.value(it)?;
                    put(&obj, &k, v);
                    self.ws();
                    match *self.s.get(self.i)? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Some(Value::Object(obj));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Some(it.array(items));
                }
                loop {
                    items.push(self.value(it)?);
                    self.ws();
                    match *self.s.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(it.array(items));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => self.string().map(|s| Value::Str(s.into())),
            b't' => self.lit("true").then_some(Value::Bool(true)),
            b'f' => self.lit("false").then_some(Value::Bool(false)),
            b'n' => self.lit("null").then_some(Value::Null),
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                while self.i < self.s.len()
                    && matches!(
                        self.s[self.i],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i])
                    .ok()?
                    .parse()
                    .ok()
                    .map(Value::Number)
            }
            _ => None,
        }
    }
}
