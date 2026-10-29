//! Map, Set and a minimal Symbol.

use crate::*;

/// A hashable stand-in for a value under SameValueZero (`NaN` equals `NaN`, `-0` equals `+0`).
#[derive(PartialEq, Eq, Hash)]
enum HKey {
    Undefined,
    Null,
    Bool(bool),
    Number(u64),
    Str(Rc<str>),
    Big(Rc<num_bigint::BigInt>),
    Object(usize),
}

fn hkey(v: &Value) -> HKey {
    match v {
        Value::Undefined | Value::Hole => HKey::Undefined,
        Value::Null => HKey::Null,
        Value::Bool(b) => HKey::Bool(*b),
        Value::Number(n) if n.is_nan() => HKey::Number(f64::NAN.to_bits()),
        Value::Number(n) => HKey::Number((if *n == 0.0 { 0.0 } else { *n }).to_bits()),
        Value::Str(s) => HKey::Str(s.clone()),
        Value::BigInt(b) => HKey::Big(b.clone()),
        Value::Object(o) => HKey::Object(Rc::as_ptr(o) as usize),
    }
}

/// Collections smaller than this are searched linearly.
const HASH_MIN: usize = 16;

/// `Map` storage: insertion-ordered entries plus a lazily built hash index.
#[derive(Default)]
pub(crate) struct MapData {
    pub(crate) entries: Vec<(Value, Value)>,
    index: Option<FxMap<HKey, usize>>,
}

impl MapData {
    fn find(&mut self, key: &Value) -> Option<usize> {
        if self.entries.len() < HASH_MIN {
            return self
                .entries
                .iter()
                .position(|(k, _)| same_value_zero(k, key));
        }
        let entries = &self.entries;
        let index = self.index.get_or_insert_with(|| {
            entries
                .iter()
                .enumerate()
                .map(|(i, (k, _))| (hkey(k), i))
                .collect()
        });
        index.get(&hkey(key)).copied()
    }
    pub(crate) fn get(&mut self, key: &Value) -> Option<Value> {
        self.find(key).map(|i| self.entries[i].1.clone())
    }
    pub(crate) fn has(&mut self, key: &Value) -> bool {
        self.find(key).is_some()
    }
    pub(crate) fn set(&mut self, key: Value, value: Value) {
        match self.find(&key) {
            Some(i) => self.entries[i].1 = value,
            None => {
                if let Some(index) = &mut self.index {
                    index.insert(hkey(&key), self.entries.len());
                }
                self.entries.push((key, value));
            }
        }
    }
    pub(crate) fn delete(&mut self, key: &Value) -> bool {
        let Some(i) = self.find(key) else {
            return false;
        };
        self.entries.remove(i);
        self.index = None;
        true
    }
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.index = None;
    }
}

/// `Set` storage, like [`MapData`].
#[derive(Default)]
pub(crate) struct SetData {
    pub(crate) items: Vec<Value>,
    index: Option<FxMap<HKey, usize>>,
}

impl SetData {
    fn find(&mut self, key: &Value) -> Option<usize> {
        if self.items.len() < HASH_MIN {
            return self.items.iter().position(|k| same_value_zero(k, key));
        }
        let items = &self.items;
        let index = self.index.get_or_insert_with(|| {
            items
                .iter()
                .enumerate()
                .map(|(i, k)| (hkey(k), i))
                .collect()
        });
        index.get(&hkey(key)).copied()
    }
    pub(crate) fn has(&mut self, key: &Value) -> bool {
        self.find(key).is_some()
    }
    pub(crate) fn add(&mut self, key: Value) {
        if self.find(&key).is_some() {
            return;
        }
        if let Some(index) = &mut self.index {
            index.insert(hkey(&key), self.items.len());
        }
        self.items.push(key);
    }
    pub(crate) fn delete(&mut self, key: &Value) -> bool {
        let Some(i) = self.find(key) else {
            return false;
        };
        self.items.remove(i);
        self.index = None;
        true
    }
    pub(crate) fn clear(&mut self) {
        self.items.clear();
        self.index = None;
    }
}

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
                    Kind::Array(items) => {
                        return Some(items.iter().map(|v| unhole(Some(v))).collect())
                    }
                    Kind::Set(set) => return Some(set.items.clone()),
                    Kind::Typed(t) => {
                        return Some(t.to_vec().into_iter().map(Value::Number).collect())
                    }
                    Kind::Map(map) => map.entries.clone(),
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
            let reg_for_key = registry.clone();
            self.set_native(s, "keyFor", move |it, _, args| {
                let Value::Object(sym) = arg(args, 0) else {
                    let shown = it.inspect(&arg(args, 0), false);
                    return it.throw_error("TypeError", &format!("{shown} is not a symbol"));
                };
                let found = reg_for_key
                    .props
                    .borrow()
                    .iter()
                    .find_map(|p| match &p.slot {
                        Slot::Data(Value::Object(o)) if Rc::ptr_eq(o, &sym) => Some(p.key.clone()),
                        _ => None,
                    });
                Ok(found.map_or(Value::Undefined, Value::Str))
            });
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
                "isConcatSpreadable",
                "match",
                "matchAll",
                "replace",
                "search",
                "split",
                "unscopables",
                "dispose",
                "asyncDispose",
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
                let mut entries = MapData::default();
                if let Some(items) = it.iter_protocol(&arg(args, 0))? {
                    for item in items {
                        let kv = it.iter_items(&item).unwrap_or_default();
                        let (k, v) = (
                            kv.first().cloned().unwrap_or(Value::Undefined),
                            kv.get(1).cloned().unwrap_or(Value::Undefined),
                        );
                        entries.set(k, v);
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
                .get(&k)
                .unwrap_or(Value::Undefined)))
        });
        self.set_native(&mp, "has", |it, this, args| {
            let k = arg(args, 0);
            Ok(Value::Bool(with_map!(this, it, |e| e.has(&k))))
        });
        self.set_native(&mp, "set", |it, this, args| {
            let (k, v) = (arg(args, 0), arg(args, 1));
            with_map!(this, it, |e| e.set(k, v));
            Ok(this.clone())
        });
        self.set_native(&mp, "delete", |it, this, args| {
            let k = arg(args, 0);
            Ok(Value::Bool(with_map!(this, it, |e| e.delete(&k))))
        });
        self.set_native(&mp, "clear", |it, this, _| {
            with_map!(this, it, |e| e.clear());
            Ok(Value::Undefined)
        });
        let size = self.native(|it, this, _| {
            Ok(Value::Number(
                with_map!(this, it, |e| e.entries.len()) as f64
            ))
        });
        define_accessor(&mp, "size", Some(size), None, false);
        self.set_native(&mp, "forEach", |it, this, args| {
            let entries = with_map!(this, it, |e| e.entries.clone());
            for (k, v) in entries {
                it.call(&arg(args, 0), &arg(args, 1), &[v, k, this.clone()])?;
            }
            Ok(Value::Undefined)
        });
        self.set_native(&mp, "keys", |it, this, _| {
            let e = with_map!(this, it, |e| e.entries.clone());
            Ok(it.iterator_array(e.into_iter().map(|(k, _)| k).collect()))
        });
        self.set_native(&mp, "values", |it, this, _| {
            let e = with_map!(this, it, |e| e.entries.clone());
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
                let mut items = SetData::default();
                for v in it.iter_protocol(&arg(args, 0))?.unwrap_or_default() {
                    items.add(v);
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
            Ok(Value::Bool(with_set!(this, it, |e| e.has(&k))))
        });
        self.set_native(&sp, "add", |it, this, args| {
            let k = arg(args, 0);
            with_set!(this, it, |e| e.add(k));
            Ok(this.clone())
        });
        self.set_native(&sp, "delete", |it, this, args| {
            let k = arg(args, 0);
            Ok(Value::Bool(with_set!(this, it, |e| e.delete(&k))))
        });
        self.set_native(&sp, "clear", |it, this, _| {
            with_set!(this, it, |e| e.clear());
            Ok(Value::Undefined)
        });
        let size = self
            .native(|it, this, _| Ok(Value::Number(with_set!(this, it, |e| e.items.len()) as f64)));
        define_accessor(&sp, "size", Some(size), None, false);
        self.set_native(&sp, "forEach", |it, this, args| {
            let items = with_set!(this, it, |e| e.items.clone());
            for v in items {
                it.call(&arg(args, 0), &arg(args, 1), &[v.clone(), v, this.clone()])?;
            }
            Ok(Value::Undefined)
        });
        for name in ["values", "keys"] {
            self.set_native(&sp, name, |it, this, _| {
                let e = with_set!(this, it, |e| e.items.clone());
                Ok(it.iterator_array(e))
            });
        }
        self.set_native(&sp, "entries", |it, this, _| {
            let e = with_set!(this, it, |e| e.items.clone());
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
