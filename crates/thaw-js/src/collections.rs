//! Map, Set and a minimal Symbol.

use crate::*;

pub(crate) fn same_value_zero(a: &Value, b: &Value) -> bool {
    strict_equals(a, b)
        || matches!((a, b), (Value::Number(x), Value::Number(y)) if x.is_nan() && y.is_nan())
}

impl Interp {
    /// An array that also behaves like an iterator (`next()` returns `{ value, done }`), so
    /// `map.keys().next().value` works while `for-of`/spread still see plain elements.
    pub(crate) fn iterator_array(&self, items: Vec<Value>) -> Value {
        let state = Rc::new((items.clone(), Cell::new(0usize)));
        let arr = self.array(items);
        if let Value::Object(o) = &arr {
            self.set_native(o, "next", move |it, _, _| {
                let (items, pos) = &*state;
                let result = it.plain();
                match items.get(pos.get()) {
                    Some(v) => {
                        pos.set(pos.get() + 1);
                        put(&result, "value", v.clone());
                        put(&result, "done", Value::Bool(false));
                    }
                    None => {
                        put(&result, "value", Value::Undefined);
                        put(&result, "done", Value::Bool(true));
                    }
                }
                Ok(Value::Object(result))
            });
        }
        arr
    }

    /// Elements of an iterable that the evaluator can enumerate without iterator objects.
    pub(crate) fn iter_items(&mut self, v: &Value) -> Option<Vec<Value>> {
        match v {
            Value::Str(s) => Some(
                s.chars()
                    .map(|c| Value::Str(c.to_string().into()))
                    .collect(),
            ),
            Value::Object(o) => {
                let pairs = match &*o.kind.borrow() {
                    Kind::Array(items) => return Some(items.clone()),
                    Kind::Set(items) => return Some(items.clone()),
                    Kind::Typed(t) => {
                        return Some(t.to_vec().into_iter().map(Value::Number).collect())
                    }
                    Kind::Map(entries) => entries.clone(),
                    _ => return None,
                };
                Some(
                    pairs
                        .into_iter()
                        .map(|(k, v)| self.array(vec![k, v]))
                        .collect(),
                )
            }
            _ => None,
        }
    }

    /// `iter_items`, extended with the `Symbol.iterator` protocol for user-defined iterables
    /// (collected eagerly).
    pub(crate) fn iter_protocol(&mut self, v: &Value) -> R<Option<Vec<Value>>> {
        self.iter_protocol_limit(v, None)
    }

    /// Like [`Interp::iter_protocol`], but pulls at most `limit` values from a user-defined iterator
    /// (closing it if it has more), as array destructuring does.
    pub(crate) fn iter_protocol_limit(
        &mut self,
        v: &Value,
        limit: Option<usize>,
    ) -> R<Option<Vec<Value>>> {
        if let Some(items) = self.iter_items(v) {
            return Ok(Some(items));
        }
        let Some((iter, next)) = self.get_iterator(v)? else {
            return Ok(None);
        };
        let mut out = Vec::new();
        loop {
            if limit.is_some_and(|n| out.len() >= n) {
                self.close_iterator(&iter);
                return Ok(Some(out));
            }
            let r = self.call(&next, &iter, &[])?;
            if !matches!(r, Value::Object(_)) {
                return self.throw_error("TypeError", "Iterator result is not an object");
            }
            if self.get(&r, "done")?.truthy() {
                return Ok(Some(out));
            }
            out.push(self.get(&r, "value")?);
        }
    }

    /// `(iterator, next)` of a `Symbol.iterator` iterable; `None` if it has no iterator method.
    pub(crate) fn get_iterator(&mut self, iterable: &Value) -> R<Option<(Value, Value)>> {
        if !matches!(iterable, Value::Object(_)) {
            return Ok(None);
        }
        let symbol = self
            .lookup(&self.global.clone(), "Symbol")
            .unwrap_or(Value::Undefined);
        let tag = self.get(&symbol, "iterator")?;
        let key = self.key_of(&tag);
        let method = self.get(iterable, &key)?;
        if !method.is_callable() {
            return Ok(None);
        }
        let iter = self.call(&method, iterable, &[])?;
        if !matches!(iter, Value::Object(_)) {
            return self.throw_error(
                "TypeError",
                "Result of the Symbol.iterator method is not an object",
            );
        }
        let next = self.get(&iter, "next")?;
        Ok(Some((iter, next)))
    }

    /// `iterator.return()` when an iteration ends early; errors from it are dropped.
    pub(crate) fn close_iterator(&mut self, iter: &Value) {
        if let Ok(ret) = self.get(iter, "return") {
            if ret.is_callable() {
                let _ = self.call(&ret, iter, &[]);
            }
        }
    }

    /// Property key for a value (symbols map to reserved `@@sym:` keys).
    pub(crate) fn key_of(&mut self, v: &Value) -> Rc<str> {
        if let Value::Object(o) = v {
            if let Kind::Symbol { id, .. } = &*o.kind.borrow() {
                return format!("@@sym:{id}").into();
            }
        }
        self.to_str(v)
    }

    pub(crate) fn install_collections(&mut self) {
        let g = self.global.clone();
        // ----- Symbol -----
        let sym_proto = self.plain();
        let symbol = self.native({
            let sp = sym_proto.clone();
            move |it, _, args| {
                it.next_symbol += 1;
                let desc = match arg(args, 0) {
                    Value::Undefined => None,
                    v => Some(it.to_str(&v)),
                };
                let o = new_obj(
                    Some(sp.clone()),
                    Kind::Symbol {
                        id: it.next_symbol,
                        desc: desc.clone(),
                    },
                );
                put_hidden(&o, "description", desc.map_or(Value::Undefined, Value::Str));
                let id = it.next_symbol;
                it.rt.symbols.insert(id, o.clone());
                Ok(Value::Object(o))
            }
        });
        if let Value::Object(s) = &symbol {
            put_hidden(s, "prototype", Value::Object(sym_proto.clone()));
            let registry = self.plain();
            let sp = sym_proto.clone();
            self.set_native(s, "for", move |it, _, args| {
                let key = it.to_str(&arg(args, 0));
                if let Some((Slot::Data(v), _)) = find_own(&registry, &key) {
                    return Ok(v);
                }
                it.next_symbol += 1;
                let o = new_obj(
                    Some(sp.clone()),
                    Kind::Symbol {
                        id: it.next_symbol,
                        desc: Some(key.clone()),
                    },
                );
                put_hidden(&o, "description", Value::Str(key.clone()));
                put_hidden(&registry, &key, Value::Object(o.clone()));
                let id = it.next_symbol;
                it.rt.symbols.insert(id, o.clone());
                Ok(Value::Object(o))
            });
            for name in [
                "iterator",
                "asyncIterator",
                "hasInstance",
                "toPrimitive",
                "toStringTag",
                "species",
            ] {
                self.next_symbol += 1;
                let desc: Rc<str> = format!("Symbol.{name}").into();
                let o = new_obj(
                    Some(sym_proto.clone()),
                    Kind::Symbol {
                        id: self.next_symbol,
                        desc: Some(desc.clone()),
                    },
                );
                put_hidden(&o, "description", Value::Str(desc));
                self.rt.symbols.insert(self.next_symbol, o.clone());
                put_hidden(s, name, Value::Object(o));
            }
        }
        self.set_native(&sym_proto, "toString", |_, this, _| {
            Ok(Value::Str(match this {
                Value::Object(o) => match &*o.kind.borrow() {
                    Kind::Symbol { desc, .. } => {
                        format!("Symbol({})", desc.as_deref().unwrap_or("")).into()
                    }
                    _ => "Symbol()".into(),
                },
                _ => "Symbol()".into(),
            }))
        });
        g.declare("Symbol", symbol, true);

        // ----- Map -----
        let mp = self.plain();
        let map = self.native({
            let mp = mp.clone();
            move |it, _, args| {
                let mut entries: Vec<(Value, Value)> = Vec::new();
                if let Some(items) = it.iter_protocol(&arg(args, 0))? {
                    for item in items {
                        let kv = it.iter_items(&item).unwrap_or_default();
                        let (k, v) = (
                            kv.first().cloned().unwrap_or(Value::Undefined),
                            kv.get(1).cloned().unwrap_or(Value::Undefined),
                        );
                        match entries.iter_mut().find(|(ek, _)| same_value_zero(ek, &k)) {
                            Some(e) => e.1 = v,
                            None => entries.push((k, v)),
                        }
                    }
                }
                Ok(Value::Object(new_obj(Some(mp.clone()), Kind::Map(entries))))
            }
        });
        if let Value::Object(m) = &map {
            put_hidden(m, "prototype", Value::Object(mp.clone()));
            put_hidden(&mp, "constructor", map.clone());
        }
        g.declare("Map", map, true);
        macro_rules! with_map {
            ($this:expr, $it:expr, |$e:ident| $body:expr) => {
                match $this {
                    Value::Object(o) => match &mut *o.kind.borrow_mut() {
                        Kind::Map($e) => $body,
                        _ => {
                            return $it.throw_error(
                                "TypeError",
                                "Method Map.prototype called on incompatible receiver",
                            )
                        }
                    },
                    _ => {
                        return $it.throw_error(
                            "TypeError",
                            "Method Map.prototype called on incompatible receiver",
                        )
                    }
                }
            };
        }
        self.set_native(&mp, "get", |it, this, args| {
            let k = arg(args, 0);
            Ok(with_map!(this, it, |e| e
                .iter()
                .find(|(ek, _)| same_value_zero(ek, &k))
                .map_or(Value::Undefined, |(_, v)| v.clone())))
        });
        self.set_native(&mp, "has", |it, this, args| {
            let k = arg(args, 0);
            Ok(Value::Bool(with_map!(this, it, |e| e
                .iter()
                .any(|(ek, _)| same_value_zero(ek, &k)))))
        });
        self.set_native(&mp, "set", |it, this, args| {
            let (k, v) = (arg(args, 0), arg(args, 1));
            with_map!(
                this,
                it,
                |e| match e.iter_mut().find(|(ek, _)| same_value_zero(ek, &k)) {
                    Some(slot) => slot.1 = v,
                    None => e.push((k, v)),
                }
            );
            Ok(this.clone())
        });
        self.set_native(&mp, "delete", |it, this, args| {
            let k = arg(args, 0);
            Ok(Value::Bool(with_map!(this, it, |e| {
                let before = e.len();
                e.retain(|(ek, _)| !same_value_zero(ek, &k));
                e.len() != before
            })))
        });
        self.set_native(&mp, "clear", |it, this, _| {
            with_map!(this, it, |e| e.clear());
            Ok(Value::Undefined)
        });
        let size =
            self.native(|it, this, _| Ok(Value::Number(with_map!(this, it, |e| e.len()) as f64)));
        define_accessor(&mp, "size", Some(size), None, false);
        self.set_native(&mp, "forEach", |it, this, args| {
            let entries = with_map!(this, it, |e| e.clone());
            for (k, v) in entries {
                it.call(&arg(args, 0), &arg(args, 1), &[v, k, this.clone()])?;
            }
            Ok(Value::Undefined)
        });
        self.set_native(&mp, "keys", |it, this, _| {
            let e = with_map!(this, it, |e| e.clone());
            Ok(it.iterator_array(e.into_iter().map(|(k, _)| k).collect()))
        });
        self.set_native(&mp, "values", |it, this, _| {
            let e = with_map!(this, it, |e| e.clone());
            Ok(it.iterator_array(e.into_iter().map(|(_, v)| v).collect()))
        });
        self.set_native(&mp, "entries", |it, this, _| {
            let items = it.iter_items(this).unwrap_or_default();
            Ok(it.iterator_array(items))
        });

        // ----- Set -----
        let sp = self.plain();
        let set = self.native({
            let sp = sp.clone();
            move |it, _, args| {
                let mut items: Vec<Value> = Vec::new();
                for v in it.iter_protocol(&arg(args, 0))?.unwrap_or_default() {
                    if !items.iter().any(|x| same_value_zero(x, &v)) {
                        items.push(v);
                    }
                }
                Ok(Value::Object(new_obj(Some(sp.clone()), Kind::Set(items))))
            }
        });
        if let Value::Object(s) = &set {
            put_hidden(s, "prototype", Value::Object(sp.clone()));
            put_hidden(&sp, "constructor", set.clone());
        }
        g.declare("Set", set, true);
        macro_rules! with_set {
            ($this:expr, $it:expr, |$e:ident| $body:expr) => {
                match $this {
                    Value::Object(o) => match &mut *o.kind.borrow_mut() {
                        Kind::Set($e) => $body,
                        _ => {
                            return $it.throw_error(
                                "TypeError",
                                "Method Set.prototype called on incompatible receiver",
                            )
                        }
                    },
                    _ => {
                        return $it.throw_error(
                            "TypeError",
                            "Method Set.prototype called on incompatible receiver",
                        )
                    }
                }
            };
        }
        self.set_native(&sp, "has", |it, this, args| {
            let k = arg(args, 0);
            Ok(Value::Bool(with_set!(this, it, |e| e
                .iter()
                .any(|x| same_value_zero(x, &k)))))
        });
        self.set_native(&sp, "add", |it, this, args| {
            let k = arg(args, 0);
            with_set!(this, it, |e| if !e.iter().any(|x| same_value_zero(x, &k)) {
                e.push(k)
            });
            Ok(this.clone())
        });
        self.set_native(&sp, "delete", |it, this, args| {
            let k = arg(args, 0);
            Ok(Value::Bool(with_set!(this, it, |e| {
                let before = e.len();
                e.retain(|x| !same_value_zero(x, &k));
                e.len() != before
            })))
        });
        self.set_native(&sp, "clear", |it, this, _| {
            with_set!(this, it, |e| e.clear());
            Ok(Value::Undefined)
        });
        let size =
            self.native(|it, this, _| Ok(Value::Number(with_set!(this, it, |e| e.len()) as f64)));
        define_accessor(&sp, "size", Some(size), None, false);
        self.set_native(&sp, "forEach", |it, this, args| {
            let items = with_set!(this, it, |e| e.clone());
            for v in items {
                it.call(&arg(args, 0), &arg(args, 1), &[v.clone(), v, this.clone()])?;
            }
            Ok(Value::Undefined)
        });
        for name in ["values", "keys"] {
            self.set_native(&sp, name, |it, this, _| {
                let e = with_set!(this, it, |e| e.clone());
                Ok(it.iterator_array(e))
            });
        }
        self.set_native(&sp, "entries", |it, this, _| {
            let e = with_set!(this, it, |e| e.clone());
            Ok(it.iterator_array(
                e.into_iter()
                    .map(|v| it.array(vec![v.clone(), v]))
                    .collect(),
            ))
        });

        // ----- URI helpers -----
        for (name, keep) in [
            ("encodeURIComponent", "-_.!~*'()"),
            ("encodeURI", "-_.!~*'();/?:@&=+$,#"),
            ("escape", "@*_+-./"),
        ] {
            g.declare(
                name,
                self.native(move |it, _, args| {
                    let s = it.to_str(&arg(args, 0));
                    let mut out = String::new();
                    for c in s.chars() {
                        if c.is_ascii_alphanumeric() || keep.contains(c) {
                            out.push(c);
                        } else if name == "escape" && (c as u32) < 256 {
                            out.push_str(&format!("%{:02X}", c as u32));
                        } else if name == "escape" {
                            out.push_str(&format!("%u{:04X}", c as u32));
                        } else {
                            let mut buf = [0u8; 4];
                            for b in c.encode_utf8(&mut buf).bytes() {
                                out.push_str(&format!("%{b:02X}"));
                            }
                        }
                    }
                    Ok(Value::Str(out.into()))
                }),
                true,
            );
        }
        for (name, reserved) in [
            ("decodeURIComponent", ""),
            ("decodeURI", ";/?:@&=+$,#"),
            ("unescape", ""),
        ] {
            g.declare(
                name,
                self.native(move |it, _, args| {
                    let s = it.to_str(&arg(args, 0));
                    let bytes = s.as_bytes();
                    let mut out: Vec<u8> = Vec::new();
                    let mut i = 0;
                    while i < bytes.len() {
                        if bytes[i] == b'%' {
                            if name == "unescape" && bytes.get(i + 1) == Some(&b'u') {
                                if let Some(code) = s
                                    .get(i + 2..i + 6)
                                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                                {
                                    let mut buf = [0u8; 4];
                                    out.extend_from_slice(
                                        char::from_u32(code)
                                            .unwrap_or('\u{FFFD}')
                                            .encode_utf8(&mut buf)
                                            .as_bytes(),
                                    );
                                    i += 6;
                                    continue;
                                }
                            }
                            if let Some(b) = s
                                .get(i + 1..i + 3)
                                .and_then(|h| u8::from_str_radix(h, 16).ok())
                            {
                                if name != "unescape" && reserved.contains(b as char) {
                                    out.extend_from_slice(&bytes[i..i + 3]);
                                } else if name == "unescape" {
                                    let mut buf = [0u8; 4];
                                    out.extend_from_slice(
                                        (b as char).encode_utf8(&mut buf).as_bytes(),
                                    );
                                } else {
                                    out.push(b);
                                }
                                i += 3;
                                continue;
                            }
                            if name != "unescape" {
                                return it.throw_error("URIError", "URI malformed");
                            }
                        }
                        out.push(bytes[i]);
                        i += 1;
                    }
                    match String::from_utf8(out) {
                        Ok(s) => Ok(Value::Str(s.into())),
                        Err(_) => it.throw_error("URIError", "URI malformed"),
                    }
                }),
                true,
            );
        }
    }
}
