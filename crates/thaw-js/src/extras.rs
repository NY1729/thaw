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
  def(R, 'construct', function (C, a) { return new C(...a); });
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

  // ----- iterators over arrays / strings, matchAll, normalize, misc numeric helpers -----
  var IterProto = {};
  def(IterProto, Symbol.iterator, function () { return this; });
  function makeIter(items, tag) {
    var i = 0, it = Object.create(IterProto);
    def(it, 'next', function next() { return i < items.length ? { value: items[i++], done: false } : { value: undefined, done: true }; });
    def(it, Symbol.toStringTag, tag);
    return it;
  }
  def(AP, 'entries', function () { var r = []; for (var i = 0; i < this.length; i++) r.push([i, this[i]]); return makeIter(r, 'Array Iterator'); });
  def(AP, 'keys', function () { var r = []; for (var i = 0; i < this.length; i++) r.push(i); return makeIter(r, 'Array Iterator'); });
  def(AP, 'values', function () { return makeIter(this.slice(), 'Array Iterator'); });
  def(AP, Symbol.iterator, AP.values);
  def(SP, Symbol.iterator, function () { return makeIter(Array.from(String(this)), 'String Iterator'); });
  Object.defineProperty(Object.prototype, '__proto__', {
    get: function () { return Object.getPrototypeOf(this); },
    set: function (p) { Object.setPrototypeOf(this, p); },
    enumerable: false, configurable: true
  });
  def(Map.prototype, Symbol.iterator, Map.prototype.entries);
  def(Set.prototype, Symbol.iterator, Set.prototype.values);
  def(Map.prototype, Symbol.toStringTag, 'Map');
  def(Set.prototype, Symbol.toStringTag, 'Set');
  def(AP, 'copyWithin', function (target, start, end) {
    var len = this.length;
    var to = target < 0 ? Math.max(len + target, 0) : Math.min(target, len);
    var from = start === undefined ? 0 : start < 0 ? Math.max(len + start, 0) : Math.min(start, len);
    var fin = end === undefined ? len : end < 0 ? Math.max(len + end, 0) : Math.min(end, len);
    var count = Math.min(fin - from, len - to), copy = this.slice(from, from + count);
    for (var i = 0; i < count; i++) this[to + i] = copy[i];
    return this;
  });
  def(SP, 'matchAll', function (re) {
    if (!(re instanceof RegExp)) re = new RegExp(re, 'g');
    else if (re.flags.indexOf('g') < 0) throw new TypeError('String.prototype.matchAll called with a non-global RegExp argument');
    var copy = new RegExp(re.source, re.flags); copy.lastIndex = re.lastIndex;
    var s = String(this), out = [], m;
    while ((m = copy.exec(s)) !== null) { out.push(m); if (m[0] === '') copy.lastIndex++; }
    return makeIter(out, 'RegExp String Iterator');
  });
  var NORM_C = '\u00c0\u00c1\u00c2\u00c3\u00c4\u00c5\u00c7\u00c8\u00c9\u00ca\u00cb\u00cc\u00cd\u00ce\u00cf\u00d1\u00d2\u00d3\u00d4\u00d5\u00d6\u00d9\u00da\u00db\u00dc\u00dd\u00e0\u00e1\u00e2\u00e3\u00e4\u00e5\u00e7\u00e8\u00e9\u00ea\u00eb\u00ec\u00ed\u00ee\u00ef\u00f1\u00f2\u00f3\u00f4\u00f5\u00f6\u00f9\u00fa\u00fb\u00fc\u00fd\u00ff\u0100\u0101\u0102\u0103\u0104\u0105\u0106\u0107\u0108\u0109\u010a\u010b\u010c\u010d\u010e\u010f\u0112\u0113\u0114\u0115\u0116\u0117\u0118\u0119\u011a\u011b\u011c\u011d\u011e\u011f\u0120\u0121\u0122\u0123\u0124\u0125\u0128\u0129\u012a\u012b\u012c\u012d\u012e\u012f\u0130\u0134\u0135\u0136\u0137\u0139\u013a\u013b\u013c\u013d\u013e\u0143\u0144\u0145\u0146\u0147\u0148\u014c\u014d\u014e\u014f\u0150\u0151\u0154\u0155\u0156\u0157\u0158\u0159\u015a\u015b\u015c\u015d\u015e\u015f\u0160\u0161\u0162\u0163\u0164\u0165\u0168\u0169\u016a\u016b\u016c\u016d\u016e\u016f\u0170\u0171\u0172\u0173\u0174\u0175\u0176\u0177\u0178\u0179\u017a\u017b\u017c\u017d\u017e\u01cd\u01ce\u01cf\u01d0\u01d1\u01d2\u01d3\u01d4\u1e00\u1e01\u1e02\u1e03\u1e04\u1e05\u1e06\u1e07\u1e0a\u1e0b\u1e0c\u1e0d\u1e0e\u1e0f\u1e10\u1e11\u1e12\u1e13\u1e18\u1e19\u1e1a\u1e1b\u1e1e\u1e1f\u1e20\u1e21\u1e22\u1e23\u1e24\u1e25\u1e26\u1e27\u1e28\u1e29\u1e2a\u1e2b\u1e2c\u1e2d\u1e30\u1e31\u1e32\u1e33\u1e34\u1e35\u1e36\u1e37\u1e3a\u1e3b\u1e3c\u1e3d\u1e3e\u1e3f\u1e40\u1e41\u1e42\u1e43\u1e44\u1e45\u1e46\u1e47\u1e48\u1e49\u1e4a\u1e4b\u1e54\u1e55\u1e56\u1e57\u1e58\u1e59\u1e5a\u1e5b\u1e5e\u1e5f\u1e60\u1e61\u1e62\u1e63\u1e6a\u1e6b\u1e6c\u1e6d\u1e6e\u1e6f\u1e70\u1e71\u1e72\u1e73\u1e74\u1e75\u1e76\u1e77\u1e7c\u1e7d\u1e7e\u1e7f\u1e80\u1e81\u1e82\u1e83\u1e84\u1e85\u1e86\u1e87\u1e88\u1e89\u1e8a\u1e8b\u1e8c\u1e8d\u1e8e\u1e8f\u1e90\u1e91\u1e92\u1e93\u1e94\u1e95\u1e96\u1e97\u1e98\u1e99\u1e9b\u1ea0\u1ea1\u1ea2\u1ea3\u1eb8\u1eb9\u1eba\u1ebb\u1ebc\u1ebd\u1ec8\u1ec9\u1eca\u1ecb\u1ecc\u1ecd\u1ece\u1ecf\u1ee4\u1ee5\u1ee6\u1ee7\u1ef2\u1ef3\u1ef4\u1ef5\u1ef6\u1ef7\u1ef8\u1ef9', NORM_D = 'A\u0300A\u0301A\u0302A\u0303A\u0308A\u030aC\u0327E\u0300E\u0301E\u0302E\u0308I\u0300I\u0301I\u0302I\u0308N\u0303O\u0300O\u0301O\u0302O\u0303O\u0308U\u0300U\u0301U\u0302U\u0308Y\u0301a\u0300a\u0301a\u0302a\u0303a\u0308a\u030ac\u0327e\u0300e\u0301e\u0302e\u0308i\u0300i\u0301i\u0302i\u0308n\u0303o\u0300o\u0301o\u0302o\u0303o\u0308u\u0300u\u0301u\u0302u\u0308y\u0301y\u0308A\u0304a\u0304A\u0306a\u0306A\u0328a\u0328C\u0301c\u0301C\u0302c\u0302C\u0307c\u0307C\u030cc\u030cD\u030cd\u030cE\u0304e\u0304E\u0306e\u0306E\u0307e\u0307E\u0328e\u0328E\u030ce\u030cG\u0302g\u0302G\u0306g\u0306G\u0307g\u0307G\u0327g\u0327H\u0302h\u0302I\u0303i\u0303I\u0304i\u0304I\u0306i\u0306I\u0328i\u0328I\u0307J\u0302j\u0302K\u0327k\u0327L\u0301l\u0301L\u0327l\u0327L\u030cl\u030cN\u0301n\u0301N\u0327n\u0327N\u030cn\u030cO\u0304o\u0304O\u0306o\u0306O\u030bo\u030bR\u0301r\u0301R\u0327r\u0327R\u030cr\u030cS\u0301s\u0301S\u0302s\u0302S\u0327s\u0327S\u030cs\u030cT\u0327t\u0327T\u030ct\u030cU\u0303u\u0303U\u0304u\u0304U\u0306u\u0306U\u030au\u030aU\u030bu\u030bU\u0328u\u0328W\u0302w\u0302Y\u0302y\u0302Y\u0308Z\u0301z\u0301Z\u0307z\u0307Z\u030cz\u030cA\u030ca\u030cI\u030ci\u030cO\u030co\u030cU\u030cu\u030cA\u0325a\u0325B\u0307b\u0307B\u0323b\u0323B\u0331b\u0331D\u0307d\u0307D\u0323d\u0323D\u0331d\u0331D\u0327d\u0327D\u032dd\u032dE\u032de\u032dE\u0330e\u0330F\u0307f\u0307G\u0304g\u0304H\u0307h\u0307H\u0323h\u0323H\u0308h\u0308H\u0327h\u0327H\u032eh\u032eI\u0330i\u0330K\u0301k\u0301K\u0323k\u0323K\u0331k\u0331L\u0323l\u0323L\u0331l\u0331L\u032dl\u032dM\u0301m\u0301M\u0307m\u0307M\u0323m\u0323N\u0307n\u0307N\u0323n\u0323N\u0331n\u0331N\u032dn\u032dP\u0301p\u0301P\u0307p\u0307R\u0307r\u0307R\u0323r\u0323R\u0331r\u0331S\u0307s\u0307S\u0323s\u0323T\u0307t\u0307T\u0323t\u0323T\u0331t\u0331T\u032dt\u032dU\u0324u\u0324U\u0330u\u0330U\u032du\u032dV\u0303v\u0303V\u0323v\u0323W\u0300w\u0300W\u0301w\u0301W\u0308w\u0308W\u0307w\u0307W\u0323w\u0323X\u0307x\u0307X\u0308x\u0308Y\u0307y\u0307Z\u0302z\u0302Z\u0323z\u0323Z\u0331z\u0331h\u0331t\u0308w\u030ay\u030a\u017f\u0307A\u0323a\u0323A\u0309a\u0309E\u0323e\u0323E\u0309e\u0309E\u0303e\u0303I\u0309i\u0309I\u0323i\u0323O\u0323o\u0323O\u0309o\u0309U\u0323u\u0323U\u0309u\u0309Y\u0300y\u0300Y\u0323y\u0323Y\u0309y\u0309Y\u0303y\u0303';
  var NORM_DEC = null, NORM_COMP = null;
  def(SP, 'normalize', function (form) {
    form = form === undefined ? 'NFC' : String(form);
    if (form !== 'NFC' && form !== 'NFD' && form !== 'NFKC' && form !== 'NFKD') throw new RangeError('The normalization form should be one of NFC, NFD, NFKC, NFKD.');
    if (!NORM_DEC) {
      NORM_DEC = {}; NORM_COMP = {};
      for (var i = 0; i < NORM_C.length; i++) { NORM_DEC[NORM_C[i]] = NORM_D[2 * i] + NORM_D[2 * i + 1]; NORM_COMP[NORM_D[2 * i] + NORM_D[2 * i + 1]] = NORM_C[i]; }
    }
    var s = String(this), out = '', i2;
    // decompose everything first; NFC then recomposes pairs
    for (i2 = 0; i2 < s.length; i2++) out += NORM_DEC[s[i2]] || s[i2];
    if (form === 'NFD' || form === 'NFKD') return out;
    var res = '';
    for (i2 = 0; i2 < out.length; i2++) {
      var two = out[i2] + (out[i2 + 1] || '');
      if (NORM_COMP[two]) { res += NORM_COMP[two]; i2++; } else res += out[i2];
    }
    return res;
  });
  var NP = Number.prototype;
  def(NP, 'toPrecision', function (p) {
    var x = Number(this);
    if (p === undefined || !isFinite(x)) return String(x);
    p = Math.trunc(p);
    if (p < 1 || p > 100) throw new RangeError('toPrecision() argument must be between 1 and 100');
    if (x === 0) return p > 1 ? '0.' + '0'.repeat(p - 1) : '0';
    var m = /^(-?)(\d)(?:\.(\d+))?e([+-]\d+)$/.exec(x.toExponential(p - 1));
    var e = +m[4], digits = m[2] + (m[3] || '');
    if (e < -6 || e >= p) return m[1] + m[2] + (m[3] ? '.' + m[3] : '') + 'e' + (e < 0 ? '-' : '+') + Math.abs(e);
    if (e >= 0) return m[1] + digits.slice(0, e + 1) + (p > e + 1 ? '.' + digits.slice(e + 1) : '');
    return m[1] + '0.' + '0'.repeat(-e - 1) + digits;
  });
  def(Math, 'fround', function (x) { return new Float32Array([x])[0]; });
  def(Map, 'groupBy', function (items, f) {
    var out = new Map(), i = 0;
    for (var x of items) { var k = f(x, i++); if (out.has(k)) out.get(k).push(x); else out.set(k, [x]); }
    return out;
  });
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

/// Globals that need the typed arrays, Buffer and URL installed first (run after them).
pub(crate) const EXTRAS_LATE: &str = r#"
(function () {
  function def(o, name, f) {
    Object.defineProperty(o, name, { value: f, writable: true, enumerable: false, configurable: true });
  }
  // ----- DataView -----
  function DataView(buffer, byteOffset, byteLength) {
    if (!(this instanceof DataView)) throw new TypeError("Constructor DataView requires 'new'");
    if (!(buffer instanceof ArrayBuffer)) throw new TypeError('First argument to DataView constructor must be an ArrayBuffer');
    var off = byteOffset === undefined ? 0 : Math.trunc(byteOffset), total = buffer.byteLength;
    if (off < 0 || off > total) throw new RangeError('Start offset ' + off + ' is outside the bounds of the buffer');
    var len = byteLength === undefined ? total - off : Math.trunc(byteLength);
    if (len < 0 || off + len > total) throw new RangeError('Invalid DataView length ' + len);
    def(this, 'buffer', buffer);
    def(this, 'byteOffset', off);
    def(this, 'byteLength', len);
    def(this, '__bytes', new Uint8Array(buffer, off, len));
  }
  var scratch = new Uint8Array(8);
  var kinds = {
    Int8: [1, Int8Array], Uint8: [1, Uint8Array], Int16: [2, Int16Array], Uint16: [2, Uint16Array],
    Int32: [4, Int32Array], Uint32: [4, Uint32Array], Float32: [4, Float32Array], Float64: [8, Float64Array]
  };
  function check(dv, index, size) {
    index = Math.trunc(index) || 0;
    if (index < 0 || index + size > dv.byteLength) throw new RangeError('Offset is outside the bounds of the DataView');
    return index;
  }
  Object.keys(kinds).forEach(function (name) {
    var size = kinds[name][0], View = kinds[name][1], view = new View(scratch.buffer, 0, 1);
    def(DataView.prototype, 'get' + name, function (index, little) {
      index = check(this, index, size);
      for (var i = 0; i < size; i++) scratch[little ? i : size - 1 - i] = this.__bytes[index + i];
      return view[0];
    });
    def(DataView.prototype, 'set' + name, function (index, value, little) {
      index = check(this, index, size);
      view[0] = value;
      for (var i = 0; i < size; i++) this.__bytes[index + i] = scratch[little ? i : size - 1 - i];
    });
  });
  [['BigInt64', true], ['BigUint64', false]].forEach(function (k) {
    def(DataView.prototype, 'get' + k[0], function (index, little) {
      index = check(this, index, 8);
      var v = 0n;
      for (var i = 0; i < 8; i++) v = (v << 8n) | BigInt(this.__bytes[index + (little ? 7 - i : i)]);
      return k[1] ? BigInt.asIntN(64, v) : v;
    });
    def(DataView.prototype, 'set' + k[0], function (index, value, little) {
      index = check(this, index, 8);
      var v = BigInt.asUintN(64, BigInt(value));
      for (var i = 7; i >= 0; i--) { this.__bytes[index + (little ? i : 7 - i)] = Number(v & 255n); v >>= 8n; }
    });
  });
  def(DataView.prototype, Symbol.toStringTag, 'DataView');
  def(DataView.prototype, 'constructor', DataView);
  globalThis.DataView = DataView;

  // ----- structuredClone (plain data, Map, Set, Date, RegExp, typed arrays, Error; cycles preserved) -----
  globalThis.structuredClone = function structuredClone(value) {
    var seen = new Map();
    function clone(v) {
      if (typeof v === 'function' || typeof v === 'symbol') {
        var msg = (typeof v === 'function' ? String(v) : String(v.toString())) + ' could not be cloned.';
        var e;
        if (typeof DOMException === 'function') e = new DOMException(msg, 'DataCloneError');
        else { e = new Error(msg); e.name = 'DataCloneError'; }
        throw e;
      }
      if (typeof v !== 'object' || v === null) return v;
      if (seen.has(v)) return seen.get(v);
      var out;
      if (Array.isArray(v)) { out = []; seen.set(v, out); for (var i = 0; i < v.length; i++) out[i] = clone(v[i]); return out; }
      if (v instanceof Date) { out = new Date(v.getTime()); seen.set(v, out); return out; }
      if (v instanceof RegExp) { out = new RegExp(v.source, v.flags); seen.set(v, out); return out; }
      if (v instanceof Map) { out = new Map(); seen.set(v, out); v.forEach(function (x, k) { out.set(clone(k), clone(x)); }); return out; }
      if (v instanceof Set) { out = new Set(); seen.set(v, out); v.forEach(function (x) { out.add(clone(x)); }); return out; }
      if (v instanceof ArrayBuffer) { out = v.slice(0); seen.set(v, out); return out; }
      if (ArrayBuffer.isView(v)) { out = new v.constructor(v); seen.set(v, out); return out; }
      if (v instanceof Error) {
        out = Object.create(Object.getPrototypeOf(v)); seen.set(v, out);
        ['message', 'stack', 'cause'].forEach(function (k) { if (Object.prototype.hasOwnProperty.call(v, k)) def(out, k, clone(v[k])); });
        return out;
      }
      out = {}; seen.set(v, out);
      Object.keys(v).forEach(function (k) { out[k] = clone(v[k]); });
      return out;
    }
    return clone(value);
  };

  // ----- util.types (the registry's node:util polyfill reads globalThis.__thaw_util_types) -----
  var tag = function (v) { return Object.prototype.toString.call(v); };
  var utilTypes = {
    isDate: function (v) { return v instanceof Date; },
    isRegExp: function (v) { return v instanceof RegExp; },
    isMap: function (v) { return v instanceof Map; },
    isSet: function (v) { return v instanceof Set; },
    isWeakMap: function (v) { return v instanceof WeakMap; },
    isWeakSet: function (v) { return v instanceof WeakSet; },
    isPromise: function (v) { return v instanceof Promise; },
    isArrayBuffer: function (v) { return v instanceof ArrayBuffer; },
    isAnyArrayBuffer: function (v) { return v instanceof ArrayBuffer; },
    isArrayBufferView: function (v) { return ArrayBuffer.isView(v); },
    isDataView: function (v) { return v instanceof DataView; },
    isTypedArray: function (v) { return ArrayBuffer.isView(v) && !(v instanceof DataView); },
    isUint8Array: function (v) { return v instanceof Uint8Array; },
    isUint8ClampedArray: function (v) { return v instanceof Uint8ClampedArray; },
    isUint16Array: function (v) { return v instanceof Uint16Array; },
    isUint32Array: function (v) { return v instanceof Uint32Array; },
    isInt8Array: function (v) { return v instanceof Int8Array; },
    isInt16Array: function (v) { return v instanceof Int16Array; },
    isInt32Array: function (v) { return v instanceof Int32Array; },
    isFloat32Array: function (v) { return v instanceof Float32Array; },
    isFloat64Array: function (v) { return v instanceof Float64Array; },
    isBigInt64Array: function () { return false; },
    isBigUint64Array: function () { return false; },
    isNativeError: function (v) { return v instanceof Error; },
    isArgumentsObject: function (v) { return tag(v) === '[object Arguments]'; },
    isNumberObject: function (v) { return tag(v) === '[object Number]' && typeof v === 'object'; },
    isStringObject: function (v) { return tag(v) === '[object String]' && typeof v === 'object'; },
    isBooleanObject: function (v) { return tag(v) === '[object Boolean]' && typeof v === 'object'; },
    isBigIntObject: function (v) { return tag(v) === '[object BigInt]' && typeof v === 'object'; },
    isSymbolObject: function (v) { return tag(v) === '[object Symbol]' && typeof v === 'object'; },
    isBoxedPrimitive: function () { return false; },
    isAsyncFunction: function (v) { return tag(v) === '[object AsyncFunction]'; },
    isGeneratorFunction: function (v) { return tag(v) === '[object GeneratorFunction]'; },
    isGeneratorObject: function (v) { return tag(v) === '[object Generator]'; },
    isExternal: function () { return false; },
    isProxy: function () { return false; },
    isModuleNamespaceObject: function () { return false; },
    isKeyObject: function () { return false; },
    isCryptoKey: function () { return false; }
  };
  utilTypes.default = utilTypes;
  utilTypes.__esModule = true;
  def(globalThis, '__thaw_util_types', utilTypes);

  var start = Date.now();
  if (typeof performance === 'undefined') {
    globalThis.performance = { now: function now() { return Date.now() - start; }, timeOrigin: start };
  }
})();
"#;
