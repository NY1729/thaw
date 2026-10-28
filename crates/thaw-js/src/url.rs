//! WHATWG `URL` and `URLSearchParams` (special schemes, relative resolution, percent-encoding, punycode).
//! ponytail: no full IPv6 canonicalisation (lowercased as written), no IDNA mapping table beyond
//! lowercasing, no `file:` drive-letter quirks.

use crate::*;

impl Interp {
    pub(crate) fn install_url(&mut self) {
        self.run(URL_PRELUDE)
            .unwrap_or_else(|_| panic!("thaw-js URL prelude must evaluate"));
    }
}

const URL_PRELUDE: &str = r#"
(function () {
  function def(o, name, f) {
    Object.defineProperty(o, name, { value: f, writable: true, enumerable: false, configurable: true });
  }
  function getter(o, name, f, set) {
    Object.defineProperty(o, name, { get: f, set: set, enumerable: true, configurable: true });
  }
  var enc8 = __thaw_utf8_encode, dec8 = __thaw_utf8_decode;
  var HEX = '0123456789ABCDEF';
  function pct(str, keep) {
    var bytes = enc8(str), out = '';
    for (var i = 0; i < bytes.length; i++) {
      var b = bytes[i], c = String.fromCharCode(b);
      out += (b < 128 && keep(b)) ? c : '%' + HEX[b >> 4] + HEX[b & 15];
    }
    return out;
  }
  function pctDecode(s) {
    var out = [], i = 0;
    var bytes = enc8(s);
    for (i = 0; i < bytes.length; i++) {
      if (bytes[i] === 37 && i + 2 < bytes.length + 0 && /^[0-9a-fA-F]{2}$/.test(String.fromCharCode(bytes[i + 1], bytes[i + 2]))) {
        out.push(parseInt(String.fromCharCode(bytes[i + 1], bytes[i + 2]), 16)); i += 2;
      } else out.push(bytes[i]);
    }
    return dec8(new Uint8Array(out), 0, out.length);
  }
  var C0 = function (b) { return b > 0x20 && b < 0x7f; };
  var QUERY = function (b) { return C0(b) && b !== 0x22 && b !== 0x23 && b !== 0x3c && b !== 0x3e; };
  var SQUERY = function (b) { return QUERY(b) && b !== 0x27; };
  var FRAG = function (b) { return C0(b) && b !== 0x22 && b !== 0x3c && b !== 0x3e && b !== 0x60; };
  var PATH = function (b) { return QUERY(b) && b !== 0x3f && b !== 0x60 && b !== 0x7b && b !== 0x7d; };
  var USER = function (b) { return PATH(b) && b !== 0x2f && b !== 0x3a && b !== 0x3b && b !== 0x3d && b !== 0x40 && b !== 0x5b && b !== 0x5c && b !== 0x5d && b !== 0x5e && b !== 0x7c; };
  var FORM = function (b) { return (b >= 48 && b <= 57) || (b >= 65 && b <= 90) || (b >= 97 && b <= 122) || b === 42 || b === 45 || b === 46 || b === 95; };
  var SPECIAL = { 'http:': '80', 'https:': '443', 'ws:': '80', 'wss:': '443', 'ftp:': '21', 'file:': '' };

  // punycode (RFC 3492) encoder for one label
  function punyEncode(input) {
    var cps = [], i;
    for (i = 0; i < input.length; i++) cps.push(input.codePointAt(i)), input.codePointAt(i) > 0xffff && i++;
    var out = '', basic = 0;
    for (i = 0; i < cps.length; i++) if (cps[i] < 128) { out += String.fromCharCode(cps[i]); basic++; }
    var h = basic, n = 128, delta = 0, bias = 72;
    if (basic) out += '-';
    function adapt(d, num, first) {
      d = first ? Math.floor(d / 700) : d >> 1; d += Math.floor(d / num);
      var k = 0; while (d > 455) { d = Math.floor(d / 35); k += 36; }
      return k + Math.floor(36 * d / (d + 38));
    }
    function digit(d) { return String.fromCharCode(d < 26 ? d + 97 : d + 22); }
    while (h < cps.length) {
      var m = 0x7fffffff;
      for (i = 0; i < cps.length; i++) if (cps[i] >= n && cps[i] < m) m = cps[i];
      delta += (m - n) * (h + 1); n = m;
      for (i = 0; i < cps.length; i++) {
        if (cps[i] < n) delta++;
        if (cps[i] === n) {
          var q = delta;
          for (var k = 36; ; k += 36) {
            var t = k <= bias ? 1 : (k >= bias + 26 ? 26 : k - bias);
            if (q < t) break;
            out += digit(t + (q - t) % (36 - t)); q = Math.floor((q - t) / (36 - t));
          }
          out += digit(q); bias = adapt(delta, h + 1, h === basic); delta = 0; h++;
        }
      }
      delta++; n++;
    }
    return out;
  }
  function toASCII(host) {
    return host.split('.').map(function (label) {
      return /[^\x00-\x7f]/.test(label) ? 'xn--' + punyEncode(label.toLowerCase()) : label.toLowerCase();
    }).join('.');
  }
  function invalid(input) {
    var e = new TypeError('Invalid URL');
    e.code = 'ERR_INVALID_URL'; e.input = input;
    return e;
  }
  function parseIPv4(host) {
    var parts = host.split('.');
    if (parts[parts.length - 1] === '' && parts.length > 1) parts.pop();
    if (parts.length > 4 || parts.length === 0) return null;
    var nums = [];
    for (var i = 0; i < parts.length; i++) {
      var p = parts[i], v;
      if (p === '') return null;
      if (/^0[xX][0-9a-fA-F]*$/.test(p)) v = p.length === 2 ? 0 : parseInt(p.slice(2), 16);
      else if (/^0[0-7]+$/.test(p)) v = parseInt(p.slice(1), 8);
      else if (/^[0-9]+$/.test(p)) v = parseInt(p, 10);
      else return null;
      nums.push(v);
    }
    for (i = 0; i < nums.length - 1; i++) if (nums[i] > 255) throw 'bad';
    if (nums[nums.length - 1] >= Math.pow(256, 5 - nums.length)) throw 'bad';
    var ip = nums[nums.length - 1];
    for (i = 0; i < nums.length - 1; i++) ip += nums[i] * Math.pow(256, 3 - i);
    return [Math.floor(ip / 16777216) % 256, Math.floor(ip / 65536) % 256, Math.floor(ip / 256) % 256, ip % 256].join('.');
  }
  function parseHost(raw, special) {
    if (raw.charAt(0) === '[') {
      if (raw.charAt(raw.length - 1) !== ']' || !/^\[[0-9a-fA-F:.]+\]$/.test(raw)) throw 'bad';
      return raw.toLowerCase();
    }
    if (!special) {
      if (/[\x00\t\n\r #\/:<>?@\[\\\]^|]/.test(raw)) throw 'bad';
      return pct(raw, C0);
    }
    var host = toASCII(pctDecode(raw));
    if (host === '' || /[\x00-\x20#%\/:<>?@\[\\\]^|\x7f]/.test(host)) throw 'bad';
    var last = host.split('.').filter(function (x, i, a) { return !(i === a.length - 1 && x === '' && a.length > 1); }).pop();
    if (/^(?:[0-9]+|0[xX][0-9a-fA-F]*)$/.test(last)) { var ip = parseIPv4(host); if (ip === null) throw 'bad'; return ip; }
    return host;
  }
  function Rec() {
    this.scheme = ''; this.username = ''; this.password = ''; this.host = null; this.port = '';
    this.path = []; this.opaque = null; this.query = null; this.fragment = null;
  }
  function isSpecial(rec) { return Object.prototype.hasOwnProperty.call(SPECIAL, rec.scheme + ':'); }
  function normPath(segs, special) {
    var out = [];
    for (var i = 0; i < segs.length; i++) {
      var s = segs[i], low = s.toLowerCase();
      if (low === '..' || low === '.%2e' || low === '%2e.' || low === '%2e%2e') { if (out.length) out.pop(); if (i === segs.length - 1) out.push(''); }
      else if (low === '.' || low === '%2e') { if (i === segs.length - 1) out.push(''); }
      else out.push(s);
    }
    return out;
  }
  function parseInto(rec, input, base) {
    input = String(input).replace(/^[\x00-\x20]+|[\x00-\x20]+$/g, '').replace(/[\t\n\r]/g, '');
    var m = /^([a-zA-Z][a-zA-Z0-9+.\-]*):/.exec(input), rest, special;
    if (m) {
      rec.scheme = m[1].toLowerCase(); rest = input.slice(m[0].length); special = isSpecial(rec);
      if (special && base && base.scheme === rec.scheme && !/^[\/\\]/.test(rest) && rec.scheme !== 'file') {
        return relative(rec, rest, base);
      }
    } else {
      if (!base) throw invalid(input);
      if (base.opaque !== null && input.charAt(0) !== '#') throw invalid(input);
      rec.scheme = base.scheme;
      return relative(rec, input, base);
    }
    special = isSpecial(rec);
    var frag = null, query = null, h = rest.indexOf('#');
    if (h >= 0) { frag = rest.slice(h + 1); rest = rest.slice(0, h); }
    var q = rest.indexOf('?');
    if (q >= 0) { query = rest.slice(q + 1); rest = rest.slice(0, q); }
    var slashes = special ? /^[\/\\]*/.exec(rest)[0].length : (rest.slice(0, 2) === '//' ? 2 : 0);
    if (rec.scheme === 'file') {
      if (!/^[\/\\]{2}/.test(rest)) { rec.host = ''; setPath(rec, rest, true); }
      else { authority(rec, rest.slice(2), special); }
    } else if (special || slashes >= 2) {
      authority(rec, rest.slice(special ? slashes : 2), special);
    } else if (rest.charAt(0) === '/') {
      setPath(rec, rest, special);
    } else {
      rec.opaque = pct(rest, function (b) { return b >= 0x20 && b < 0x7f; });
    }
    if (query !== null) rec.query = pct(query, special ? SQUERY : QUERY);
    if (frag !== null) rec.fragment = pct(frag, FRAG);
    return rec;
  }
  function authority(rec, s, special) {
    var end = s.search(special ? /[\/\\]/ : /\//);
    var auth = end < 0 ? s : s.slice(0, end), path = end < 0 ? '' : s.slice(end);
    var at = auth.lastIndexOf('@');
    if (at >= 0) {
      var ui = auth.slice(0, at), colon = ui.indexOf(':');
      rec.username = pct(colon < 0 ? ui : ui.slice(0, colon), USER);
      rec.password = colon < 0 ? '' : pct(ui.slice(colon + 1), USER);
      auth = auth.slice(at + 1);
    }
    var hostPart = auth, port = '';
    var pm = /:([0-9]*)$/.exec(auth);
    if (pm && !(auth.charAt(0) === '[' && auth.indexOf(']') > auth.length - 2 - pm[1].length && auth.charAt(auth.length - 1) === ']')) { hostPart = auth.slice(0, pm.index); port = pm[1]; }
    else if (auth.charAt(0) !== '[' && auth.indexOf(':') >= 0) throw invalid(auth);
    if (hostPart === '' && (special && rec.scheme !== 'file')) throw invalid(auth);
    try { rec.host = hostPart === '' ? '' : parseHost(hostPart, special); } catch (e) { if (e && e.code) throw e; throw invalid(auth); }
    if (rec.scheme === 'file' && rec.host === 'localhost') rec.host = '';
    if (port !== '') {
      var pn = parseInt(port, 10);
      if (pn > 65535) throw invalid(auth);
      rec.port = String(pn) === SPECIAL[rec.scheme + ':'] ? '' : String(pn);
    }
    setPath(rec, path, special);
  }
  function setPath(rec, path, special) {
    if (path === '' && special) { rec.path = ['']; return; }
    if (path === '') { rec.path = []; return; }
    var segs = (special ? path.replace(/\\/g, '/') : path).split('/');
    segs.shift();
    rec.path = normPath(segs.map(function (s) { return pct(s, PATH); }), special);
  }
  function relative(rec, input, base) {
    var special = isSpecial(rec);
    var frag = null, query = null, h = input.indexOf('#');
    if (h >= 0) { frag = input.slice(h + 1); input = input.slice(0, h); }
    var q = input.indexOf('?');
    if (q >= 0) { query = input.slice(q + 1); input = input.slice(0, q); }
    if (special ? /^[\/\\]{2}/.test(input) : input.slice(0, 2) === '//') {
      authority(rec, input.slice(2), special);
      if (query !== null) rec.query = pct(query, special ? SQUERY : QUERY);
    } else {
      rec.username = base.username; rec.password = base.password; rec.host = base.host; rec.port = base.port;
      if (input === '') {
        rec.path = base.path.slice(); rec.opaque = base.opaque;
        rec.query = query !== null ? pct(query, special ? SQUERY : QUERY) : base.query;
      } else if (input.charAt(0) === '/' || (special && input.charAt(0) === '\\')) {
        setPath(rec, input, special);
        if (query !== null) rec.query = pct(query, special ? SQUERY : QUERY);
      } else {
        var segs = base.path.slice(0, -1).concat((special ? input.replace(/\\/g, '/') : input).split('/'));
        rec.path = normPath(segs.map(function (s) { return pct(s, PATH); }), special);
        if (query !== null) rec.query = pct(query, special ? SQUERY : QUERY);
      }
    }
    if (frag !== null) rec.fragment = pct(frag, FRAG);
    return rec;
  }
  function serialize(rec, noFragment) {
    var s = rec.scheme + ':';
    if (rec.host !== null) {
      s += '//';
      if (rec.username !== '' || rec.password !== '') { s += rec.username; if (rec.password !== '') s += ':' + rec.password; s += '@'; }
      s += rec.host; if (rec.port !== '') s += ':' + rec.port;
    }
    s += rec.opaque !== null ? rec.opaque : (rec.path.length ? '/' + rec.path.join('/') : '');
    if (rec.query !== null) s += '?' + rec.query;
    if (!noFragment && rec.fragment !== null) s += '#' + rec.fragment;
    return s;
  }

  // ----- URLSearchParams -----
  var PARAMS = '@@params';
  function USP(init) {
    var list = [];
    Object.defineProperty(this, '@@list', { value: list, enumerable: false, writable: true });
    Object.defineProperty(this, '@@url', { value: null, enumerable: false, writable: true });
    if (init === undefined || init === null) return;
    if (typeof init === 'string') parseQuery(list, init);
    else if (init instanceof USP) init['@@list'].forEach(function (p) { list.push([p[0], p[1]]); });
    else if (typeof init === 'object' || typeof init === 'function') {
      if (typeof init[Symbol.iterator] === 'function' || Array.isArray(init)) {
        for (var pair of init) {
          var p = Array.from(pair);
          if (p.length !== 2) { var e = new TypeError('Each query pair must be an iterable [name, value] tuple'); e.code = 'ERR_INVALID_TUPLE'; throw e; }
          list.push([String(p[0]), String(p[1])]);
        }
      } else Object.keys(init).forEach(function (k) { list.push([k, String(init[k])]); });
    } else parseQuery(list, String(init));
  }
  function parseQuery(list, s) {
    if (s.charAt(0) === '?') s = s.slice(1);
    if (s === '') return;
    s.split('&').forEach(function (part) {
      if (part === '') return;
      var eq = part.indexOf('='), k = eq < 0 ? part : part.slice(0, eq), v = eq < 0 ? '' : part.slice(eq + 1);
      list.push([pctDecode(k.replace(/\+/g, ' ')), pctDecode(v.replace(/\+/g, ' '))]);
    });
  }
  function formEncode(s) { return pct(s, FORM).replace(/%20/g, '+'); }
  function update(sp) {
    var u = sp['@@url'];
    if (u) { var s = sp.toString(); u['@@rec'].query = s === '' ? null : s; }
  }
  var P = USP.prototype;
  Object.defineProperty(P, Symbol.toStringTag, { value: 'URLSearchParams', configurable: true });
  function need(n, args) {
    if (args.length < n) { var e = new TypeError('The "' + (n === 2 ? 'name" and "value" arguments' : 'name" argument') + ' must be specified'); e.code = 'ERR_MISSING_ARGS'; throw e; }
  }
  def(P, 'append', function (k, v) { need(2, arguments); this['@@list'].push([String(k), String(v)]); update(this); });
  def(P, 'delete', function (k, v) {
    need(1, arguments);
    k = String(k);
    this['@@list'] = this['@@list'].filter(function (p) { return !(p[0] === k && (v === undefined || p[1] === String(v))); });
    update(this);
  });
  def(P, 'get', function (k) { need(1, arguments); k = String(k); var l = this['@@list']; for (var i = 0; i < l.length; i++) if (l[i][0] === k) return l[i][1]; return null; });
  def(P, 'getAll', function (k) { need(1, arguments); k = String(k); return this['@@list'].filter(function (p) { return p[0] === k; }).map(function (p) { return p[1]; }); });
  def(P, 'has', function (k, v) { need(1, arguments); k = String(k); return this['@@list'].some(function (p) { return p[0] === k && (v === undefined || p[1] === String(v)); }); });
  def(P, 'set', function (k, v) {
    need(2, arguments);
    k = String(k); v = String(v);
    var l = this['@@list'], found = false, out = [];
    for (var i = 0; i < l.length; i++) {
      if (l[i][0] === k) { if (!found) { out.push([k, v]); found = true; } } else out.push(l[i]);
    }
    if (!found) out.push([k, v]);
    this['@@list'] = out; update(this);
  });
  def(P, 'sort', function () {
    var l = this['@@list'];
    // stable by UTF-16 code units of the name
    var idx = l.map(function (p, i) { return [p, i]; });
    idx.sort(function (a, b) { return a[0][0] < b[0][0] ? -1 : a[0][0] > b[0][0] ? 1 : a[1] - b[1]; });
    this['@@list'] = idx.map(function (x) { return x[0]; }); update(this);
  });
  def(P, 'toString', function () { return this['@@list'].map(function (p) { return formEncode(p[0]) + '=' + formEncode(p[1]); }).join('&'); });
  def(P, 'forEach', function (f, t) {
    if (typeof f !== 'function') { var e = new TypeError('The "callback" argument must be of type function. Received ' + (f === undefined ? 'undefined' : typeof f)); e.code = 'ERR_INVALID_ARG_TYPE'; throw e; }
    var l = this['@@list'];
    for (var i = 0; i < l.length; i++) f.call(t, l[i][1], l[i][0], this);
  });
  // live iterator over the list (WHATWG: sees appends made while iterating)
  var ITP = {};
  def(ITP, 'next', function () {
    var l = this['@@l'];
    if (this['@@i'] >= l.length) return { value: undefined, done: true };
    var p = l[this['@@i']++], k = this['@@k'];
    return { value: k === 0 ? p[0] : k === 1 ? p[1] : [p[0], p[1]], done: false };
  });
  ITP[Symbol.iterator] = function () { return this; };
  Object.defineProperty(ITP, Symbol.toStringTag, { value: 'URLSearchParams Iterator', configurable: true });
  function iter(list, kind) {
    var it = Object.create(ITP);
    Object.defineProperty(it, '@@l', { value: list }); Object.defineProperty(it, '@@k', { value: kind });
    Object.defineProperty(it, '@@i', { value: 0, writable: true });
    return it;
  }
  def(P, 'keys', function () { return iter(this['@@list'], 0); });
  def(P, 'values', function () { return iter(this['@@list'], 1); });
  def(P, 'entries', function () { return iter(this['@@list'], 2); });
  Object.defineProperty(P, 'size', { get: function () { return this['@@list'].length; }, configurable: true });
  P[Symbol.iterator] = P.entries;

  // ----- URL -----
  function URLImpl(input, base) {
    if (arguments.length === 0) { var e = new TypeError('The "url" argument must be specified'); e.code = 'ERR_MISSING_ARGS'; throw e; }
    var baseRec = null;
    if (base !== undefined) baseRec = parseInto(new Rec(), String(base), null);
    var rec = parseInto(new Rec(), String(input), baseRec);
    Object.defineProperty(this, '@@rec', { value: rec, enumerable: false, writable: true });
    var sp = new USP(rec.query || '');
    sp['@@url'] = this;
    Object.defineProperty(this, '@@sp', { value: sp, enumerable: false, writable: true });
  }
  var U = URLImpl.prototype;
  function R(u) { return u['@@rec']; }
  function resync(u) { var sp = u['@@sp']; sp['@@list'] = []; parseQuery(sp['@@list'], R(u).query || ''); }
  getter(U, 'href', function () { return serialize(R(this)); }, function (v) { var r = parseInto(new Rec(), String(v), null); this['@@rec'] = r; resync(this); });
  getter(U, 'origin', function () {
    var r = R(this);
    if (r.scheme === 'blob') { try { return new URLImpl(r.opaque !== null ? r.opaque : r.path.join('/')).origin; } catch (e) { return 'null'; } }
    return isSpecial(r) && r.scheme !== 'file' ? r.scheme + '://' + r.host + (r.port ? ':' + r.port : '') : 'null';
  });
  getter(U, 'protocol', function () { return R(this).scheme + ':'; }, function (v) {
    var s = String(v).replace(/:.*$/, '').toLowerCase();
    if (/^[a-z][a-z0-9+.\-]*$/.test(s) && (Object.prototype.hasOwnProperty.call(SPECIAL, s + ':') === isSpecial(R(this)))) {
      R(this).scheme = s;
      if (R(this).port === SPECIAL[s + ':']) R(this).port = '';
    }
  });
  getter(U, 'username', function () { return R(this).username; }, function (v) { if (R(this).host) R(this).username = pct(String(v), USER); });
  getter(U, 'password', function () { return R(this).password; }, function (v) { if (R(this).host) R(this).password = pct(String(v), USER); });
  getter(U, 'host', function () { var r = R(this); return r.host === null ? '' : r.host + (r.port ? ':' + r.port : ''); }, function (v) {
    var r = R(this); if (r.opaque !== null) return;
    var m = /^([^:\/?#]*|\[[^\]]*\])(?::([0-9]*))?/.exec(String(v));
    try { if (m[1] === '') return; r.host = parseHost(m[1], isSpecial(r)); } catch (e) { return; }
    if (m[2]) r.port = String(parseInt(m[2], 10)) === SPECIAL[r.scheme + ':'] ? '' : String(parseInt(m[2], 10));
  });
  getter(U, 'hostname', function () { var r = R(this); return r.host === null ? '' : r.host; }, function (v) {
    var r = R(this); if (r.opaque !== null) return;
    try { var h = String(v).replace(/[:\/?#].*$/, ''); if (h === '') return; r.host = parseHost(h, isSpecial(r)); } catch (e) {}
  });
  getter(U, 'port', function () { return R(this).port; }, function (v) {
    var r = R(this); if (r.host === null || r.host === '' || r.scheme === 'file') return;
    v = String(v);
    if (v === '') { r.port = ''; return; }
    var m = /^[0-9]+/.exec(v); if (!m) return;
    var n = parseInt(m[0], 10); if (n > 65535) return;
    r.port = String(n) === SPECIAL[r.scheme + ':'] ? '' : String(n);
  });
  getter(U, 'pathname', function () { var r = R(this); return r.opaque !== null ? r.opaque : (r.path.length ? '/' + r.path.join('/') : ''); }, function (v) {
    var r = R(this); if (r.opaque !== null) return;
    var s = String(v); setPath(r, s.charAt(0) === '/' || (isSpecial(r) && s.charAt(0) === '\\') ? s : '/' + s, isSpecial(r));
  });
  getter(U, 'search', function () { var q = R(this).query; return q === null || q === '' ? '' : '?' + q; }, function (v) {
    v = String(v); var r = R(this);
    if (v === '') r.query = null; else r.query = pct(v.charAt(0) === '?' ? v.slice(1) : v, isSpecial(r) ? SQUERY : QUERY);
    resync(this);
  });
  getter(U, 'hash', function () { var f = R(this).fragment; return f === null || f === '' ? '' : '#' + f; }, function (v) {
    v = String(v); R(this).fragment = v === '' ? null : pct(v.charAt(0) === '#' ? v.slice(1) : v, FRAG);
  });
  Object.defineProperty(U, 'searchParams', { get: function () { return this['@@sp']; }, enumerable: true, configurable: true });
  def(U, 'toString', function () { return this.href; });
  def(U, 'toJSON', function () { return this.href; });
  def(URLImpl, 'canParse', function (u, b) { try { new URLImpl(u, b); return true; } catch (e) { return false; } });
  def(URLImpl, 'parse', function (u, b) { try { return new URLImpl(u, b); } catch (e) { return null; } });
  globalThis.URL = URLImpl;
  globalThis.URLSearchParams = USP;
})();
"#;
