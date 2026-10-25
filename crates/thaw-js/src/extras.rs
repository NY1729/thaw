//! Small ES2015+ library functions written in JS on top of the native core (run after the preludes).

pub(crate) const EXTRAS: &str = r#"
(function () {
  function def(o, name, f) {
    Object.defineProperty(o, name, { value: f, writable: true, enumerable: false, configurable: true });
  }
  var AP = Array.prototype;
  if (!AP.flatMap) def(AP, 'flatMap', function (f, t) {
    var r = [];
    for (var i = 0; i < this.length; i++) {
      var v = f.call(t, this[i], i, this);
      if (Array.isArray(v)) { for (var j = 0; j < v.length; j++) r.push(v[j]); } else r.push(v);
    }
    return r;
  });

  def(AP, 'toString', function () { return this.join(','); });
  def(AP, 'entries', function () { var r = []; for (var i = 0; i < this.length; i++) r.push([i, this[i]]); return r; });
  def(AP, 'keys', function () { var r = []; for (var i = 0; i < this.length; i++) r.push(i); return r; });
  def(AP, 'values', function () { return this.slice(); });
  def(AP, 'at', function (i) { i = Math.trunc(i) || 0; if (i < 0) i += this.length; return this[i]; });
  def(AP, 'findLast', function (f, t) { for (var i = this.length - 1; i >= 0; i--) if (f.call(t, this[i], i, this)) return this[i]; });
  def(AP, 'findLastIndex', function (f, t) { for (var i = this.length - 1; i >= 0; i--) if (f.call(t, this[i], i, this)) return i; return -1; });
  def(AP, 'flat', function (d) {
    d = d === undefined ? 1 : d;
    var r = [];
    (function walk(a, depth) {
      for (var i = 0; i < a.length; i++) {
        if (Array.isArray(a[i]) && depth > 0) walk(a[i], depth - 1); else r.push(a[i]);
      }
    })(this, d);
    return r;
  });
  def(AP, 'toReversed', function () { return this.slice().reverse(); });
  def(AP, 'toSorted', function (c) { return this.slice().sort(c); });
  def(AP, 'toSpliced', function () { var c = this.slice(); c.splice.apply(c, arguments); return c; });
  def(AP, 'with', function (i, v) {
    if (i < 0) i += this.length;
    if (i < 0 || i >= this.length) throw new RangeError('Invalid index : ' + i);
    var c = this.slice(); c[i] = v; return c;
  });
  var SP = String.prototype;
  def(SP, 'at', function (i) { i = Math.trunc(i) || 0; if (i < 0) i += this.length; return i >= 0 && i < this.length ? this[i] : undefined; });
  def(SP, 'trimLeft', SP.trimStart);
  def(SP, 'trimRight', SP.trimEnd);
  def(SP, 'toLocaleLowerCase', function () { return this.toLowerCase(); });
  def(SP, 'toLocaleUpperCase', function () { return this.toUpperCase(); });
  def(SP, 'isWellFormed', function () { return true; });
  def(Object, 'hasOwn', function (o, k) { return Object.prototype.hasOwnProperty.call(o, k); });
  def(Object, 'groupBy', function (items, f) {
    var out = Object.create(null), i = 0;
    for (var x of items) { var k = f(x, i++); (out[k] = out[k] || []).push(x); }
    return out;
  });
  def(Object, 'isSealed', function (o) { return Object.isFrozen(o); });
  def(Math, 'sinh', function (x) { return (Math.exp(x) - Math.exp(-x)) / 2; });
  def(Math, 'cosh', function (x) { return (Math.exp(x) + Math.exp(-x)) / 2; });
  def(Math, 'tanh', function (x) { if (x === Infinity) return 1; if (x === -Infinity) return -1; var a = Math.exp(x), b = Math.exp(-x); return (a - b) / (a + b); });
  def(Math, 'asinh', function (x) { return x === -Infinity ? x : Math.log(x + Math.sqrt(x * x + 1)); });
  def(Math, 'acosh', function (x) { return Math.log(x + Math.sqrt(x * x - 1)); });
  def(Math, 'atanh', function (x) { return Math.log((1 + x) / (1 - x)) / 2; });
  def(Math, 'expm1', function (x) { return Math.exp(x) - 1; });
  def(Math, 'log1p', function (x) { return Math.log(1 + x); });
  def(Math, 'hypot', function () { var s = 0; for (var i = 0; i < arguments.length; i++) s += arguments[i] * arguments[i]; return Math.sqrt(s); });
  def(Math, 'clz32', function (x) { x = x >>> 0; if (x === 0) return 32; var n = 0; while (!(x & 0x80000000)) { x <<= 1; n++; } return n; });
  var consts = { LN10: Math.log(10), LN2: Math.log(2), LOG10E: 1 / Math.log(10), LOG2E: 1 / Math.log(2), SQRT2: Math.sqrt(2), SQRT1_2: Math.sqrt(0.5) };
  for (var cn in consts) Object.defineProperty(Math, cn, { value: consts[cn], writable: false, enumerable: false, configurable: false });
  if (Number.MIN_VALUE === undefined) Object.defineProperty(Number, 'MIN_VALUE', { value: 5e-324, writable: false, enumerable: false, configurable: false });
  var SetP = Set.prototype;
  def(SetP, 'union', function (o) { var r = new Set(this); for (var x of o.keys()) r.add(x); return r; });
  def(SetP, 'intersection', function (o) { var r = new Set(); for (var x of this) if (o.has(x)) r.add(x); return r; });
  def(SetP, 'difference', function (o) { var r = new Set(); for (var x of this) if (!o.has(x)) r.add(x); return r; });
  def(SetP, 'symmetricDifference', function (o) { var r = new Set(this); for (var x of o.keys()) { if (this.has(x)) r.delete(x); else r.add(x); } return r; });
  def(SetP, 'isSubsetOf', function (o) { for (var x of this) if (!o.has(x)) return false; return true; });
  def(SetP, 'isSupersetOf', function (o) { for (var x of o.keys()) if (!this.has(x)) return false; return true; });
  def(SetP, 'isDisjointFrom', function (o) { for (var x of this) if (o.has(x)) return false; return true; });
  def(Object, 'is', function (a, b) {
    if (a !== a) return b !== b;
    if (a === 0 && b === 0) return 1 / a === 1 / b;
    return a === b;
  });
  def(Object, 'getOwnPropertyDescriptors', function (o) {
    var out = {}, names = Object.getOwnPropertyNames(o);
    for (var i = 0; i < names.length; i++) out[names[i]] = Object.getOwnPropertyDescriptor(o, names[i]);
    return out;
  });
  def(String, 'raw', function (strings) {
    var raw = strings.raw, out = '';
    for (var i = 0; i < raw.length; i++) {
      out += raw[i];
      if (i + 1 < raw.length && i + 1 < arguments.length) out += String(arguments[i + 1]);
    }
    return out;
  });
  def(String, 'fromCodePoint', function () {
    var out = '';
    for (var i = 0; i < arguments.length; i++) {
      var c = arguments[i];
      if (c < 0 || c > 0x10ffff || c !== Math.floor(c)) throw new RangeError('Invalid code point ' + c);
      if (c > 0xffff) { c -= 0x10000; out += String.fromCharCode(0xd800 + (c >> 10), 0xdc00 + (c & 0x3ff)); }
      else out += String.fromCharCode(c);
    }
    return out;
  });
  def(String.prototype, 'codePointAt', function (i) {
    var c = this.charCodeAt(i);
    if (c >= 0xd800 && c < 0xdc00 && i + 1 < this.length) {
      var d = this.charCodeAt(i + 1);
      if (d >= 0xdc00 && d < 0xe000) return (c - 0xd800) * 0x400 + (d - 0xdc00) + 0x10000;
    }
    return c !== c ? undefined : c;
  });
  var B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  btoa = function (s) {
    s = String(s);
    var out = '';
    for (var i = 0; i < s.length; i += 3) {
      var a = s.charCodeAt(i), b = s.charCodeAt(i + 1), c = s.charCodeAt(i + 2);
      if (a > 255 || b > 255 || c > 255) throw new Error('Invalid character');
      var n = (a << 16) | ((b || 0) << 8) | (c || 0);
      out += B64[(n >> 18) & 63] + B64[(n >> 12) & 63] + (i + 1 < s.length ? B64[(n >> 6) & 63] : '=') + (i + 2 < s.length ? B64[n & 63] : '=');
    }
    return out;
  };
  atob = function (s) {
    s = String(s).replace(/[\t\n\f\r ]/g, '');
    if (s.length % 4 === 0) s = s.replace(/==?$/, '');
    if (s.length % 4 === 1 || /[^A-Za-z0-9+\/]/.test(s)) throw new Error('The string to be decoded is not correctly encoded.');
    var out = '', bits = 0, acc = 0;
    for (var i = 0; i < s.length; i++) {
      acc = (acc << 6) | B64.indexOf(s[i]); bits += 6;
      if (bits >= 8) { bits -= 8; out += String.fromCharCode((acc >> bits) & 255); }
    }
    return out;
  };
  var R = {};
  def(R, 'ownKeys', function (o) { return Object.getOwnPropertyNames(o).concat(Object.getOwnPropertySymbols(o)); });
  def(R, 'has', function (o, k) { return k in o; });
  def(R, 'get', function (o, k, r) { return o[k]; });
  def(R, 'set', function (o, k, v) { o[k] = v; return true; });
  def(R, 'getPrototypeOf', Object.getPrototypeOf);
  def(R, 'setPrototypeOf', function (o, p) { Object.setPrototypeOf(o, p); return true; });
  def(R, 'defineProperty', function (o, k, d) { try { Object.defineProperty(o, k, d); return true; } catch (e) { return false; } });
  def(R, 'deleteProperty', function (o, k) { return delete o[k]; });
  def(R, 'getOwnPropertyDescriptor', Object.getOwnPropertyDescriptor);
  def(R, 'apply', function (f, t, a) { return Function.prototype.apply.call(f, t, a); });
  def(R, 'construct', function (C, a) { return new (Function.prototype.bind.apply(C, [null].concat(a)))(); });
  Reflect = R;
  // WeakMap / WeakSet / WeakRef / FinalizationRegistry. ponytail: entries are held strongly (no GC is
  // observable from a short-lived process); only the key checks and the API surface matter.
  function isObject(v) { return (typeof v === 'object' && v !== null) || typeof v === 'function'; }
  function weakCollection(name, isMap) {
    var C = function (iterable) {
      if (!(this instanceof C)) throw new TypeError("Constructor " + name + " requires 'new'");
      def(this, '__store', new Map());
      if (iterable !== undefined && iterable !== null) {
        var items = Array.isArray(iterable) ? iterable : Array.from(iterable);
        for (var i = 0; i < items.length; i++) {
          if (isMap) {
            if (!isObject(items[i])) throw new TypeError('Iterator value ' + String(items[i]) + ' is not an entry object');
            this.set(items[i][0], items[i][1]);
          } else this.add(items[i]);
        }
      }
    };
    var bad = isMap ? 'Invalid value used as weak map key' : 'Invalid value used in weak set';
    var P = C.prototype;
    def(C, 'name', name);
    def(P, 'constructor', C);
    if (isMap) {
      def(P, 'set', function set(k, v) { if (!isObject(k)) throw new TypeError(bad); this.__store.set(k, v); return this; });
      def(P, 'get', function get(k) { return this.__store.get(k); });
    } else {
      def(P, 'add', function add(k) { if (!isObject(k)) throw new TypeError(bad); this.__store.set(k, true); return this; });
    }
    def(P, 'has', function has(k) { return this.__store.has(k); });
    def(P, 'delete', function (k) { return this.__store['delete'](k); });
    def(P, Symbol.toStringTag, name);
    return C;
  }
  WeakMap = weakCollection('WeakMap', true);
  WeakSet = weakCollection('WeakSet', false);
  WeakRef = function WeakRef(target) {
    if (!(this instanceof WeakRef)) throw new TypeError("Constructor WeakRef requires 'new'");
    if (!isObject(target)) throw new TypeError('WeakRef: target must be an object');
    def(this, '__target', target);
  };
  def(WeakRef.prototype, 'deref', function deref() { return this.__target; });
  def(WeakRef.prototype, 'constructor', WeakRef);
  def(WeakRef.prototype, Symbol.toStringTag, 'WeakRef');
  FinalizationRegistry = function FinalizationRegistry(cleanup) {
    if (!(this instanceof FinalizationRegistry)) throw new TypeError("Constructor FinalizationRegistry requires 'new'");
    if (typeof cleanup !== 'function') throw new TypeError('FinalizationRegistry: cleanup must be callable');
  };
  def(FinalizationRegistry.prototype, 'register', function register(target) {
    if (!isObject(target)) throw new TypeError('FinalizationRegistry.prototype.register: invalid target');
  });
  def(FinalizationRegistry.prototype, 'unregister', function unregister() { return false; });
  def(FinalizationRegistry.prototype, 'constructor', FinalizationRegistry);
  def(FinalizationRegistry.prototype, Symbol.toStringTag, 'FinalizationRegistry');
})();
"#;

impl crate::Interp {
    /// Give every native function a `name` (`String.name`, `Object.keys.name`, ...), like V8 does.
    pub(crate) fn name_builtins(&mut self) {
        use crate::{find_own, put_hidden, Kind, Slot, Value};
        let globals: Vec<(std::rc::Rc<str>, Value)> = self
            .global
            .vars
            .borrow()
            .iter()
            .map(|(k, b)| (k.clone(), b.value.clone()))
            .collect();
        let is_native = |v: &Value| matches!(v, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Native(_)));
        let mut work: Vec<(crate::Obj, std::rc::Rc<str>)> = Vec::new();
        for (name, v) in &globals {
            let Value::Object(o) = v else { continue };
            if is_native(v) {
                work.push((o.clone(), name.clone()));
            }
            let mut holders = vec![o.clone()];
            if let Some((Slot::Data(Value::Object(p)), _)) = find_own(o, "prototype") {
                holders.push(p);
            }
            for h in holders {
                for p in h.props.borrow().iter() {
                    if let Slot::Data(pv @ Value::Object(po)) = &p.slot {
                        if is_native(pv) {
                            work.push((po.clone(), p.key.clone()));
                        }
                    }
                }
            }
        }
        for (o, name) in work {
            if find_own(&o, "name").is_none() && !name.starts_with("@@") {
                put_hidden(&o, "name", Value::Str(name));
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Hint {
    Default,
    Number,
    String,
}

impl crate::Interp {
    /// ToPrimitive: `Symbol.toPrimitive`, then `valueOf` / `toString` in hint order.
    pub(crate) fn primitive_of(&mut self, v: &crate::Value, hint: Hint) -> crate::R<crate::Value> {
        use crate::{find_in_chain, Ctl, Slot, Value};
        let Value::Object(o) = v else {
            return Ok(v.clone());
        };
        let key = match self.rt.to_prim_key.clone() {
            Some(k) => k,
            None => {
                let sym = self
                    .lookup(&self.global.clone(), "Symbol")
                    .unwrap_or(Value::Undefined);
                let tp = self.get(&sym, "toPrimitive")?;
                let k = self.key_of(&tp);
                self.rt.to_prim_key = Some(k.clone());
                k
            }
        };
        if let Some(Slot::Data(f)) = find_in_chain(o, &key) {
            if f.is_callable() {
                let h = match hint {
                    Hint::Default => "default",
                    Hint::Number => "number",
                    Hint::String => "string",
                };
                let r = self.call(&f, v, &[Value::str(h)])?;
                if !matches!(r, Value::Object(_)) {
                    return Ok(r);
                }
                return self.throw_error("TypeError", "Cannot convert object to primitive value");
            }
        }
        // a Date prefers its string form unless a number is asked for explicitly
        let is_date = find_in_chain(o, "__time").is_some();
        let string_first = hint == Hint::String || (hint == Hint::Default && is_date);
        let order = if string_first {
            ["toString", "valueOf"]
        } else {
            ["valueOf", "toString"]
        };
        for name in order {
            let f = self.get(v, name)?;
            if f.is_callable() {
                match self.call(&f, v, &[]) {
                    Ok(r) if !matches!(r, Value::Object(_)) => return Ok(r),
                    Ok(_) => {}
                    Err(e @ Ctl::Throw(_)) => return Err(e),
                    Err(e) => return Err(e),
                }
            }
        }
        self.throw_error("TypeError", "Cannot convert object to primitive value")
    }
}
