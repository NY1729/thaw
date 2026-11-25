//! `Buffer`, `TextEncoder`, `TextDecoder` on top of the typed arrays (UTF-8 conversion is native, the rest JS).

use crate::*;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// `(typed array, start, end)` arguments as the bytes they select.
fn byte_window(it: &mut Interp, a: &[Value]) -> R<Vec<u8>> {
    let Some(t) = Interp::typed_of(&arg(a, 0)) else {
        return it.throw_error("TypeError", "argument must be a typed array");
    };
    let bytes = t.window();
    let start = (it.num(&arg(a, 1)).max(0.0) as usize).min(bytes.len());
    let end = (it.num(&arg(a, 2)).max(0.0) as usize).min(bytes.len());
    Ok(bytes[start..end.max(start)].to_vec())
}

impl Interp {
    pub(crate) fn install_buffer(&mut self) {
        let g = self.global.clone();
        let f = self.native(|it, _, a| {
            let s = it.to_str(&arg(a, 0));
            // a lone surrogate is U+FFFD in UTF-8 (the string layer keeps it as a private-use char)
            Ok(it.u8_from_vec(crate::builtins::lossy_out(&s).as_bytes().to_vec()))
        });
        g.declare("__thaw_utf8_encode", f, true);
        // (bytes, start, end, fatal) -> string, or `undefined` when `fatal` and the input is not UTF-8
        let f = self.native(|it, _, a| {
            let Some(t) = Self::typed_of(&arg(a, 0)) else {
                return it.throw_error("TypeError", "argument must be a typed array");
            };
            let bytes: Vec<u8> = t.window();
            let start = (it.num(&arg(a, 1)).max(0.0) as usize).min(bytes.len());
            let end = match arg(a, 2) {
                Value::Undefined => bytes.len(),
                v => (it.num(&v).max(0.0) as usize).min(bytes.len()),
            };
            let slice = &bytes[start..end.max(start)];
            if arg(a, 3).truthy() {
                return Ok(std::str::from_utf8(slice).map_or(Value::Undefined, |t| {
                    Value::str(&crate::builtins::from_scalar(t))
                }));
            }
            Ok(Value::str(&crate::builtins::from_scalar(
                &String::from_utf8_lossy(slice),
            )))
        });
        g.declare("__thaw_utf8_decode", f, true);
        // hex string -> bytes, stopping at the first pair that is not two hex digits (as Node does)
        let f = self.native(|it, _, a| {
            let s = it.to_str(&arg(a, 0));
            let digit = |c: u8| match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                b'A'..=b'F' => Some(c - b'A' + 10),
                _ => None,
            };
            let bytes: Vec<u8> = s
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map_while(|pair| Some(digit(pair[0])? * 16 + digit(pair[1])?))
                .collect();
            Ok(it.u8_from_vec(bytes))
        });
        g.declare("__thaw_hex_decode", f, true);
        // number of trailing bytes that are a valid but incomplete UTF-8 sequence (streaming decode keeps them)
        let f = self.native(|it, _, a| {
            let Some(t) = Self::typed_of(&arg(a, 0)) else {
                return it.throw_error("TypeError", "argument must be a typed array");
            };
            let bytes = t.window();
            let tail = (1..=3.min(bytes.len())).find(|&k| {
                matches!(std::str::from_utf8(&bytes[bytes.len() - k..]),
                    Err(e) if e.valid_up_to() == 0 && e.error_len().is_none())
            });
            Ok(Value::Number(tail.unwrap_or(0) as f64))
        });
        g.declare("__thaw_utf8_tail", f, true);
        // (bytes, start, end, url) -> base64 / base64url text
        let f = self.native(|it, _, a| {
            let bytes = byte_window(it, a)?;
            let url = arg(a, 3).truthy();
            let table: &[u8; 64] = if url { B64URL } else { B64 };
            let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
            for c in bytes.chunks(3) {
                let n = u32::from(c[0]) << 16
                    | u32::from(*c.get(1).unwrap_or(&0)) << 8
                    | u32::from(*c.get(2).unwrap_or(&0));
                let ch = |sh: u32| table[(n >> sh & 63) as usize] as char;
                out.push(ch(18));
                out.push(ch(12));
                match c.len() {
                    1 if !url => out.push_str("=="),
                    1 => {}
                    2 => {
                        out.push(ch(6));
                        if !url {
                            out.push('=');
                        }
                    }
                    _ => {
                        out.push(ch(6));
                        out.push(ch(0));
                    }
                }
            }
            Ok(Value::str(&out))
        });
        g.declare("__thaw_b64_encode", f, true);
        // text -> bytes: either alphabet, invalid characters skipped, `=` ends the data (as Node does)
        let f = self.native(|it, _, a| {
            let s = it.to_str(&arg(a, 0));
            let (mut acc, mut bits) = (0u32, 0u32);
            let mut out = Vec::with_capacity(s.len() / 4 * 3);
            for c in s.chars() {
                let v = match c {
                    'A'..='Z' => c as u32 - 'A' as u32,
                    'a'..='z' => c as u32 - 'a' as u32 + 26,
                    '0'..='9' => c as u32 - '0' as u32 + 52,
                    '+' | '-' => 62,
                    '/' | '_' => 63,
                    '=' => break,
                    _ => continue,
                };
                acc = acc << 6 | v;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    out.push((acc >> bits & 255) as u8);
                }
            }
            Ok(it.u8_from_vec(out))
        });
        g.declare("__thaw_b64_decode", f, true);
        let f = self.native(|it, _, a| {
            let bytes = byte_window(it, a)?;
            let mut out = String::with_capacity(bytes.len() * 2);
            for b in bytes {
                out.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
                out.push(char::from_digit(u32::from(b & 15), 16).unwrap_or('0'));
            }
            Ok(Value::str(&out))
        });
        g.declare("__thaw_hex_encode", f, true);
        // (bytes, start, end, ascii) -> one char per byte (ascii masks to 7 bits)
        let f = self.native(|it, _, a| {
            let bytes = byte_window(it, a)?;
            let mask = if arg(a, 3).truthy() { 0x7f } else { 0xff };
            Ok(Value::str(
                &bytes
                    .iter()
                    .map(|&b| char::from(b & mask))
                    .collect::<String>(),
            ))
        });
        g.declare("__thaw_latin1_decode", f, true);
        // btoa: `undefined` when a character is above U+00FF
        let f = self.native(|it, _, a| {
            let s = it.to_str(&arg(a, 0));
            if s.chars().any(|c| c as u32 > 255) {
                return Ok(Value::Undefined);
            }
            let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
            let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
            for c in bytes.chunks(3) {
                let n = u32::from(c[0]) << 16
                    | u32::from(*c.get(1).unwrap_or(&0)) << 8
                    | u32::from(*c.get(2).unwrap_or(&0));
                let ch = |sh: u32| B64[(n >> sh & 63) as usize] as char;
                out.push(ch(18));
                out.push(ch(12));
                out.push(if c.len() > 1 { ch(6) } else { '=' });
                out.push(if c.len() > 2 { ch(0) } else { '=' });
            }
            Ok(Value::str(&out))
        });
        g.declare("__thaw_btoa", f, true);
        // atob (forgiving-base64 decode): `undefined` when the text is not correctly encoded
        let f = self.native(|it, _, a| {
            let s = it.to_str(&arg(a, 0));
            let mut t: Vec<char> = s.chars().filter(|c| !"\t\n\x0c\r ".contains(*c)).collect();
            if t.len().is_multiple_of(4) {
                for _ in 0..2 {
                    if t.last() == Some(&'=') {
                        t.pop();
                    }
                }
            }
            if t.len() % 4 == 1
                || t.iter()
                    .any(|c| !(c.is_ascii_alphanumeric() || *c == '+' || *c == '/'))
            {
                return Ok(Value::Undefined);
            }
            let (mut acc, mut bits) = (0u32, 0u32);
            let mut out = String::with_capacity(t.len() * 3 / 4);
            for c in t {
                let v = B64.iter().position(|&b| b as char == c).unwrap_or(0) as u32;
                acc = acc << 6 | v;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    out.push(char::from((acc >> bits & 255) as u8));
                }
            }
            Ok(Value::str(&out))
        });
        g.declare("__thaw_atob", f, true);
        self.run_builtin(BUFFER_PRELUDE)
            .unwrap_or_else(|_| panic!("thaw-js buffer prelude must evaluate"));
    }
}

const BUFFER_PRELUDE: &str = r#"
(function () {
  function def(o, name, f) {
    Object.defineProperty(o, name, { value: f, writable: true, enumerable: false, configurable: true });
  }
  var enc8 = __thaw_utf8_encode, dec8 = __thaw_utf8_decode, hexDec = __thaw_hex_decode,
    b64Dec = __thaw_b64_decode, b64Enc = __thaw_b64_encode, hexEnc = __thaw_hex_encode, latin1Dec = __thaw_latin1_decode;
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
    if (enc === 'hex') return hexDec(s);
    if (enc === 'base64' || enc === 'base64url') return b64Dec(s);
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
    // no length: a view to the buffer's end (length-tracking when it is resizable, as in Node)
    if (v instanceof ArrayBuffer) return wrap(new U8(v, a === undefined ? 0 : a, b));
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
  def(Buffer, 'of', (...items) => Buffer.from(items));
  function rangeArg(v, name) {
    if (typeof v !== 'number') { var te = new TypeError('The "' + name + '" argument must be of type number. Received type ' + typeof v + ' (' + String(v) + ')'); te.code = 'ERR_INVALID_ARG_TYPE'; throw te; }
    if (!(v >= 0 && v <= 9007199254740991)) { var re = new RangeError('The value of "' + name + '" is out of range. It must be >= 0 && <= 9007199254740991. Received ' + v); re.code = 'ERR_OUT_OF_RANGE'; throw re; }
    return v;
  }
  def(Buffer, 'copyBytesFrom', function copyBytesFrom(view, offset, length) {
    if (!ArrayBuffer.isView(view) || view instanceof DataView) {
      var e = new TypeError('The "view" argument must be an instance of TypedArray. Received ' + (view && typeof view === 'object' ? 'an instance of ' + view.constructor.name : 'type ' + typeof view + ' (' + String(view) + ')'));
      e.code = 'ERR_INVALID_ARG_TYPE';
      throw e;
    }
    offset = offset === undefined ? 0 : rangeArg(offset, 'offset');
    var avail = Math.max(0, view.length - offset);
    var count = length === undefined ? avail : Math.min(rangeArg(length, 'length'), avail);
    if (count === 0) return wrap(new U8(0));
    var bpe = view.BYTES_PER_ELEMENT;
    return wrap(new U8(new U8(view.buffer, view.byteOffset + offset * bpe, count * bpe)));
  });
  // ponytail: Node's FastBuffer is its own class over Buffer.prototype; reusing Buffer only matches the name and `instanceof`.
  function FastBuffer(a, b, c) { return Buffer(a, b, c); }
  FastBuffer.prototype = Buffer.prototype;
  Object.defineProperty(Buffer, Symbol.species, { get() { return FastBuffer; }, configurable: true });
  def(Buffer, 'isBuffer', function (b) { return b instanceof Buffer; });
  def(Buffer, 'isEncoding', function (e) { try { normEnc(e); return typeof e === 'string'; } catch (x) { return false; } });
  def(Buffer, 'byteLength', function (v, enc) {
    if (typeof v === 'string') return fromString(v, enc).length;
    return v.byteLength;
  });
  def(Buffer, 'concat', function (list, total) {
    // Buffers inside an array argument cross the typed ABI as {type:'Buffer', data:[...]}: normalize first.
    var items = [];
    for (var i = 0; i < list.length; i++) {
      var it = list[i];
      if (!ArrayBuffer.isView(it) && it && typeof it === 'object' && it.type === 'Buffer' && Array.isArray(it.data)) it = Buffer.from(it.data);
      items.push(it);
    }
    if (total === undefined || total !== total) { total = 0; for (var n = 0; n < items.length; n++) total += items[n].length; }
    var out = Buffer.alloc(total), pos = 0;
    for (var j = 0; j < items.length && pos < total; j++) {
      var item = items[j];
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
    if (enc === 'hex') return hexEnc(this, start, end);
    if (enc === 'base64' || enc === 'base64url') return b64Enc(this, start, end, enc === 'base64url');
    if (enc === 'utf16le') { for (i = start; i + 1 < end; i += 2) s += String.fromCharCode(this[i] | (this[i + 1] << 8)); return s; }
    return latin1Dec(this, start, end, enc === 'ascii');
  });
  def(P, 'toJSON', function () { return { type: 'Buffer', data: Array.from(this) }; });
  def(P, 'equals', function (o) { return Buffer.compare(this, o) === 0; });
  def(P, 'compare', function (o) { return Buffer.compare(this, o); });
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
  def(P, 'swap16', function () {
    if (this.length % 2) throw new RangeError('Buffer size must be a multiple of 16-bits');
    for (var i = 0; i < this.length; i += 2) { var t = this[i]; this[i] = this[i + 1]; this[i + 1] = t; }
    return this;
  });
  def(P, 'swap32', function () {
    if (this.length % 4) throw new RangeError('Buffer size must be a multiple of 32-bits');
    for (var i = 0; i < this.length; i += 4) {
      var a = this[i], b = this[i + 1];
      this[i] = this[i + 3]; this[i + 1] = this[i + 2]; this[i + 2] = b; this[i + 3] = a;
    }
    return this;
  });
  def(P, 'swap64', function () {
    if (this.length % 8) throw new RangeError('Buffer size must be a multiple of 64-bits');
    for (var i = 0; i < this.length; i += 8) {
      for (var o = 0; o < 4; o++) { var t = this[i + o]; this[i + o] = this[i + 7 - o]; this[i + 7 - o] = t; }
    }
    return this;
  });
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
  // ----- variable-width ints, BigInt64, string slice/write, subarray, inspect, parent, offset (Node 22 checks) -----
  function recv(v) {
    if (v === null || v === undefined) return 'Received ' + v;
    if (typeof v === 'function') return 'Received function ' + v.name;
    if (typeof v === 'object') return 'Received an instance of ' + (v.constructor ? v.constructor.name : 'Object');
    return 'Received type ' + typeof v + ' (' + (typeof v === 'string' ? "'" + v.slice(0, 25) + "'" : String(v)) + ')';
  }
  function sep(s) {
    var start = s[0] === '-' ? 1 : 0, i = s.length, res = '';
    for (; i >= start + 4; i -= 3) res = '_' + s.slice(i - 3, i) + res;
    return s.slice(0, i) + res;
  }
  function outOfRange(name, range, value) {
    var got;
    if (typeof value === 'bigint') {
      got = String(value);
      if (value > 4294967296n || value < -4294967296n) got = sep(got);
      got += 'n';
    } else if (Number.isInteger(value) && Math.abs(value) > 4294967296) got = sep(String(value));
    else got = String(value);
    var e = new RangeError('The value of "' + name + '" is out of range. It must be ' + range + '. Received ' + got);
    e.code = 'ERR_OUT_OF_RANGE';
    return e;
  }
  function validateNumber(v, name) {
    if (typeof v !== 'number') { var e = new TypeError('The "' + name + '" argument must be of type number. ' + recv(v)); e.code = 'ERR_INVALID_ARG_TYPE'; throw e; }
  }
  function bufBounds(what) {
    var e = what ? new RangeError('"' + what + '" is outside of buffer bounds') : new RangeError('Attempt to access memory outside buffer bounds');
    e.code = 'ERR_BUFFER_OUT_OF_BOUNDS';
    return e;
  }
  function boundsError(value, length, name) {
    if (Math.floor(value) !== value) { validateNumber(value, name || 'offset'); throw outOfRange(name || 'offset', 'an integer', value); }
    if (length < 0) throw bufBounds();
    throw outOfRange(name || 'offset', '>= ' + (name ? 1 : 0) + ' and <= ' + length, value);
  }
  function checkByteLength(bl) {
    if (Math.floor(bl) !== bl || bl < 1 || bl > 6) boundsError(bl, 6, 'byteLength');
  }
  function checkBounds(buf, off, n) {
    validateNumber(off, 'offset');
    if (!(off >= 0 && Math.floor(off) === off && off <= buf.length - n)) boundsError(off, buf.length - n);
  }
  function checkIntValue(v, bl, signed) {
    var bits = 8 * bl, min = signed ? -Math.pow(2, bits - 1) : 0, max = signed ? Math.pow(2, bits - 1) - 1 : Math.pow(2, bits) - 1;
    if (v > max || v < min) {
      // Node's checkInt: byteLength 5 and 6 use the power form, 1..4 the numeric bounds
      var range = bl >= 5
        ? (signed ? '>= -(2 ** ' + (bits - 1) + ') and < 2 ** ' + (bits - 1) : '>= 0 and < 2 ** ' + bits)
        : '>= ' + min + ' and <= ' + max;
      throw outOfRange('value', range, v);
    }
  }
  // Multiply-accumulate (not bit shifts): exact for every byteLength up to 6 (< 2^48).
  function getInt(buf, off, bl, little, signed) {
    var v = 0;
    for (var i = 0; i < bl; i++) v = v * 256 + buf[off + (little ? bl - 1 - i : i)];
    if (signed && v >= Math.pow(2, 8 * bl - 1)) v -= Math.pow(2, 8 * bl);
    return v;
  }
  function putInt(buf, v, off, bl, little) {
    // two's complement from the truncated value; each byte is the low 8 bits of the remaining value
    var t = Math.trunc(v);
    if (t !== t) t = 0;
    if (t < 0) t += Math.pow(2, 8 * bl);
    for (var i = 0; i < bl; i++) {
      var b = t % 256;
      buf[off + (little ? i : bl - 1 - i)] = b;
      t = (t - b) / 256;
    }
  }
  function readVarInt(buf, offset, byteLength, little, signed) {
    if (offset === undefined) validateNumber(offset, 'offset');
    checkByteLength(byteLength);
    checkBounds(buf, offset, byteLength);
    return getInt(buf, offset, byteLength, little, signed);
  }
  function writeVarInt(buf, value, offset, byteLength, little, signed) {
    checkByteLength(byteLength);
    checkIntValue(Number(value), byteLength, signed);
    checkBounds(buf, offset, byteLength);
    putInt(buf, Number(value), offset, byteLength, little);
    return offset + byteLength;
  }
  def(P, 'readUIntLE', function readUIntLE(offset, byteLength) { return readVarInt(this, offset, byteLength, true, false); });
  def(P, 'readUIntBE', function readUIntBE(offset, byteLength) { return readVarInt(this, offset, byteLength, false, false); });
  def(P, 'readIntLE', function readIntLE(offset, byteLength) { return readVarInt(this, offset, byteLength, true, true); });
  def(P, 'readIntBE', function readIntBE(offset, byteLength) { return readVarInt(this, offset, byteLength, false, true); });
  def(P, 'writeUIntLE', function writeUIntLE(value, offset, byteLength) { return writeVarInt(this, value, offset, byteLength, true, false); });
  def(P, 'writeUIntBE', function writeUIntBE(value, offset, byteLength) { return writeVarInt(this, value, offset, byteLength, false, false); });
  def(P, 'writeIntLE', function writeIntLE(value, offset, byteLength) { return writeVarInt(this, value, offset, byteLength, true, true); });
  def(P, 'writeIntBE', function writeIntBE(value, offset, byteLength) { return writeVarInt(this, value, offset, byteLength, false, true); });
  def(P, 'readUintLE', P.readUIntLE); def(P, 'readUintBE', P.readUIntBE);
  def(P, 'writeUintLE', P.writeUIntLE); def(P, 'writeUintBE', P.writeUIntBE);

  function readBig(buf, off, little, signed) {
    validateNumber(off, 'offset');
    checkBounds(buf, off, 8);
    var lo = getInt(buf, little ? off : off + 4, 4, little, false);
    var hi = getInt(buf, little ? off + 4 : off, 4, little, false);
    var v = BigInt(hi) * 4294967296n + BigInt(lo);
    if (signed && v >= 9223372036854775808n) v -= 18446744073709551616n;
    return v;
  }
  function writeBig(buf, value, off, little, signed) {
    var min = signed ? -9223372036854775808n : 0n, max = signed ? 9223372036854775807n : 18446744073709551615n;
    if (value > max || value < min) {
      throw outOfRange('value', signed ? '>= -(2n ** 63n) and < 2n ** 63n' : '>= 0n and < 2n ** 64n', value);
    }
    validateNumber(off, 'offset');
    checkBounds(buf, off, 8);
    // a Number value throws the BigInt mixing TypeError here, as Node does
    if (value < 0n) value += 18446744073709551616n;
    var lo = Number(value % 4294967296n), hi = Number(value / 4294967296n);
    putInt(buf, lo, little ? off : off + 4, 4, little);
    putInt(buf, hi, little ? off + 4 : off, 4, little);
    return off + 8;
  }
  def(P, 'readBigInt64LE', function readBigInt64LE(offset = 0) { return readBig(this, offset, true, true); });
  def(P, 'readBigInt64BE', function readBigInt64BE(offset = 0) { return readBig(this, offset, false, true); });
  def(P, 'readBigUInt64LE', function readBigUInt64LE(offset = 0) { return readBig(this, offset, true, false); });
  def(P, 'readBigUInt64BE', function readBigUInt64BE(offset = 0) { return readBig(this, offset, false, false); });
  def(P, 'writeBigInt64LE', function writeBigInt64LE(value, offset = 0) { return writeBig(this, value, offset, true, true); });
  def(P, 'writeBigInt64BE', function writeBigInt64BE(value, offset = 0) { return writeBig(this, value, offset, false, true); });
  def(P, 'writeBigUInt64LE', function writeBigUInt64LE(value, offset = 0) { return writeBig(this, value, offset, true, false); });
  def(P, 'writeBigUInt64BE', function writeBigUInt64BE(value, offset = 0) { return writeBig(this, value, offset, false, false); });
  def(P, 'readBigUint64LE', P.readBigUInt64LE); def(P, 'readBigUint64BE', P.readBigUInt64BE);
  def(P, 'writeBigUint64LE', P.writeBigUInt64LE); def(P, 'writeBigUint64BE', P.writeBigUInt64BE);

  // string slices: ParseArrayIndex (undefined -> default, NaN -> 0, truncate); negatives and end > length throw
  function toIndex(v, dflt) {
    if (v === undefined) return dflt;
    var n = Math.trunc(Number(v));
    return n === n ? n : 0;
  }
  function oor() { var e = new RangeError('Index out of range'); e.code = 'ERR_OUT_OF_RANGE'; return e; }
  function sliceOf(buf, enc, a) {
    var s = toIndex(a[0], 0), e = toIndex(a[1], buf.length);
    if (s < 0 || e < 0) throw oor();
    if (e < s) e = s;
    if (e > buf.length) throw oor();
    return P.toString.call(buf, enc, s, e);
  }
  // utf8/latin1/ascii report a negative offset or length as outside the buffer; the others as an index error
  function writeOf(buf, enc, str, off, len) {
    if (typeof str !== 'string') { var te = new TypeError('argument must be a string'); te.code = 'ERR_INVALID_ARG_TYPE'; throw te; }
    var strict = enc === 'utf8' || enc === 'latin1' || enc === 'ascii';
    var o = toIndex(off, 0);
    if (o < 0) throw strict ? bufBounds('offset') : oor();
    if (o > buf.length) throw bufBounds('offset');
    var max = buf.length - o;
    if (len !== undefined) {
      var l = toIndex(len, 0);
      if (l < 0) throw strict ? bufBounds('length') : oor();
      if (l < max) max = l;
    }
    var bytes = fromString(str, enc), n = Math.min(bytes.length, max);
    // never split a UTF-8 sequence, never split a UTF-16 unit
    if (enc === 'utf8') while (n > 0 && n < bytes.length && (bytes[n] & 0xc0) === 0x80) n--;
    if (enc === 'utf16le') n -= n % 2;
    for (var i = 0; i < n; i++) buf[o + i] = bytes[i];
    return n;
  }
  // *Slice are method-shaped (no prototype) with length 0, as in Node
  var SLICE = {
    utf8Slice(...a) { return sliceOf(this, 'utf8', a); },
    latin1Slice(...a) { return sliceOf(this, 'latin1', a); },
    asciiSlice(...a) { return sliceOf(this, 'ascii', a); },
    hexSlice(...a) { return sliceOf(this, 'hex', a); },
    base64Slice(...a) { return sliceOf(this, 'base64', a); },
    base64urlSlice(...a) { return sliceOf(this, 'base64url', a); },
    ucs2Slice(...a) { return sliceOf(this, 'utf16le', a); }
  };
  for (var sk in SLICE) def(P, sk, SLICE[sk]);
  var WRITE0 = {
    hexWrite(...a) { return writeOf(this, 'hex', a[0], a[1], a[2]); },
    base64Write(...a) { return writeOf(this, 'base64', a[0], a[1], a[2]); },
    base64urlWrite(...a) { return writeOf(this, 'base64url', a[0], a[1], a[2]); },
    ucs2Write(...a) { return writeOf(this, 'utf16le', a[0], a[1], a[2]); }
  };
  for (var wk in WRITE0) def(P, wk, WRITE0[wk]);
  // these three are plain functions (they have a prototype), length 1
  def(P, 'utf8Write', function utf8Write(string, ...r) { return writeOf(this, 'utf8', string, r[0], r[1]); });
  def(P, 'latin1Write', function latin1Write(string, ...r) { return writeOf(this, 'latin1', string, r[0], r[1]); });
  def(P, 'asciiWrite', function asciiWrite(string, ...r) { return writeOf(this, 'ascii', string, r[0], r[1]); });
  // validateInteger(value, name, 0, buf.length), as lib/buffer.js write() does for offset and length
  function validateOffset(v, name, max) {
    validateNumber(v, name);
    if (!Number.isInteger(v)) throw outOfRange(name, 'an integer', v);
    if (v < 0 || v > max) throw outOfRange(name, '>= 0 && <= ' + max, v);
  }
  def(P, 'write', function write(string, offset, length, encoding) {
    if (offset === undefined) { offset = 0; length = this.length; }
    else if (length === undefined && typeof offset === 'string') { encoding = offset; length = this.length; offset = 0; }
    else {
      validateOffset(offset, 'offset', this.length);
      var remaining = this.length - offset;
      if (length === undefined) length = remaining;
      else if (typeof length === 'string') { encoding = length; length = remaining; }
      else { validateOffset(length, 'length', this.length); if (length > remaining) length = remaining; }
    }
    return writeOf(this, normEnc(encoding || 'utf8'), string, offset, length);
  });

  def(P, 'subarray', function subarray(start, end) { return U8.prototype.subarray.call(this, start, end); });
  def(P, 'toLocaleString', P.toString);
  def(P, 'inspect', function inspect(recurseTimes, ctx) {
    var n = Math.min(50, this.length), rem = this.length - 50, s = '';
    for (var i = 0; i < n; i++) s += (i ? ' ' : '') + (this[i] < 16 ? '0' : '') + this[i].toString(16);
    if (rem > 0) s += ' ... ' + rem + ' more byte' + (rem > 1 ? 's' : '');
    return '<' + this.constructor.name + ' ' + s + '>';
  });
  // accessors: Node's getter halves are named `get` and have no prototype (method shorthand)
  Object.defineProperty(P, 'parent', { get: { get() { return this.buffer; } }.get, enumerable: true, configurable: false });
  Object.defineProperty(P, 'offset', { get: { get() { return this.byteOffset; } }.get, enumerable: true, configurable: false });

  globalThis.Buffer = Buffer;

  // Deprecated Node global: an unpooled Buffer of `size`. Shares Buffer.prototype so
  // `require('buffer').SlowBuffer.prototype.equal` resolves (buffer-equal-constant-time loads it).
  function SlowBuffer(size) { return Buffer.alloc(size); }
  SlowBuffer.prototype = Buffer.prototype;
  globalThis.SlowBuffer = SlowBuffer;
})();
"#;
