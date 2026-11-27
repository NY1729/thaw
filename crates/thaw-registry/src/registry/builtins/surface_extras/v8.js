// node:v8 members that need no engine hooks: appended by builtins.rs `with_surface`.
(function () {
  var V = module.exports;
  function notBuilding() { var e = new Error('Not building snapshot'); e.code = 'ERR_NOT_BUILDING_SNAPSHOT'; return e; }
  V.isStringOneByteRepresentation = function isStringOneByteRepresentation(content) {
    if (typeof content !== 'string') { var e = new TypeError('The "content" argument must be of type string. Received ' + typeof content); e.code = 'ERR_INVALID_ARG_TYPE'; throw e; }
    for (var i = 0; i < content.length; i++) if (content.charCodeAt(i) > 0xff) return false;
    return true;
  };
  V.getCppHeapStatistics = function getCppHeapStatistics(detailLevel) {
    return { committed_size_bytes: 0, resident_size_bytes: 0, used_size_bytes: 0, space_statistics: [], type_names: [], detail_level: detailLevel === 'brief' ? 'brief' : 'detailed' };
  };
  V.setHeapSnapshotNearHeapLimit = function setHeapSnapshotNearHeapLimit(limit) {
    if (typeof limit !== 'number' || !Number.isInteger(limit) || limit < 1) { var e = new RangeError('The value of "limit" is out of range. It must be >= 1. Received ' + String(limit)); e.code = 'ERR_OUT_OF_RANGE'; throw e; }
  };
  // No NODE_V8_COVERAGE collection in thaw: like Node without it, these are no-ops.
  V.takeCoverage = function takeCoverage() {};
  V.stopCoverage = function stopCoverage() {};
  // Node outside a snapshot build: isBuildingSnapshot() is false and the callbacks throw ERR_NOT_BUILDING_SNAPSHOT.
  V.startupSnapshot = {
    addSerializeCallback: function addSerializeCallback() { throw notBuilding(); },
    addDeserializeCallback: function addDeserializeCallback() { throw notBuilding(); },
    setDeserializeMainFunction: function setDeserializeMainFunction() { throw notBuilding(); },
    isBuildingSnapshot: function isBuildingSnapshot() { return false; }
  };
  function GCProfiler() { if (!(this instanceof GCProfiler)) throw new TypeError("Class constructor GCProfiler cannot be invoked without 'new'"); Object.defineProperty(this, '_start', { value: undefined, writable: true, configurable: true }); }
  GCProfiler.prototype.start = function start() { if (this._start === undefined) this._start = Date.now(); };
  GCProfiler.prototype.stop = function stop() {
    if (this._start === undefined) return undefined;
    var data = { version: 1, startTime: this._start, statistics: [], endTime: Date.now() };
    this._start = undefined;
    return data;
  };
  V.GCProfiler = GCProfiler;

  // v8.Serializer / Deserializer / DefaultSerializer / DefaultDeserializer: V8's own wire format (version 15).
  var DEFAULT_VERSION = 15, st = new WeakMap();
  function argType(name, what, v) { var e = new TypeError('The "' + name + '" argument must be ' + what + '. Received ' + (v === null || v === undefined ? v : typeof v)); e.code = 'ERR_INVALID_ARG_TYPE'; return e; }
  var VIEWS = [['Int8Array', 'b'], ['Uint8Array', 'B'], ['Uint8ClampedArray', 'C'], ['Int16Array', 'w'], ['Uint16Array', 'W'], ['Int32Array', 'd'], ['Uint32Array', 'D'], ['Float32Array', 'f'], ['Float64Array', 'F'], ['DataView', '?'], ['Buffer', '?'], ['BigInt64Array', 'q'], ['BigUint64Array', 'Q']];
  function Serializer() {
    if (!(this instanceof Serializer)) throw new TypeError("Class constructor Serializer cannot be invoked without 'new'");
    st.set(this, { bytes: [], ids: new Map(), hostViews: false });
  }
  var SP = Serializer.prototype;
  function varint(s, n) { n = Math.floor(n); do { var b = n % 128; n = Math.floor(n / 128); s.bytes.push(n > 0 ? b | 128 : b); } while (n > 0); }
  function zigzag(s, n) { varint(s, n < 0 ? (-n) * 2 - 1 : n * 2); }
  function f64(s, d) { var b = new Uint8Array(new Float64Array([d]).buffer); for (var i = 0; i < 8; i++) s.bytes.push(b[i]); }
  function str(s, v, tag) {
    var one = true; for (var i = 0; i < v.length; i++) if (v.charCodeAt(i) > 0xff) { one = false; break; }
    if (one) { s.bytes.push(0x22); varint(s, v.length); for (var j = 0; j < v.length; j++) s.bytes.push(v.charCodeAt(j)); return; }
    var len = v.length * 2, size = 1; for (var t = len; t >= 128; t = Math.floor(t / 128)) size++;
    if ((s.bytes.length + 1 + size) & 1) s.bytes.push(0);
    s.bytes.push(0x63); varint(s, len); for (var k = 0; k < v.length; k++) { var c = v.charCodeAt(k); s.bytes.push(c & 255, c >> 8); }
  }
  function bigint(s, v) {
    var neg = v < 0n, m = neg ? -v : v, digits = []; while (m > 0n) { digits.push(Number(m & 0xffn)); m >>= 8n; }
    while (digits.length % 8) digits.push(0);
    s.bytes.push(0x5a); varint(s, digits.length * 2 + (neg ? 1 : 0)); for (var i = 0; i < digits.length; i++) s.bytes.push(digits[i]);
  }
  function cloneError(self, v) { var C = self._getDataCloneError || Error; return new C(String(typeof v === 'symbol' ? v.toString() : v) + ' could not be cloned.'); }
  function write(self, s, v) {
    switch (typeof v) {
      case 'undefined': s.bytes.push(0x5f); return;
      case 'boolean': s.bytes.push(v ? 0x54 : 0x46); return;
      case 'number': if (Number.isInteger(v) && !Object.is(v, -0) && v >= -1073741824 && v <= 1073741823) { s.bytes.push(0x49); zigzag(s, v); } else { s.bytes.push(0x4e); f64(s, v); } return;
      case 'bigint': bigint(s, v); return;
      case 'string': str(s, v); return;
      case 'function': case 'symbol': throw cloneError(self, v);
    }
    if (v === null) { s.bytes.push(0x30); return; }
    if (s.ids.has(v)) { s.bytes.push(0x5e); varint(s, s.ids.get(v)); return; }
    var isView = ArrayBuffer.isView(v);
    if (isView && s.hostViews) { s.ids.set(v, s.ids.size); s.bytes.push(0x5c); self._writeHostObject(v); return; }
    s.ids.set(v, s.ids.size);
    if (v instanceof ArrayBuffer) { s.bytes.push(0x42); varint(s, v.byteLength); var a = new Uint8Array(v); for (var i = 0; i < a.length; i++) s.bytes.push(a[i]); return; }
    if (isView) {
      s.ids.delete(v); s.ids.set(v.buffer, s.ids.size); s.bytes.push(0x42); varint(s, v.buffer.byteLength); var ba = new Uint8Array(v.buffer); for (var q = 0; q < ba.length; q++) s.bytes.push(ba[q]);
      s.ids.set(v, s.ids.size); var name = v instanceof DataView ? 'DataView' : v.constructor.name; var sub = '?'; for (var z = 0; z < VIEWS.length; z++) if (VIEWS[z][0] === name) sub = VIEWS[z][1];
      if (name === 'Buffer') sub = 'B';
      s.bytes.push(0x56, sub.charCodeAt(0)); varint(s, v.byteOffset); varint(s, v.byteLength); varint(s, 0); return;
    }
    if (v instanceof Date) { s.bytes.push(0x44); f64(s, v.getTime()); return; }
    if (v instanceof RegExp) { s.bytes.push(0x52); str(s, v.source); var f = 0, fl = v.flags, order = 'gimuysdv'; for (var x = 0; x < fl.length; x++) f |= 1 << order.indexOf(fl[x]); varint(s, f); return; }
    if (v instanceof Boolean) { s.bytes.push(v.valueOf() ? 0x79 : 0x78); return; }
    if (v instanceof Number) { s.bytes.push(0x6e); f64(s, v.valueOf()); return; }
    if (v instanceof String) { s.bytes.push(0x73); str(s, v.valueOf()); return; }
    if (v instanceof Map) { s.bytes.push(0x3b); var n = 0; v.forEach(function (val, key) { write(self, s, key); write(self, s, val); n += 2; }); s.bytes.push(0x3a); varint(s, n); return; }
    if (v instanceof Set) { s.bytes.push(0x27); var m = 0; v.forEach(function (val) { write(self, s, val); m++; }); s.bytes.push(0x2c); varint(s, m); return; }
    if (v instanceof Error) {
      s.bytes.push(0x72); var tags = { EvalError: 0x45, RangeError: 0x52, ReferenceError: 0x46, SyntaxError: 0x53, TypeError: 0x54, URIError: 0x55 }; if (tags[v.name]) s.bytes.push(tags[v.name]);
      if (Object.prototype.hasOwnProperty.call(v, 'message')) { s.bytes.push(0x6d); str(s, String(v.message)); }
      if (typeof v.stack === 'string') { s.bytes.push(0x73); str(s, v.stack); }
      s.bytes.push(0x2e); return;
    }
    if (Array.isArray(v)) {
      var holes = false; for (var h = 0; h < v.length; h++) if (!(h in v)) { holes = true; break; }
      var extra = Object.keys(v).filter(function (k) { return !/^(0|[1-9]\d*)$/.test(k) || +k >= v.length; });
      if (!holes) {
        s.bytes.push(0x41); varint(s, v.length); for (var e = 0; e < v.length; e++) write(self, s, v[e]);
        for (var p = 0; p < extra.length; p++) { write(self, s, extra[p]); write(self, s, v[extra[p]]); }
        s.bytes.push(0x24); varint(s, extra.length); varint(s, v.length); return;
      }
      s.bytes.push(0x61); varint(s, v.length); var keys = Object.keys(v);
      for (var kk = 0; kk < keys.length; kk++) { write(self, s, /^(0|[1-9]\d*)$/.test(keys[kk]) ? +keys[kk] : keys[kk]); write(self, s, v[keys[kk]]); }
      s.bytes.push(0x40); varint(s, keys.length); varint(s, v.length); return;
    }
    s.bytes.push(0x6f); var own = Object.keys(v);
    for (var o = 0; o < own.length; o++) { write(self, s, /^(0|[1-9]\d*)$/.test(own[o]) && +own[o] <= 1073741823 ? +own[o] : own[o]); write(self, s, v[own[o]]); }
    s.bytes.push(0x7b); varint(s, own.length);
  }
  SP.writeHeader = function writeHeader() { var s = st.get(this); s.bytes.push(0xff, DEFAULT_VERSION); };
  SP.writeValue = function writeValue(value) { write(this, st.get(this), value); return true; };
  SP.releaseBuffer = function releaseBuffer() { var s = st.get(this), b = Buffer.from(s.bytes); s.bytes = []; return b; };
  SP.transferArrayBuffer = function transferArrayBuffer(id, arrayBuffer) { };
  SP.writeUint32 = function writeUint32(n) { varint(st.get(this), n >>> 0); };
  SP.writeUint64 = function writeUint64(hi, lo) { var s = st.get(this), v = (BigInt(hi >>> 0) << 32n) | BigInt(lo >>> 0); do { var b = Number(v & 127n); v >>= 7n; s.bytes.push(v > 0n ? b | 128 : b); } while (v > 0n); };
  SP.writeDouble = function writeDouble(d) { f64(st.get(this), d); };
  SP.writeRawBytes = function writeRawBytes(source) {
    if (!ArrayBuffer.isView(source)) throw argType('source', 'an instance of Buffer, TypedArray, or DataView', source);
    var s = st.get(this), a = new Uint8Array(source.buffer, source.byteOffset, source.byteLength); for (var i = 0; i < a.length; i++) s.bytes.push(a[i]);
  };
  SP._setTreatArrayBufferViewsAsHostObjects = function _setTreatArrayBufferViewsAsHostObjects(flag) { st.get(this).hostViews = !!flag; };
  SP._getDataCloneError = Error;
  SP._writeHostObject = function _writeHostObject() { throw cloneError(this, 'object'); };

  function Deserializer(buffer) {
    if (!(this instanceof Deserializer)) throw new TypeError("Class constructor Deserializer cannot be invoked without 'new'");
    if (!ArrayBuffer.isView(buffer)) throw argType('buffer', 'a TypedArray or a DataView', buffer);
    var bytes = new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength);
    st.set(this, { bytes: bytes, pos: 0, version: 0, objects: [] });
    Object.defineProperty(this, 'buffer', { value: buffer, enumerable: false, configurable: true, writable: true });
  }
  var DP = Deserializer.prototype;
  function bad() { return new Error('Unable to deserialize cloned data.'); }
  function rbyte(s) { if (s.pos >= s.bytes.length) throw bad(); return s.bytes[s.pos++]; }
  function rvar(s) { var r = 0, mul = 1, b; do { b = rbyte(s); r += (b & 127) * mul; mul *= 128; } while (b & 128); return r; }
  function rf64(s) { if (s.pos + 8 > s.bytes.length) throw bad(); var a = new Uint8Array(8); for (var i = 0; i < 8; i++) a[i] = s.bytes[s.pos++]; return new Float64Array(a.buffer)[0]; }
  function rstr(s, tag) {
    var len = rvar(s), r = '';
    if (tag === 0x22) { for (var i = 0; i < len; i++) r += String.fromCharCode(rbyte(s)); }
    else if (tag === 0x63) { for (var j = 0; j < len; j += 2) { var lo = rbyte(s); r += String.fromCharCode(lo | (rbyte(s) << 8)); } }
    else { var b = []; for (var k = 0; k < len; k++) b.push(rbyte(s)); r = Buffer.from(b).toString('utf8'); }
    return r;
  }
  function rtag(s) { var t; do { t = rbyte(s); } while (t === 0); return t; }
  function read(self, s) {
    var t = rtag(s), v, id;
    switch (t) {
      case 0x5f: return undefined; case 0x30: return null; case 0x54: return true; case 0x46: return false;
      case 0x49: { var z = rvar(s); return z % 2 ? -(z + 1) / 2 : z / 2; }
      case 0x55: return rvar(s);
      case 0x4e: return rf64(s);
      case 0x5a: { var bf = rvar(s), len = Math.floor(bf / 2), r = 0n; for (var i = 0; i < len; i++) r |= BigInt(rbyte(s)) << BigInt(8 * i); return bf % 2 ? -r : r; }
      case 0x22: case 0x63: case 0x53: return rstr(s, t);
      case 0x5e: id = rvar(s); if (id >= s.objects.length) throw bad(); return s.objects[id];
      case 0x44: v = new Date(rf64(s)); s.objects.push(v); return v;
      case 0x79: v = new Boolean(true); s.objects.push(v); return v;
      case 0x78: v = new Boolean(false); s.objects.push(v); return v;
      case 0x6e: v = new Number(rf64(s)); s.objects.push(v); return v;
      case 0x73: v = new String(rstr(s, rtag(s))); s.objects.push(v); return v;
      case 0x52: { var src = rstr(s, rtag(s)), f = rvar(s), fl = '', order = 'gimuysdv'; for (var x = 0; x < order.length; x++) if (f & (1 << x)) fl += order[x]; v = new RegExp(src, fl); s.objects.push(v); return v; }
      case 0x42: { var n = rvar(s); if (s.pos + n > s.bytes.length) throw bad(); v = s.bytes.slice(s.pos, s.pos + n).buffer; s.pos += n; s.objects.push(v);
        if (s.bytes[s.pos] === 0x56) { s.pos++; return readView(s, v); } return v; }
      case 0x3b: { v = new Map(); s.objects.push(v); while (s.bytes[s.pos] !== 0x3a) { var key = read(self, s); v.set(key, read(self, s)); } s.pos++; rvar(s); return v; }
      case 0x27: { v = new Set(); s.objects.push(v); while (s.bytes[s.pos] !== 0x2c) v.add(read(self, s)); s.pos++; rvar(s); return v; }
      case 0x6f: { v = {}; s.objects.push(v); while (s.bytes[s.pos] !== 0x7b) { var k = read(self, s); v[k] = read(self, s); } s.pos++; rvar(s); return v; }
      case 0x41: { var l = rvar(s); v = new Array(l); s.objects.push(v); for (var a = 0; a < l; a++) v[a] = read(self, s); while (s.bytes[s.pos] !== 0x24) { var ek = read(self, s); v[ek] = read(self, s); } s.pos++; rvar(s); rvar(s); return v; }
      case 0x61: { var sl = rvar(s); v = new Array(sl); s.objects.push(v); while (s.bytes[s.pos] !== 0x40) { var sk = read(self, s); v[sk] = read(self, s); } s.pos++; rvar(s); rvar(s); return v; }
      case 0x72: {
        var C = Error, msg, stack, tag = rbyte(s), table = { 0x45: EvalError, 0x52: RangeError, 0x46: ReferenceError, 0x53: SyntaxError, 0x54: TypeError, 0x55: URIError };
        if (table[tag]) { C = table[tag]; tag = rbyte(s); }
        v = new C(); s.objects.push(v);
        for (; tag !== 0x2e; tag = rbyte(s)) { if (tag === 0x6d) msg = rstr(s, rtag(s)); else if (tag === 0x73) stack = rstr(s, rtag(s)); else throw bad(); }
        if (msg !== undefined) Object.defineProperty(v, 'message', { value: msg, writable: true, configurable: true, enumerable: false });
        if (stack !== undefined) Object.defineProperty(v, 'stack', { value: stack, writable: true, configurable: true, enumerable: false });
        return v;
      }
      case 0x5c: id = s.objects.length; s.objects.push(undefined); v = self._readHostObject(); s.objects[id] = v; return v;
    }
    throw bad();
  }
  function readView(s, buffer) {
    var sub = String.fromCharCode(rbyte(s)), offset = rvar(s), length = rvar(s); if (s.version >= 14) rvar(s);
    var map = { b: Int8Array, B: Uint8Array, C: Uint8ClampedArray, w: Int16Array, W: Uint16Array, d: Int32Array, D: Uint32Array, f: Float32Array, F: Float64Array, q: BigInt64Array, Q: BigUint64Array };
    var v = sub === '?' ? new DataView(buffer, offset, length) : new map[sub](buffer, offset, length / map[sub].BYTES_PER_ELEMENT);
    s.objects.push(v); return v;
  }
  DP.readHeader = function readHeader() {
    var s = st.get(this); if (s.bytes[s.pos] !== 0xff) throw new Error('Unable to deserialize cloned data due to invalid or unsupported version.');
    s.pos++; s.version = rvar(s);
    if (s.version > DEFAULT_VERSION) throw new Error('Unable to deserialize cloned data due to invalid or unsupported version.');
    return true;
  };
  DP.readValue = function readValue() { return read(this, st.get(this)); };
  DP.getWireFormatVersion = function getWireFormatVersion() { return st.get(this).version; };
  DP.transferArrayBuffer = function transferArrayBuffer(id, arrayBuffer) { };
  DP.readUint32 = function readUint32() { return rvar(st.get(this)) >>> 0; };
  DP.readUint64 = function readUint64() {
    var s = st.get(this), v = 0n, shift = 0n, b; do { b = rbyte(s); v |= BigInt(b & 127) << shift; shift += 7n; } while (b & 128);
    return [Number((v >> 32n) & 0xffffffffn), Number(v & 0xffffffffn)];
  };
  DP.readDouble = function readDouble() { return rf64(st.get(this)); };
  DP._readRawBytes = function _readRawBytes(length) { var s = st.get(this), at = s.pos; if (at + length > s.bytes.length) throw bad(); s.pos += length; return at; };
  DP.readRawBytes = function readRawBytes(length) { var at = this._readRawBytes(length), s = st.get(this); return Buffer.from(s.bytes.buffer, s.bytes.byteOffset + at, length); };
  DP._readHostObject = function _readHostObject() { throw bad(); };

  function DefaultSerializer() { Serializer.call(this); this._setTreatArrayBufferViewsAsHostObjects(true); }
  DefaultSerializer.prototype = Object.create(SP, { constructor: { value: DefaultSerializer, writable: true, configurable: true } });
  Object.setPrototypeOf(DefaultSerializer, Serializer);
  DefaultSerializer.prototype._writeHostObject = function _writeHostObject(view) {
    var index = 10; if (view instanceof DataView) index = 9; else if (!Buffer.isBuffer(view)) { for (var i = 0; i < VIEWS.length; i++) if (VIEWS[i][0] === view.constructor.name) index = i; }
    this.writeUint32(index); this.writeUint32(view.byteLength); this.writeRawBytes(new Uint8Array(view.buffer, view.byteOffset, view.byteLength));
  };
  function DefaultDeserializer(buffer) { Deserializer.call(this, buffer); }
  DefaultDeserializer.prototype = Object.create(DP, { constructor: { value: DefaultDeserializer, writable: true, configurable: true } });
  Object.setPrototypeOf(DefaultDeserializer, Deserializer);
  DefaultDeserializer.prototype._readHostObject = function _readHostObject() {
    var index = this.readUint32(), length = this.readUint32(), raw = this.readRawBytes(length), name = VIEWS[index] && VIEWS[index][0];
    if (name === undefined) throw bad();
    var copy = new Uint8Array(length); copy.set(raw);
    if (name === 'Buffer') return Buffer.from(copy.buffer, 0, length);
    if (name === 'DataView') return new DataView(copy.buffer, 0, length);
    var C = globalThis[name]; return new C(copy.buffer, 0, length / C.BYTES_PER_ELEMENT);
  };
  V.Serializer = Serializer; V.Deserializer = Deserializer; V.DefaultSerializer = DefaultSerializer; V.DefaultDeserializer = DefaultDeserializer;
})();
