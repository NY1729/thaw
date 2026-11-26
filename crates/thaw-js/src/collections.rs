//! Map, Set and a minimal Symbol.

use crate::*;

/// A hashable stand-in for a value under SameValueZero (`NaN` equals `NaN`, `-0` equals `+0`).
#[derive(PartialEq, Eq, Hash)]
enum HKey {
    Undefined,
    Null,
    Bool(bool),
    Number(u64),
    Str(JsStr),
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

/// A deleted slot of `MapData::entries` / `SetData::items` (`delete` is O(1); `compact` drops the dead slots).
fn dead(v: &Value) -> bool {
    matches!(v, Value::Hole)
}

/// `Hole` never is a user key (`hkey` already reads it as `undefined`), so it can mark a deleted slot.
fn live_key(k: Value) -> Value {
    if dead(&k) {
        Value::Undefined
    } else {
        k
    }
}

/// Live cursors move to where their entry lands once the `dead` slots before it are dropped.
fn remap_cursors(
    cursors: &mut Vec<Weak<Cell<usize>>>,
    is_dead: impl Fn(usize) -> bool,
    len: usize,
) {
    cursors.retain(|w| match w.upgrade() {
        Some(c) => {
            let at = c.get().min(len);
            let gone = (0..at).filter(|&i| is_dead(i)).count();
            c.set(c.get() - gone);
            true
        }
        None => false,
    });
}

/// `Map` storage: insertion-ordered entries (deleted ones stay as `Hole` keys until `compact`) plus a lazily
/// built hash index.
#[derive(Default)]
pub(crate) struct MapData {
    pub(crate) entries: Vec<(Value, Value)>,
    index: Option<FxMap<HKey, usize>>,
    /// Positions of live iterators, kept in step with `delete`/`clear`.
    #[allow(clippy::box_collection)]
    // one word instead of three keeps `Kind` at 80 bytes (size_tests)
    cursors: Box<Vec<Weak<Cell<usize>>>>,
    /// Deleted slots in `entries`.
    dead: usize,
}

impl MapData {
    // JsStr only mutates a cached ASCII flag; its Hash/Eq go through the string contents.
    #[allow(clippy::mutable_key_type)]
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
                .filter(|(_, (k, _))| !dead(k))
                .map(|(i, (k, _))| (hkey(k), i))
                .collect()
        });
        index.get(&hkey(key)).copied()
    }
    /// Number of live entries.
    pub(crate) fn len(&self) -> usize {
        self.entries.len() - self.dead
    }
    /// The live `(key, value)` pairs in insertion order.
    pub(crate) fn pairs(&self) -> Vec<(Value, Value)> {
        self.entries
            .iter()
            .filter(|(k, _)| !dead(k))
            .cloned()
            .collect()
    }
    pub(crate) fn get(&mut self, key: &Value) -> Option<Value> {
        self.find(key).map(|i| self.entries[i].1.clone())
    }
    pub(crate) fn has(&mut self, key: &Value) -> bool {
        self.find(key).is_some()
    }
    pub(crate) fn set(&mut self, key: Value, value: Value) {
        let key = live_key(key);
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
        if let Some(index) = &mut self.index {
            index.remove(&hkey(key));
        }
        self.entries[i] = (Value::Hole, Value::Undefined);
        self.dead += 1;
        if self.dead >= HASH_MIN && 2 * self.dead > self.entries.len() {
            self.compact();
        }
        true
    }
    /// Drops the deleted slots; the index is rebuilt on the next lookup.
    fn compact(&mut self) {
        let entries = &self.entries;
        remap_cursors(&mut self.cursors, |i| dead(&entries[i].0), entries.len());
        self.entries.retain(|(k, _)| !dead(k));
        self.dead = 0;
        self.index = None;
    }
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.dead = 0;
        self.index = None;
        reset_cursors(&mut self.cursors);
    }
}

/// `Set` storage, like [`MapData`].
#[derive(Default)]
pub(crate) struct SetData {
    pub(crate) items: Vec<Value>,
    index: Option<FxMap<HKey, usize>>,
    /// Positions of live iterators, kept in step with `delete`/`clear`.
    #[allow(clippy::box_collection)]
    // one word instead of three keeps `Kind` at 80 bytes (size_tests)
    cursors: Box<Vec<Weak<Cell<usize>>>>,
    /// Deleted slots in `items`.
    dead: usize,
}

impl SetData {
    // JsStr only mutates a cached ASCII flag; its Hash/Eq go through the string contents.
    #[allow(clippy::mutable_key_type)]
    fn find(&mut self, key: &Value) -> Option<usize> {
        if self.items.len() < HASH_MIN {
            return self.items.iter().position(|k| same_value_zero(k, key));
        }
        let items = &self.items;
        let index = self.index.get_or_insert_with(|| {
            items
                .iter()
                .enumerate()
                .filter(|(_, k)| !dead(k))
                .map(|(i, k)| (hkey(k), i))
                .collect()
        });
        index.get(&hkey(key)).copied()
    }
    /// Number of live items.
    pub(crate) fn len(&self) -> usize {
        self.items.len() - self.dead
    }
    /// The live items in insertion order.
    pub(crate) fn values(&self) -> Vec<Value> {
        self.items.iter().filter(|k| !dead(k)).cloned().collect()
    }
    pub(crate) fn has(&mut self, key: &Value) -> bool {
        self.find(key).is_some()
    }
    pub(crate) fn add(&mut self, key: Value) {
        let key = live_key(key);
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
        if let Some(index) = &mut self.index {
            index.remove(&hkey(key));
        }
        self.items[i] = Value::Hole;
        self.dead += 1;
        if self.dead >= HASH_MIN && 2 * self.dead > self.items.len() {
            self.compact();
        }
        true
    }
    /// Drops the deleted slots; the index is rebuilt on the next lookup.
    fn compact(&mut self) {
        let items = &self.items;
        remap_cursors(&mut self.cursors, |i| dead(&items[i]), items.len());
        self.items.retain(|k| !dead(k));
        self.dead = 0;
        self.index = None;
    }
    pub(crate) fn clear(&mut self) {
        self.items.clear();
        self.dead = 0;
        self.index = None;
        reset_cursors(&mut self.cursors);
    }
}

/// State of a live `Map`/`Set` iterator from `keys()`, `values()` or `entries()`.
pub(crate) struct CollIterData {
    /// The collection being walked: one `Rc` clone, read by the collector's edge enumeration (gc.rs).
    pub(crate) src: Obj,
    pub(crate) is_map: bool,
    /// 0 keys, 1 values, 2 entries.
    mode: u8,
    /// Number of entries already visited; shared with the collection so it can be fixed up.
    cursor: Rc<Cell<usize>>,
    done: Cell<bool>,
}

// ponytail: Vec<Weak> scanned on every delete/clear, O(live iterators); pruned on register.
fn register_cursor(cursors: &mut Vec<Weak<Cell<usize>>>, cursor: &Rc<Cell<usize>>) {
    cursors.retain(|w| w.strong_count() > 0);
    cursors.push(Rc::downgrade(cursor));
}

/// `clear()`: live iterators continue from the start of whatever is added later.
fn reset_cursors(cursors: &mut Vec<Weak<Cell<usize>>>) {
    cursors.retain(|w| match w.upgrade() {
        Some(c) => {
            c.set(0);
            true
        }
        None => false,
    });
}

/// A live cursor registered with the collection behind `src` (a Map or Set).
fn live_cursor(src: &Obj) -> Rc<Cell<usize>> {
    let cursor = Rc::new(Cell::new(0usize));
    match &mut *src.kind.borrow_mut() {
        Kind::Map(m) => register_cursor(&mut m.cursors, &cursor),
        Kind::Set(s) => register_cursor(&mut s.cursors, &cursor),
        _ => {}
    }
    cursor
}

/// The first live `(key, value)` at or after slot `i` of a Map or Set (for a Set both are the item), and the
/// slot after it (where a cursor continues).
fn entry_at(src: &Obj, i: usize) -> Option<(usize, Value, Value)> {
    match &*src.kind.borrow() {
        Kind::Map(m) => {
            let at = i + m.entries.get(i..)?.iter().position(|(k, _)| !dead(k))?;
            let (k, v) = m.entries[at].clone();
            Some((at + 1, k, v))
        }
        Kind::Set(s) => {
            let at = i + s.items.get(i..)?.iter().position(|k| !dead(k))?;
            let v = s.items[at].clone();
            Some((at + 1, v.clone(), v))
        }
        _ => None,
    }
}

/// `next()` of a `Map Iterator` (`is_map`) or `Set Iterator`.
fn coll_iter_next(it: &mut Interp, this: &Value, is_map: bool) -> R<Value> {
    let Value::Object(o) = this else {
        return it.throw_error(
            "TypeError",
            "Method Map Iterator.prototype.next called on incompatible receiver",
        );
    };
    let (src, mode, cursor, finished) = match &*o.kind.borrow() {
        Kind::CollIter(c) if c.is_map == is_map => {
            (c.src.clone(), c.mode, c.cursor.clone(), c.done.get())
        }
        _ => {
            let name = if is_map { "Map" } else { "Set" };
            return it.throw_error(
                "TypeError",
                &format!("Method {name} Iterator.prototype.next called on incompatible receiver"),
            );
        }
    };
    let result = it.plain();
    let entry = if finished {
        None
    } else {
        entry_at(&src, cursor.get())
    };
    match entry {
        Some((next, k, v)) => {
            cursor.set(next);
            let value = match mode {
                0 => k,
                1 => v,
                _ => it.array(vec![k, v]),
            };
            put(&result, "value", value);
            put(&result, "done", Value::Bool(false));
        }
        None => {
            if let Kind::CollIter(c) = &*o.kind.borrow() {
                c.done.set(true);
            }
            put(&result, "value", Value::Undefined);
            put(&result, "done", Value::Bool(true));
        }
    }
    Ok(Value::Object(result))
}

pub(crate) fn same_value_zero(a: &Value, b: &Value) -> bool {
    strict_equals(a, b)
        || matches!((a, b), (Value::Number(x), Value::Number(y)) if x.is_nan() && y.is_nan())
}

impl Interp {
    /// A live `Map`/`Set` iterator over `this` (`mode`: 0 keys, 1 values, 2 entries), or a
    /// TypeError if `this` is not the right kind of collection.
    pub(crate) fn coll_iterator(
        &mut self,
        this: &Value,
        is_map: bool,
        mode: u8,
        proto: &Obj,
    ) -> R<Value> {
        let Value::Object(src) = this else {
            return self.throw_error(
                "TypeError",
                "Method Map.prototype called on incompatible receiver",
            );
        };
        let cursor = Rc::new(Cell::new(0usize));
        let ok = match &mut *src.kind.borrow_mut() {
            Kind::Map(m) if is_map => {
                register_cursor(&mut m.cursors, &cursor);
                true
            }
            Kind::Set(s) if !is_map => {
                register_cursor(&mut s.cursors, &cursor);
                true
            }
            _ => false,
        };
        if !ok {
            let name = if is_map { "Map" } else { "Set" };
            return self.throw_error(
                "TypeError",
                &format!("Method {name}.prototype called on incompatible receiver"),
            );
        }
        let data = CollIterData {
            src: src.clone(),
            is_map,
            mode,
            cursor,
            done: Cell::new(false),
        };
        Ok(Value::Object(new_obj(
            Some(proto.clone()),
            Kind::CollIter(data),
        )))
    }

    /// Elements of an iterable that the evaluator can enumerate without iterator objects.
    pub(crate) fn iter_items(&mut self, v: &Value) -> Option<Vec<Value>> {
        match v {
            Value::Str(s) => Some(
                crate::builtins::str_code_points(s)
                    .into_iter()
                    .map(|c| Value::Str(c.into()))
                    .collect(),
            ),
            // a sparse array takes the protocol path (live Get per index), which sees its tail
            Value::Object(o) if crate::array_snapshot_unsafe(o) => None,
            Value::Object(o) => {
                let pairs = match &*o.kind.borrow() {
                    Kind::Array(items) => {
                        return Some(items.iter().map(|v| unhole(Some(v))).collect())
                    }
                    Kind::Set(set) => return Some(set.values()),
                    // a detached or out-of-bounds view: the protocol path throws V8's TypeError. A resizable buffer's
                    // view may change length while it is iterated, so it also takes the lazy protocol path.
                    Kind::Typed(t) if t.gone() || t.rz => return None,
                    Kind::Typed(t) => return Some(t.to_values()),
                    Kind::Map(map) => map.pairs(),
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

    /// Whether `Array.prototype[Symbol.iterator]` and `%ArrayIteratorPrototype%.next` are the originals, so an
    /// array may be read directly instead of through its iterator (`__thaw_array_iter_intact`, extras.rs).
    pub(crate) fn array_iter_intact(&mut self) -> bool {
        let global = self.global.clone();
        match self.lookup(&global, "__thaw_array_iter_intact") {
            Some(f) if f.is_callable() => self
                .call(&f, &Value::Undefined, &[])
                .map(|v| v.truthy())
                .unwrap_or(false),
            _ => false,
        }
    }

    /// Steps `v`'s iterator, calling `f` on each value; when `f` throws, the iterator is closed (`return()` once,
    /// its own error dropped) and that error propagates. `Ok(false)`: `v` has no iterator, nothing was called.
    /// A plain array is read directly only while its iteration protocol is unmodified.
    pub(crate) fn for_each_iterated(
        &mut self,
        v: &Value,
        f: &mut dyn FnMut(&mut Interp, Value) -> R<()>,
    ) -> R<bool> {
        let is_array = matches!(v, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_)));
        if !is_array || self.array_iter_intact() {
            if let Some(items) = self.iter_items(v) {
                for x in items {
                    f(self, x)?;
                }
                return Ok(true);
            }
        }
        let Some((iter, next)) = self.get_iterator(v)? else {
            return Ok(false);
        };
        loop {
            let r = self.call(&next, &iter, &[])?;
            if !matches!(r, Value::Object(_)) {
                return self.throw_error("TypeError", "Iterator result is not an object");
            }
            if self.get(&r, "done")?.truthy() {
                return Ok(true);
            }
            let value = self.get(&r, "value")?;
            if let Err(e) = f(self, value) {
                self.close_iterator(&iter);
                return Err(e);
            }
        }
    }

    /// `new Map(iterable)` / `new Set(iterable)`: feeds each value through `adder` called on `target`.
    /// `entries`: the values must be entry objects whose `0` and `1` are the adder's arguments.
    fn fill_collection(
        &mut self,
        target: &Value,
        src: &Value,
        adder_name: &str,
        entries: bool,
    ) -> R<()> {
        if matches!(src, Value::Undefined | Value::Null) {
            return Ok(());
        }
        let adder = self.get(target, adder_name)?;
        if !adder.is_callable() {
            let shown = self.inspect(&adder, false);
            return self.throw_error(
                "TypeError",
                &format!(
                    "'{shown}' returned for property '{adder_name}' of object is not a function"
                ),
            );
        }
        let found = self.for_each_iterated(src, &mut |it, item| {
            if !entries {
                it.call(&adder, target, &[item])?;
                return Ok(());
            }
            if !matches!(item, Value::Object(_)) || crate::is_symbol_object(&item) {
                let shown = it.inspect(&item, false);
                return it.throw_error(
                    "TypeError",
                    &format!("Iterator value {shown} is not an entry object"),
                );
            }
            let (k, v) = (it.get(&item, "0")?, it.get(&item, "1")?);
            it.call(&adder, target, &[k, v])?;
            Ok(())
        })?;
        if !found {
            let shown = self.inspect(src, false);
            return self.throw_error("TypeError", &format!("{shown} is not iterable"));
        }
        Ok(())
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
        self.to_str(v).into_rc()
    }

    /// The `name` a function takes from property key `key`: `[desc]` for an internal `@@sym:<id>`
    /// key (`[]` for an empty description, `""` for none), the key itself otherwise.
    pub(crate) fn fn_name_of_key(&self, key: &str) -> Rc<str> {
        let Some(id) = key
            .strip_prefix("@@sym:")
            .and_then(|i| i.parse::<u64>().ok())
        else {
            return key.into();
        };
        let desc = symbol_table::by_id(id).and_then(|o| match &*o.kind.borrow() {
            Kind::Symbol { desc, .. } => desc.clone(),
            _ => None,
        });
        desc.map_or("".into(), |d| format!("[{d}]").into())
    }

    /// A new symbol with `desc`, made by the intrinsic `Symbol` constructor (not the global `Symbol`).
    pub(crate) fn new_symbol(&mut self, desc: Option<Rc<str>>) -> R<Value> {
        let ctor = self
            .symbol_ctor
            .clone()
            .expect("Symbol is installed at realm setup");
        let arg = desc.map_or(Value::Undefined, |d| Value::Str(d.into()));
        self.call(&Value::Object(ctor), &Value::Undefined, &[arg])
    }

    /// `Symbol.for(key)`: the registered symbol for `key`, created and registered on first use.
    pub(crate) fn symbol_for(&mut self, key: Rc<str>) -> R<Value> {
        let registry = self
            .symbol_registry
            .clone()
            .unwrap_or_else(symbol_table::registry);
        if let Some((Slot::Data(v), _)) = find_own(&registry, &key) {
            return Ok(v);
        }
        let sym = self.new_symbol(Some(key.clone()))?;
        put_hidden(&registry, &key, sym.clone());
        Ok(sym)
    }

    /// `Symbol.keyFor(sym)`: the key `sym` is registered under by `Symbol.for`, if any.
    pub(crate) fn symbol_key_for(&self, sym: &Obj) -> Option<Rc<str>> {
        let registry = self
            .symbol_registry
            .clone()
            .unwrap_or_else(symbol_table::registry);
        symbol_table::key_in_registry(&registry, sym)
    }

    /// Whether `sym` is one of the intrinsic well-known symbols (`Symbol.iterator`, ...).
    pub(crate) fn is_well_known(&self, sym: &Obj) -> bool {
        symbol_table::is_well_known(sym)
    }

    pub(crate) fn install_collections(&mut self) {
        let g = self.global.clone();
        // ----- Symbol -----
        let sym_proto = self.plain();
        let symbol = self.native({
            let sp = sym_proto.clone();
            move |it, _, args| {
                let desc = match arg(args, 0) {
                    Value::Undefined => None,
                    v => Some(it.to_str(&v).into_rc()),
                };
                let (id, o) = symbol_table::new_symbol(sp.clone(), desc);
                it.symbols_made.push(id);
                Ok(Value::Object(o))
            }
        });
        if let Value::Object(s) = &symbol {
            put_hidden(s, "prototype", Value::Object(sym_proto.clone()));
            self.symbol_ctor = Some(s.clone());
            self.set_native(s, "keyFor", move |it, _, args| {
                let Value::Object(sym) = arg(args, 0) else {
                    let shown = it.inspect(&arg(args, 0), false);
                    return it.throw_error("TypeError", &format!("{shown} is not a symbol"));
                };
                Ok(it
                    .symbol_key_for(&sym)
                    .map_or(Value::Undefined, |k| Value::Str(k.into())))
            });
            self.set_native(s, "for", move |it, _, args| {
                let key = it.to_str(&arg(args, 0)).into_rc();
                it.symbol_for(key)
            });
            // the same symbol objects in every realm of the thread
            for (name, o) in symbol_table::WELL_KNOWN_NAMES
                .iter()
                .zip(symbol_table::well_known(&sym_proto))
            {
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
        // thisSymbolValue: a symbol, or the TypeError Node gives for any other receiver
        let this_symbol = |it: &mut Interp, this: &Value, what: &str| -> R<Value> {
            if let Value::Object(o) = this {
                if matches!(&*o.kind.borrow(), Kind::Symbol { .. }) {
                    return Ok(this.clone());
                }
            }
            it.throw_error(
                "TypeError",
                &format!("{what} requires that 'this' be a Symbol"),
            )
        };
        self.set_native(&sym_proto, "valueOf", move |it, this, _| {
            this_symbol(it, this, "Symbol.prototype.valueOf")
        });
        let description = self.native(move |it, this, _| {
            let sym = this_symbol(it, this, "Symbol.prototype.description")?;
            let Value::Object(o) = &sym else {
                return Ok(Value::Undefined);
            };
            let desc = match &*o.kind.borrow() {
                Kind::Symbol { desc, .. } => desc.clone(),
                _ => None,
            };
            Ok(desc.map_or(Value::Undefined, |d| Value::Str(d.into())))
        });
        define_accessor(&sym_proto, "description", Some(description), None, false);
        g.declare("Symbol", symbol, true);

        // ----- Map -----
        let mp = self.plain();
        let map = self.native({
            let mp = mp.clone();
            move |it, _, args| {
                let obj = Value::Object(new_obj(Some(mp.clone()), Kind::Map(MapData::default())));
                it.fill_collection(&obj, &arg(args, 0), "set", true)?;
                Ok(obj)
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
        let size =
            self.native(|it, this, _| Ok(Value::Number(with_map!(this, it, |e| e.len()) as f64)));
        define_accessor(&mp, "size", Some(size), None, false);
        self.set_native(&mp, "forEach", |it, this, args| {
            let Value::Object(src) = this else {
                return it.throw_error(
                    "TypeError",
                    "Method Map.prototype.forEach called on incompatible receiver",
                );
            };
            with_map!(this, it, |_e| ());
            let cursor = live_cursor(src);
            while let Some((next, k, v)) = entry_at(src, cursor.get()) {
                cursor.set(next);
                it.call(&arg(args, 0), &arg(args, 1), &[v, k, this.clone()])?;
            }
            Ok(Value::Undefined)
        });
        // Iterator prototypes; `next` is native, tags and the Iterator.prototype parent are set in extras.
        let map_iter_proto = self.plain();
        self.set_native(&map_iter_proto, "next", |it, this, _| {
            coll_iter_next(it, this, true)
        });
        for (name, mode) in [("keys", 0u8), ("values", 1), ("entries", 2)] {
            let proto = map_iter_proto.clone();
            self.set_native(&mp, name, move |it, this, _| {
                it.coll_iterator(this, true, mode, &proto)
            });
        }

        // ----- Set -----
        let sp = self.plain();
        let set = self.native({
            let sp = sp.clone();
            move |it, _, args| {
                let obj = Value::Object(new_obj(Some(sp.clone()), Kind::Set(SetData::default())));
                it.fill_collection(&obj, &arg(args, 0), "add", false)?;
                Ok(obj)
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
        let size =
            self.native(|it, this, _| Ok(Value::Number(with_set!(this, it, |e| e.len()) as f64)));
        define_accessor(&sp, "size", Some(size), None, false);
        self.set_native(&sp, "forEach", |it, this, args| {
            let Value::Object(src) = this else {
                return it.throw_error(
                    "TypeError",
                    "Method Set.prototype.forEach called on incompatible receiver",
                );
            };
            with_set!(this, it, |_e| ());
            let cursor = live_cursor(src);
            while let Some((next, v, _)) = entry_at(src, cursor.get()) {
                cursor.set(next);
                it.call(&arg(args, 0), &arg(args, 1), &[v.clone(), v, this.clone()])?;
            }
            Ok(Value::Undefined)
        });
        let set_iter_proto = self.plain();
        self.set_native(&set_iter_proto, "next", |it, this, _| {
            coll_iter_next(it, this, false)
        });
        // Node: Set.prototype.keys is Set.prototype.values (and @@iterator), one function object
        let values = self.native({
            let proto = set_iter_proto.clone();
            move |it, this, _| it.coll_iterator(this, false, 0, &proto)
        });
        put_hidden(&sp, "keys", values.clone());
        put_hidden(&sp, "values", values);
        let entries_proto = set_iter_proto.clone();
        self.set_native(&sp, "entries", move |it, this, _| {
            it.coll_iterator(this, false, 2, &entries_proto)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{Hash, Hasher};

    #[test]
    fn fx_hash_spreads_integer_keys_across_low_bits() {
        let mut low = std::collections::HashSet::new();
        for i in 0..1000 {
            let mut h = FxHasher::default();
            HKey::Number((i as f64).to_bits()).hash(&mut h);
            low.insert(h.finish() & 0xFFFF);
        }
        assert!(
            low.len() > 900,
            "only {} distinct low-16-bit hashes",
            low.len()
        );
    }

    #[test]
    fn map_with_20k_integer_keys_round_trips_in_insertion_order() {
        let src = "const m = new Map();
for (let i = 0; i < 20000; i++) m.set(i, i * 2);
m.delete(5); m.delete(19999);
let sum = 0;
for (let i = 0; i < 20000; i++) if (m.has(i)) sum += m.get(i);
let prev = -1, ordered = true, n = 0;
m.forEach((v, k) => { if (k <= prev) ordered = false; prev = k; n++; });
console.log(m.size, sum, m.has(5), m.get(19999), n, ordered);";
        let mut it = Interp::new();
        assert!(it.run(src).is_ok(), "script failed");
        assert_eq!(
            it.out.join("|"),
            "19998 399939992 false undefined 19998 true"
        );
    }
}

#[cfg(test)]
mod symbol_helper_tests {
    use super::*;

    fn ok<T>(r: R<T>) -> T {
        match r {
            Ok(v) => v,
            Err(_) => panic!("helper or script threw"),
        }
    }

    fn obj(v: Value) -> Obj {
        match v {
            Value::Object(o) => o,
            _ => panic!("expected a symbol object"),
        }
    }

    #[test]
    fn symbol_helpers_still_work_after_script_replaces_global_symbol() {
        let mut it = Interp::new();
        assert!(it
            .run("globalThis.Symbol = () => 17; Symbol.for = () => 0;")
            .is_ok());
        let sym = obj(ok(it.new_symbol(Some("d".into()))));
        assert!(matches!(&*sym.kind.borrow(), Kind::Symbol { desc: Some(d), .. } if &**d == "d"));
        let reg = obj(ok(it.symbol_for("k".into())));
        assert!(matches!(&*reg.kind.borrow(), Kind::Symbol { .. }));
    }

    #[test]
    fn symbol_for_returns_same_symbol_and_key_for_round_trips() {
        let mut it = Interp::new();
        let a = obj(ok(it.symbol_for("k".into())));
        let b = obj(ok(it.symbol_for("k".into())));
        assert!(Rc::ptr_eq(&a, &b));
        assert_eq!(it.symbol_key_for(&a).as_deref(), Some("k"));
        let fresh = obj(ok(it.new_symbol(None)));
        assert_eq!(it.symbol_key_for(&fresh), None);
    }

    #[test]
    fn is_well_known_is_true_for_iterator_and_false_for_fresh_symbol() {
        let mut it = Interp::new();
        let ctor = Value::Object(it.symbol_ctor.clone().expect("installed"));
        let iterator = obj(ok(it.get(&ctor, "iterator")));
        assert!(it.is_well_known(&iterator));
        let fresh = obj(ok(it.new_symbol(Some("iterator".into()))));
        assert!(!it.is_well_known(&fresh));
    }
}
