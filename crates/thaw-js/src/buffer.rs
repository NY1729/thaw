//! `Buffer`, `TextEncoder`, `TextDecoder` on top of the typed arrays (UTF-8 conversion is native, the rest JS).

use crate::*;

impl Interp {
    pub(crate) fn install_buffer(&mut self) {
        let g = self.global.clone();
        let f = self.native(|it, _, a| {
            let s = it.to_str(&arg(a, 0));
            Ok(it.u8_from_vec(s.as_bytes().to_vec()))
        });
        g.declare("__thaw_utf8_encode", f, true);
        // (bytes, start, end, fatal) -> string, or `undefined` when `fatal` and the input is not UTF-8
        let f = self.native(|it, _, a| {
            let Some(t) = Self::typed_of(&arg(a, 0)) else {
                return it.throw_error("TypeError", "argument must be a typed array");
            };
            let bytes: Vec<u8> = {
                let d = t.data.borrow();
                d[t.off..t.off + t.len * t.ty.size()].to_vec()
            };
            let start = (it.num(&arg(a, 1)).max(0.0) as usize).min(bytes.len());
            let end = match arg(a, 2) {
                Value::Undefined => bytes.len(),
                v => (it.num(&v).max(0.0) as usize).min(bytes.len()),
            };
            let slice = &bytes[start..end.max(start)];
            if arg(a, 3).truthy() {
                return Ok(std::str::from_utf8(slice).map_or(Value::Undefined, Value::str));
            }
            Ok(Value::str(&String::from_utf8_lossy(slice)))
        });
        g.declare("__thaw_utf8_decode", f, true);
        self.run(BUFFER_PRELUDE)
            .unwrap_or_else(|_| panic!("thaw-js buffer prelude must evaluate"));
    }
}

const BUFFER_PRELUDE: &str = r#"
(function () {
  function def(o, name, f) {
    Object.defineProperty(o, name, { value: f, writable: true, enumerable: false, configurable: true });
  }
  var enc8 = __thaw_utf8_encode, dec8 = __thaw_utf8_decode;
  var B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  var B64URL = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';

  // ----- TextEncoder / TextDecoder -----
  function TextEncoder() {}
  def(TextEncoder.prototype, 'encoding', 'utf-8');
  def(TextEncoder.prototype, 'encode', function (s) { return enc8(s === undefined ? '' : String(s)); });
  def(TextEncoder.prototype, 'encodeInto', function (s, dest) {
    var bytes = enc8(String(s)), n = Math.min(bytes.length, dest.length);
    // never split a code point: back off to a character boundary
    while (n > 0 && n < bytes.length && (bytes[n] & 0xc0) === 0x80) n--;
    for (var i = 0; i < n; i++) dest[i] = bytes[i];
    return { read: dec8(bytes, 0, n).length, written: n };
  });
  globalThis.TextEncoder = TextEncoder;

  var LABELS = { 'utf-8': 'utf-8', 'utf8': 'utf-8', 'unicode-1-1-utf-8': 'utf-8', 'utf-16le': 'utf-16le', 'utf-16': 'utf-16le', 'ucs-2': 'utf-16le', 'latin1': 'windows-1252', 'iso-8859-1': 'windows-1252', 'windows-1252': 'windows-1252', 'ascii': 'windows-1252', 'us-ascii': 'windows-1252' };
  function TextDecoder(label, options) {
    var enc = LABELS[String(label === undefined ? 'utf-8' : label).trim().toLowerCase()];
    if (!enc) { var e = new RangeError('The "' + label + '" encoding is not supported'); e.code = 'ERR_ENCODING_NOT_SUPPORTED'; throw e; }
    this.encoding = enc;
    this.fatal = !!(options && options.fatal);
    this.ignoreBOM = !!(options && options.ignoreBOM);
  }
  def(TextDecoder.prototype, 'decode', function (input) {
    if (input === undefined) return '';
    var bytes = input instanceof ArrayBuffer ? new Uint8Array(input) : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
    var s;
    if (this.encoding === 'utf-8') {
      s = dec8(bytes, 0, bytes.length, this.fatal);
      if (s === undefined) { var e = new TypeError('The encoded data was not valid for encoding utf-8'); e.code = 'ERR_ENCODING_INVALID_ENCODED_DATA'; throw e; }
      if (!this.ignoreBOM && s.charCodeAt(0) === 0xfeff) s = s.slice(1);
    } else if (this.encoding === 'utf-16le') {
      s = '';
      for (var i = 0; i + 1 < bytes.length; i += 2) s += String.fromCharCode(bytes[i] | (bytes[i + 1] << 8));
      if (!this.ignoreBOM && s.charCodeAt(0) === 0xfeff) s = s.slice(1);
    } else {
      s = '';
      for (var j = 0; j < bytes.length; j++) s += String.fromCharCode(bytes[j]);
    }
    return s;
  });
  globalThis.TextDecoder = TextDecoder;

  // ----- Buffer -----
  var U8 = Uint8Array;
  function Buffer(arg, enc, len) {
    if (typeof arg === 'number') return Buffer.alloc(arg);
    return Buffer.from(arg, enc, len);
  }
  Buffer.prototype = Object.create(U8.prototype);
  def(Buffer.prototype, 'constructor', Buffer);
  def(Buffer.prototype, '__isBuffer', true);
  Object.setPrototypeOf(Buffer, U8);
  function wrap(u8) { Object.setPrototypeOf(u8, Buffer.prototype); return u8; }
  function normEnc(e) {
    e = e === undefined || e === null ? 'utf8' : String(e).toLowerCase();
    switch (e) {
      case 'utf8': case 'utf-8': return 'utf8';
      case 'hex': case 'base64': case 'base64url': case 'ascii': return e;
      case 'latin1': case 'binary': return 'latin1';
      case 'ucs2': case 'ucs-2': case 'utf16le': case 'utf-16le': return 'utf16le';
    }
    var err = new TypeError('Unknown encoding: ' + e);
    err.code = 'ERR_UNKNOWN_ENCODING';
    throw err;
  }
  function fromString(s, enc) {
    enc = normEnc(enc);
    var out, i, n;
    if (enc === 'utf8') return enc8(s);
    if (enc === 'hex') {
      n = s.length >> 1; out = [];
      for (i = 0; i < n; i++) {
        var h = parseInt(s.substr(i * 2, 2), 16);
        if (h !== h || !/^[0-9a-fA-F]{2}$/.test(s.substr(i * 2, 2))) break;
        out.push(h);
      }
      return new U8(out);
    }
    if (enc === 'base64' || enc === 'base64url') {
      out = []; var acc = 0, bits = 0;
      for (i = 0; i < s.length; i++) {
        var c = s[i], v = B64.indexOf(c);
        if (v < 0) v = B64URL.indexOf(c);
        if (c === '=') break;
        if (v < 0) continue;
        acc = (acc << 6) | v; bits += 6;
        if (bits >= 8) { bits -= 8; out.push((acc >> bits) & 255); }
      }
      return new U8(out);
    }
    if (enc === 'utf16le') {
      out = [];
      for (i = 0; i < s.length; i++) { var u = s.charCodeAt(i); out.push(u & 255, u >> 8); }
      return new U8(out);
    }
    out = [];
    for (i = 0; i < s.length; i++) out.push(s.charCodeAt(i) & 255);
    return new U8(out);
  }
  def(Buffer, 'from', function (v, a, b) {
    if (typeof v === 'string') return wrap(fromString(v, a));
    if (v instanceof ArrayBuffer) return wrap(new U8(v, a === undefined ? 0 : a, b === undefined ? v.byteLength - (a || 0) : b));
    if (ArrayBuffer.isView(v)) return wrap(new U8(Array.from(v)));
    if (v && typeof v === 'object') {
      if (v.type === 'Buffer' && Array.isArray(v.data)) return wrap(new U8(v.data));
      if (typeof v.length === 'number' || Array.isArray(v)) return wrap(new U8(v));
    }
    var e = new TypeError('The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object. Received ' + (v === null ? 'null' : v === undefined ? 'undefined' : 'type ' + typeof v + ' (' + String(v) + ')'));
    e.code = 'ERR_INVALID_ARG_TYPE';
    throw e;
  });
  def(Buffer, 'alloc', function (n, fill, enc) {
    if (typeof n !== 'number') { var e = new TypeError('The "size" argument must be of type number. Received ' + (n === undefined ? 'undefined' : 'type ' + typeof n)); e.code = 'ERR_INVALID_ARG_TYPE'; throw e; }
    var b = wrap(new U8(n));
    if (fill !== undefined && fill !== 0) b.fill(fill, 0, n, enc);
    return b;
  });
  def(Buffer, 'allocUnsafe', function (n) { return Buffer.alloc(n); });
  def(Buffer, 'allocUnsafeSlow', function (n) { return Buffer.alloc(n); });
  def(Buffer, 'isBuffer', function (b) { return b instanceof Buffer; });
  def(Buffer, 'isEncoding', function (e) { try { normEnc(e); return typeof e === 'string'; } catch (x) { return false; } });
  def(Buffer, 'byteLength', function (v, enc) {
    if (typeof v === 'string') return fromString(v, enc).length;
    return v.byteLength;
  });
  def(Buffer, 'concat', function (list, total) {
    if (total === undefined) { total = 0; for (var i = 0; i < list.length; i++) total += list[i].length; }
    var out = Buffer.alloc(total), pos = 0;
    for (var j = 0; j < list.length && pos < total; j++) {
      var item = list[j];
      for (var k = 0; k < item.length && pos < total; k++) out[pos++] = item[k];
    }
    return out;
  });
  def(Buffer, 'compare', function (a, b) {
    var n = Math.min(a.length, b.length);
    for (var i = 0; i < n; i++) if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
    return a.length < b.length ? -1 : a.length > b.length ? 1 : 0;
  });
  def(Buffer, 'poolSize', 8192);

  var P = Buffer.prototype;
  def(P, 'toString', function (enc, start, end) {
    start = start === undefined ? 0 : Math.max(0, start);
    end = end === undefined ? this.length : Math.min(this.length, end);
    if (end <= start) return '';
    enc = normEnc(enc);
    var i, s = '';
    if (enc === 'utf8') return dec8(this, start, end);
    if (enc === 'hex') { for (i = start; i < end; i++) s += (this[i] < 16 ? '0' : '') + this[i].toString(16); return s; }
    if (enc === 'base64' || enc === 'base64url') {
      var T = enc === 'base64' ? B64 : B64URL;
      for (i = start; i < end; i += 3) {
        var a = this[i], b = i + 1 < end ? this[i + 1] : 0, c = i + 2 < end ? this[i + 2] : 0;
        var n = (a << 16) | (b << 8) | c;
        s += T[(n >> 18) & 63] + T[(n >> 12) & 63];
        s += i + 1 < end ? T[(n >> 6) & 63] : (enc === 'base64' ? '=' : '');
        s += i + 2 < end ? T[n & 63] : (enc === 'base64' ? '=' : '');
      }
      return s;
    }
    if (enc === 'utf16le') { for (i = start; i + 1 < end; i += 2) s += String.fromCharCode(this[i] | (this[i + 1] << 8)); return s; }
    for (i = start; i < end; i++) s += String.fromCharCode(enc === 'ascii' ? this[i] & 127 : this[i]);
    return s;
  });
  def(P, 'toJSON', function () { return { type: 'Buffer', data: Array.from(this) }; });
  def(P, 'equals', function (o) { return Buffer.compare(this, o) === 0; });
  def(P, 'compare', function (o) { return Buffer.compare(this, o); });
  def(P, 'write', function (s, off, len, enc) {
    if (typeof off === 'string') { enc = off; off = 0; len = undefined; } else if (typeof len === 'string') { enc = len; len = undefined; }
    off = off === undefined ? 0 : off;
    var bytes = fromString(String(s), enc), n = Math.min(bytes.length, this.length - off, len === undefined ? Infinity : len);
    for (var i = 0; i < n; i++) this[off + i] = bytes[i];
    return n;
  });
  def(P, 'fill', function (v, start, end, enc) {
    if (typeof start === 'string') { enc = start; start = 0; end = this.length; }
    start = start === undefined ? 0 : start; end = end === undefined ? this.length : end;
    var pat = typeof v === 'string' ? fromString(v, enc) : (typeof v === 'number' ? [v & 255] : v);
    if (pat.length === 0) pat = [0];
    for (var i = start, j = 0; i < end; i++, j++) this[i] = pat[j % pat.length];
    return this;
  });
  def(P, 'copy', function (target, ts, ss, se) {
    ts = ts || 0; ss = ss || 0; se = se === undefined ? this.length : se;
    var n = Math.min(se - ss, target.length - ts);
    if (n <= 0) return 0;
    var tmp = Array.from(this.subarray(ss, ss + n));
    for (var i = 0; i < n; i++) target[ts + i] = tmp[i];
    return n;
  });
  def(P, 'slice', function (s, e) { return this.subarray(s, e); });
  function search(buf, v, from, enc, last) {
    var needle = typeof v === 'number' ? [v & 255] : typeof v === 'string' ? fromString(v, enc) : v;
    from = from === undefined ? (last ? buf.length : 0) : (from < 0 ? Math.max(buf.length + from, 0) : from);
    if (needle.length === 0) return Math.min(from, buf.length);
    var lo = last ? Math.min(from, buf.length - needle.length) : from, i, j;
    if (last) {
      for (i = lo; i >= 0; i--) { for (j = 0; j < needle.length && buf[i + j] === needle[j]; j++); if (j === needle.length) return i; }
    } else {
      for (i = lo; i + needle.length <= buf.length; i++) { for (j = 0; j < needle.length && buf[i + j] === needle[j]; j++); if (j === needle.length) return i; }
    }
    return -1;
  }
  def(P, 'indexOf', function (v, from, enc) { return search(this, v, typeof from === 'string' ? undefined : from, typeof from === 'string' ? from : enc, false); });
  def(P, 'lastIndexOf', function (v, from, enc) { return search(this, v, typeof from === 'string' ? undefined : from, typeof from === 'string' ? from : enc, true); });
  def(P, 'includes', function (v, from, enc) { return this.indexOf(v, from, enc) !== -1; });

  // fixed-width integer / float accessors
  var scratch = new ArrayBuffer(8), sb = new Uint8Array(scratch), f32 = new Float32Array(scratch), f64 = new Float64Array(scratch);
  function rangeErr(off, len, buf) {
    var e;
    if (typeof off !== 'number' || off !== Math.floor(off)) { e = new TypeError('The "offset" argument must be of type number. Received ' + (off === undefined ? 'undefined' : typeof off)); e.code = 'ERR_INVALID_ARG_TYPE'; return e; }
    e = new RangeError(buf.length - len < 0 ? 'Attempt to access memory outside buffer bounds' : 'The value of "offset" is out of range. It must be >= 0 and <= ' + (buf.length - len) + '. Received ' + off);
    e.code = buf.length - len < 0 ? 'ERR_BUFFER_OUT_OF_BOUNDS' : 'ERR_OUT_OF_RANGE';
    return e;
  }
  function defRW(name, size, signed, little, isFloat) {
    def(P, 'read' + name, function (off) {
      off = off === undefined ? 0 : off;
      if (typeof off !== 'number' || off < 0 || off + size > this.length || off !== Math.floor(off)) throw rangeErr(off, size, this);
      var i;
      if (isFloat) {
        for (i = 0; i < size; i++) sb[i] = this[off + (little ? i : size - 1 - i)];
        return size === 4 ? f32[0] : f64[0];
      }
      var v = 0;
      for (i = 0; i < size; i++) v = v * 256 + this[off + (little ? size - 1 - i : i)];
      if (signed && v >= Math.pow(2, size * 8 - 1)) v -= Math.pow(2, size * 8);
      return v;
    });
    def(P, 'write' + name, function (v, off) {
      off = off === undefined ? 0 : off;
      if (typeof off !== 'number' || off < 0 || off + size > this.length || off !== Math.floor(off)) throw rangeErr(off, size, this);
      var i;
      if (isFloat) {
        if (size === 4) f32[0] = v; else f64[0] = v;
        for (i = 0; i < size; i++) this[off + (little ? i : size - 1 - i)] = sb[i];
        return off + size;
      }
      var bits = size * 8, lo = signed ? -Math.pow(2, bits - 1) : 0, hi = signed ? Math.pow(2, bits - 1) - 1 : Math.pow(2, bits) - 1;
      if (v < lo || v > hi) { var e = new RangeError('The value of "value" is out of range. It must be >= ' + lo + ' and <= ' + hi + '. Received ' + v); e.code = 'ERR_OUT_OF_RANGE'; throw e; }
      if (v < 0) v += Math.pow(2, bits);
      for (i = 0; i < size; i++) { this[off + (little ? i : size - 1 - i)] = v % 256; v = Math.floor(v / 256); }
      return off + size;
    });
  }
  [['UInt8', 1, false], ['Int8', 1, true], ['UInt16', 2, false], ['Int16', 2, true], ['UInt32', 4, false], ['Int32', 4, true]].forEach(function (d) {
    if (d[1] === 1) { defRW(d[0], 1, d[2], true, false); return; }
    defRW(d[0] + 'LE', d[1], d[2], true, false);
    defRW(d[0] + 'BE', d[1], d[2], false, false);
  });
  defRW('FloatLE', 4, true, true, true); defRW('FloatBE', 4, true, false, true);
  defRW('DoubleLE', 8, true, true, true); defRW('DoubleBE', 8, true, false, true);
  ['UInt8', 'UInt16LE', 'UInt16BE', 'UInt32LE', 'UInt32BE'].forEach(function (n) {
    def(P, 'readUint' + n.slice(4), P['readUInt' + n.slice(4)]);
    def(P, 'writeUint' + n.slice(4), P['writeUInt' + n.slice(4)]);
  });
  globalThis.Buffer = Buffer;
})();
"#;
