//! `JSON.stringify` (replacer function / allow-list, `toJSON`, gap, cycle detection) and
//! `JSON.parse` (reviver, V8-style error messages).

use crate::builtins::json_quote;
use crate::host::Native;
use crate::*;

struct Ser {
    replacer: Option<Value>,
    allow: Option<Vec<Rc<str>>>,
    gap: String,
    /// Objects being serialized with the key that reached each one.
    stack: Vec<(Obj, Rc<str>)>,
}

impl Interp {
    pub(crate) fn install_json(&mut self, json: &Obj) {
        self.set_native(json, "stringify", |it, _, args| {
            let gap = match arg(args, 2) {
                Value::Number(n) => " ".repeat(n.clamp(0.0, 10.0) as usize),
                Value::Str(s) => s.chars().take(10).collect(),
                _ => String::new(),
            };
            let (mut replacer, mut allow) = (None, None);
            match arg(args, 1) {
                f if f.is_callable() => replacer = Some(f),
                Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_)) => {
                    let mut keys: Vec<Rc<str>> = Vec::new();
                    for item in it.array_items(&Value::Object(o.clone())) {
                        if matches!(item, Value::Str(_) | Value::Number(_)) {
                            let k = it.to_str(&item);
                            if !keys.contains(&k) {
                                keys.push(k);
                            }
                        }
                    }
                    allow = Some(keys);
                }
                _ => {}
            }
            let mut ser = Ser {
                replacer,
                allow,
                gap,
                stack: Vec::new(),
            };
            let holder = it.plain();
            put(&holder, "", arg(args, 0));
            let mut out = String::new();
            let wrote = it.json_ser(
                &Value::Object(holder),
                "",
                arg(args, 0),
                &mut ser,
                "",
                &mut out,
            )?;
            Ok(if wrote {
                Value::Str(out.into())
            } else {
                Value::Undefined
            })
        });
        self.set_native(json, "parse", |it, _, args| {
            let text = it.to_str(&arg(args, 0));
            let mut p = Parser {
                s: text.as_bytes(),
                i: 0,
            };
            let v = match p.top(it) {
                Ok(v) => v,
                Err(msg) => return it.throw_error("SyntaxError", &msg),
            };
            let reviver = arg(args, 1);
            if reviver.is_callable() {
                let root = it.plain();
                put(&root, "", v);
                return it.revive(&Value::Object(root), "", &reviver);
            }
            Ok(v)
        });
    }

    fn revive(&mut self, holder: &Value, key: &str, reviver: &Value) -> R<Value> {
        let val = self.get(holder, key)?;
        if let Value::Object(o) = &val {
            let is_arr = matches!(&*o.kind.borrow(), Kind::Array(_));
            let keys: Vec<Rc<str>> = self.own_keys(&val);
            for k in keys {
                let nv = self.revive(&val, &k, reviver)?;
                if matches!(nv, Value::Undefined) {
                    if !is_arr {
                        o.remove_prop(&k);
                    }
                } else {
                    self.set(&val, &k, nv)?;
                }
            }
        }
        self.call(reviver, holder, &[Value::Str(key.into()), val])
    }

    fn ctor_name(&mut self, o: &Obj) -> String {
        let v = Value::Object(o.clone());
        match self.get(&v, "constructor") {
            Ok(c) => match self.get(&c, "name") {
                Ok(Value::Str(n)) if !n.is_empty() => n.to_string(),
                _ => "Object".into(),
            },
            _ => "Object".into(),
        }
    }

    fn circular_error<T>(&mut self, ser: &Ser, at: usize, key: &str, is_arr: bool) -> R<T> {
        let start = ser.stack[at].0.clone();
        let mut msg = format!(
            "Converting circular structure to JSON\n    --> starting at object with constructor '{}'",
            self.ctor_name(&start)
        );
        let label = |k: &str, arr: bool| {
            if arr {
                format!("index {k}")
            } else {
                format!("property '{k}'")
            }
        };
        let rest: Vec<(Obj, Rc<str>)> = ser.stack[at + 1..].to_vec();
        let mut parent_arr = matches!(&*start.kind.borrow(), Kind::Array(_));
        for (o, k) in rest {
            let name = self.ctor_name(&o);
            msg.push_str(&format!(
                "\n    |     {} -> object with constructor '{}'",
                label(&k, parent_arr),
                name
            ));
            parent_arr = matches!(&*o.kind.borrow(), Kind::Array(_));
        }
        msg.push_str(&format!(
            "\n    --- {} closes the circle",
            label(key, is_arr)
        ));
        self.throw_error("TypeError", &msg)
    }

    #[allow(clippy::too_many_arguments)]
    fn json_ser(
        &mut self,
        holder: &Value,
        key: &str,
        mut v: Value,
        ser: &mut Ser,
        cur: &str,
        out: &mut String,
    ) -> R<bool> {
        if matches!(v, Value::Object(_) | Value::BigInt(_)) {
            let f = self.get(&v, "toJSON")?;
            if f.is_callable() {
                v = self.call(&f, &v, &[Value::Str(key.into())])?;
            }
        }
        if let Some(r) = ser.replacer.clone() {
            v = self.call(&r, holder, &[Value::Str(key.into()), v])?;
        }
        match &v {
            Value::Undefined | Value::Hole => Ok(false),
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
            Value::BigInt(_) => {
                self.throw_error("TypeError", "Do not know how to serialize a BigInt")
            }
            Value::Object(o) => {
                if v.is_callable() || matches!(&*o.kind.borrow(), Kind::Symbol { .. }) {
                    return Ok(false);
                }
                let wrapped = match &*o.kind.borrow() {
                    Kind::Wrapper(p) => Some(p.clone()),
                    _ => None,
                };
                if let Some(p) = wrapped {
                    return self.json_ser(holder, key, p, ser, cur, out);
                }
                let is_arr = matches!(&*o.kind.borrow(), Kind::Array(_));
                if let Some(at) = ser.stack.iter().position(|(s, _)| Rc::ptr_eq(s, o)) {
                    let parent_arr = ser
                        .stack
                        .last()
                        .is_some_and(|(p, _)| matches!(&*p.kind.borrow(), Kind::Array(_)));
                    return self.circular_error(ser, at, key, parent_arr);
                }
                let inner = format!("{cur}{}", ser.gap);
                let pretty = !ser.gap.is_empty();
                ser.stack.push((o.clone(), key.into()));
                let mut pieces: Vec<String> = Vec::new();
                if is_arr {
                    for (i, item) in self.array_items(&v).into_iter().enumerate() {
                        let mut piece = String::new();
                        let k = i.to_string();
                        if !self.json_ser(&v, &k, item, ser, &inner, &mut piece)? {
                            piece = "null".into();
                        }
                        pieces.push(piece);
                    }
                } else {
                    let keys = match &ser.allow {
                        Some(a) => a
                            .iter()
                            .filter(|k| find_in_chain(o, k).is_some())
                            .cloned()
                            .collect(),
                        None => self.own_keys(&v),
                    };
                    for k in keys {
                        let val = self.get(&v, &k)?;
                        let mut piece = String::new();
                        if self.json_ser(&v, &k, val, ser, &inner, &mut piece)? {
                            let mut entry = String::new();
                            json_quote(&k, &mut entry);
                            entry.push(':');
                            if pretty {
                                entry.push(' ');
                            }
                            entry.push_str(&piece);
                            pieces.push(entry);
                        }
                    }
                }
                ser.stack.pop();
                let (open, close) = if is_arr { ('[', ']') } else { ('{', '}') };
                if pieces.is_empty() {
                    out.push(open);
                    out.push(close);
                } else if pretty {
                    out.push(open);
                    out.push_str(&format!("\n{inner}"));
                    out.push_str(&pieces.join(&format!(",\n{inner}")));
                    out.push_str(&format!("\n{cur}"));
                    out.push(close);
                } else {
                    out.push(open);
                    out.push_str(&pieces.join(","));
                    out.push(close);
                }
                Ok(true)
            }
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn pos_msg(&self, what: &str, at: usize) -> String {
        let before = &self.s[..at.min(self.s.len())];
        let line = before.iter().filter(|b| **b == b'\n').count() + 1;
        let col = before.iter().rev().take_while(|b| **b != b'\n').count() + 1;
        format!("{what} in JSON at position {at} (line {line} column {col})")
    }

    /// V8's "Unexpected token" message with the surrounding source.
    fn unexpected(&self) -> String {
        let Some(&b) = self.s.get(self.i) else {
            return "Unexpected end of JSON input".into();
        };
        let text = String::from_utf8_lossy(self.s);
        let chars: Vec<char> = text.chars().collect();
        let at = String::from_utf8_lossy(&self.s[..self.i]).chars().count();
        let tok = chars.get(at).copied().unwrap_or(b as char);
        if chars.len() >= 21 {
            let start = at.saturating_sub(10);
            let end = (at + 10).min(chars.len());
            let snippet: String = chars[start..end].iter().collect();
            format!(
                "Unexpected token '{tok}', {}\"{snippet}\"{} is not valid JSON",
                if start > 0 { "..." } else { "" },
                if end < chars.len() { "..." } else { "" }
            )
        } else {
            format!("Unexpected token '{tok}', \"{text}\" is not valid JSON")
        }
    }

    fn top(&mut self, it: &mut Interp) -> Result<Value, String> {
        let v = self.value(it, 0)?;
        self.ws();
        if self.i < self.s.len() {
            return Err(self.pos_msg("Unexpected non-whitespace character after JSON", self.i));
        }
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else {
                return Err(self.pos_msg("Unterminated string", self.i));
            };
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(String::from_utf8_lossy(&out).into_owned());
                }
                b'\\' => {
                    self.i += 1;
                    let Some(&e) = self.s.get(self.i) else {
                        return Err(self.pos_msg("Unterminated string", self.i));
                    };
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'/' | b'\\' | b'"' => out.push(e),
                        b'u' => {
                            let hex = self
                                .s
                                .get(self.i..self.i + 4)
                                .and_then(|h| std::str::from_utf8(h).ok())
                                .filter(|h| h.chars().all(|c| c.is_ascii_hexdigit()));
                            let Some(hex) = hex else {
                                return Err(self.pos_msg("Bad Unicode escape", self.i));
                            };
                            let mut code = u32::from_str_radix(hex, 16).unwrap_or(0xFFFD);
                            self.i += 4;
                            // a surrogate pair written as two escapes
                            if (0xD800..0xDC00).contains(&code)
                                && self.s.get(self.i..self.i + 2) == Some(b"\\u")
                            {
                                if let Some(lo) = self
                                    .s
                                    .get(self.i + 2..self.i + 6)
                                    .and_then(|h| std::str::from_utf8(h).ok())
                                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                                    .filter(|lo| (0xDC00..0xE000).contains(lo))
                                {
                                    code = 0x10000 + ((code - 0xD800) << 10) + (lo - 0xDC00);
                                    self.i += 6;
                                }
                            }
                            // ponytail: lone surrogates become U+FFFD
                            let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => {
                            self.i -= 1;
                            return Err(self.pos_msg("Bad escaped character", self.i));
                        }
                    }
                }
                c if c < 0x20 => {
                    return Err(self.pos_msg("Bad control character in string literal", self.i))
                }
                c => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }

    fn value(&mut self, it: &mut Interp, depth: usize) -> Result<Value, String> {
        if depth > 5000 {
            return Err("Maximum call stack size exceeded".into());
        }
        self.ws();
        match self.s.get(self.i).copied() {
            None => Err("Unexpected end of JSON input".into()),
            Some(b'{') => {
                self.i += 1;
                let obj = it.plain();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Value::Object(obj));
                }
                let mut first = true;
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return Err(if self.i >= self.s.len() {
                            "Unexpected end of JSON input".into()
                        } else if first {
                            self.pos_msg("Expected property name or '}'", self.i)
                        } else {
                            self.pos_msg("Expected double-quoted property name", self.i)
                        });
                    }
                    first = false;
                    let k = self.string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return Err(if self.i >= self.s.len() {
                            "Unexpected end of JSON input".into()
                        } else {
                            self.pos_msg("Expected ':' after property name", self.i)
                        });
                    }
                    self.i += 1;
                    let v = self.value(it, depth + 1)?;
                    put(&obj, &k, v);
                    self.ws();
                    match self.s.get(self.i).copied() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Object(obj));
                        }
                        None => return Err("Unexpected end of JSON input".into()),
                        _ => {
                            return Err(
                                self.pos_msg("Expected ',' or '}' after property value", self.i)
                            )
                        }
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(it.array(items));
                }
                loop {
                    items.push(self.value(it, depth + 1)?);
                    self.ws();
                    match self.s.get(self.i).copied() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(it.array(items));
                        }
                        None => return Err("Unexpected end of JSON input".into()),
                        _ => {
                            return Err(
                                self.pos_msg("Expected ',' or ']' after array element", self.i)
                            )
                        }
                    }
                }
            }
            Some(b'"') => self.string().map(|s| Value::Str(s.into())),
            Some(b't') => self.word("true", Value::Bool(true)),
            Some(b'f') => self.word("false", Value::Bool(false)),
            Some(b'n') => self.word("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.unexpected()),
        }
    }

    /// The same grammar as [`Parser::value`], producing host data directly (no interpreter needed).
    fn native(&mut self, depth: usize) -> Result<Native, String> {
        if depth > 5000 {
            return Err("Maximum call stack size exceeded".into());
        }
        self.ws();
        match self.s.get(self.i).copied() {
            None => Err("Unexpected end of JSON input".into()),
            Some(b'{') => {
                self.i += 1;
                let mut fields: Vec<(String, Native)> = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Native::Object(fields));
                }
                let mut first = true;
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return Err(if self.i >= self.s.len() {
                            "Unexpected end of JSON input".into()
                        } else if first {
                            self.pos_msg("Expected property name or '}'", self.i)
                        } else {
                            self.pos_msg("Expected double-quoted property name", self.i)
                        });
                    }
                    first = false;
                    let k = self.string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return Err(if self.i >= self.s.len() {
                            "Unexpected end of JSON input".into()
                        } else {
                            self.pos_msg("Expected ':' after property name", self.i)
                        });
                    }
                    self.i += 1;
                    let v = self.native(depth + 1)?;
                    match fields.iter_mut().find(|(name, _)| *name == k) {
                        Some(slot) => slot.1 = v,
                        None => fields.push((k, v)),
                    }
                    self.ws();
                    match self.s.get(self.i).copied() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Native::Object(fields));
                        }
                        None => return Err("Unexpected end of JSON input".into()),
                        _ => {
                            return Err(
                                self.pos_msg("Expected ',' or '}' after property value", self.i)
                            )
                        }
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Native::Array(items));
                }
                loop {
                    items.push(self.native(depth + 1)?);
                    self.ws();
                    match self.s.get(self.i).copied() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Native::Array(items));
                        }
                        None => return Err("Unexpected end of JSON input".into()),
                        _ => {
                            return Err(
                                self.pos_msg("Expected ',' or ']' after array element", self.i)
                            )
                        }
                    }
                }
            }
            Some(b'"') => self.string().map(Native::Str),
            Some(b't') => self.word("true", Value::Null).map(|_| Native::Bool(true)),
            Some(b'f') => self.word("false", Value::Null).map(|_| Native::Bool(false)),
            Some(b'n') => self.word("null", Value::Null).map(|_| Native::Null),
            Some(b'-' | b'0'..=b'9') => self.number_f64().map(Native::Number),
            Some(_) => Err(self.unexpected()),
        }
    }

    fn word(&mut self, w: &str, v: Value) -> Result<Value, String> {
        for (k, b) in w.bytes().enumerate() {
            if self.s.get(self.i + k) != Some(&b) {
                self.i += k;
                return Err(self.unexpected());
            }
        }
        self.i += w.len();
        Ok(v)
    }

    fn number(&mut self) -> Result<Value, String> {
        self.number_f64().map(Value::Number)
    }

    fn number_f64(&mut self) -> Result<f64, String> {
        let start = self.i;
        if self.s[self.i] == b'-' {
            self.i += 1;
        }
        let digits = |p: &mut Self| {
            let s0 = p.i;
            while p.s.get(p.i).is_some_and(|b| b.is_ascii_digit()) {
                p.i += 1;
            }
            p.i > s0
        };
        match self.s.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                digits(self);
            }
            None => {
                return Err(
                    "No number after minus sign in JSON at position 1 (line 1 column 2)".into(),
                )
            }
            _ => return Err(self.pos_msg("No number after minus sign", self.i)),
        }
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            if !digits(self) {
                return Err(self.pos_msg("Unterminated fractional number", self.i));
            }
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !digits(self) {
                return Err(self.pos_msg("Exponent part is missing a number", self.i));
            }
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| self.unexpected())
    }
}

/// Parse JSON text into host data without an interpreter (the typed-call ABI parses on every call).
pub(crate) fn parse_native(text: &str) -> Result<Native, String> {
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
    };
    let v = p.native(0)?;
    p.ws();
    if p.i < p.s.len() {
        return Err(p.pos_msg("Unexpected non-whitespace character after JSON", p.i));
    }
    Ok(v)
}
