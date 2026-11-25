//! Differential tests: the native UTF-8 / base64 / hex / atob / btoa paths against the previous pure-JS loops
//! (kept in `fixtures/text_codec_oracle.js` and inline below), plus WHATWG vectors.
use super::*;

const ORACLE: &str = include_str!("fixtures/text_codec_oracle.js");

fn run_ok(src: &str) -> String {
    let mut it = Interp::new();
    if let Err(e) = it.run(src) {
        let msg = match e {
            Ctl::Throw(v) => it.to_str(&v).to_string(),
            _ => "non-throw completion".into(),
        };
        panic!("script failed: {msg}");
    }
    it.out.join("\n")
}

fn with_oracle(body: &str) -> String {
    format!(
        "var O = (function (globalThis) {{\n{ORACLE}\nreturn globalThis;}})({{}});\n\
         var seed = 12345;\n\
         function rnd(n) {{ seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed % n; }}\n\
         var HOT = [0,0x41,0x7f,0x80,0x8f,0x90,0x9f,0xa0,0xbf,0xc0,0xc1,0xc2,0xdf,0xe0,0xe1,0xec,0xed,0xee,0xef,0xf0,0xf1,0xf3,0xf4,0xf5,0xf8,0xff];\n\
         var SEQ = [[0xc3,0xa9],[0xe2,0x82,0xac],[0xf0,0x9f,0x98,0x80],[0xef,0xbb,0xbf],[0xed,0x9f,0xbf],[0xf4,0x8f,0xbf,0xbf],[0xed,0xa0,0x80],[0xe0,0x80,0x80],[0xf0,0x80,0x80,0x80]];\n\
         function rbytes() {{\n\
           var out = [], n = rnd(14);\n\
           while (out.length < n) {{\n\
             var k = rnd(10);\n\
             if (k < 4) out.push(HOT[rnd(HOT.length)]);\n\
             else if (k < 5) out.push(rnd(256));\n\
             else {{ var q = SEQ[rnd(SEQ.length)]; for (var i = 0; i < q.length; i++) out.push(q[i]); if (rnd(4) === 0) out.length -= 1 + rnd(2); }}\n\
           }}\n\
           return new Uint8Array(out);\n\
         }}\n\
         function res(f) {{ try {{ return 'ok:' + f(); }} catch (e) {{ return 'err:' + e.name + ':' + e.code; }} }}\n\
         var bad = [];\n\
         function same(label, a, b) {{ if (a !== b) bad.push(label + ' ' + JSON.stringify(a) + ' vs ' + JSON.stringify(b)); }}\n\
         {body}\n\
         console.log(bad.length ? bad.slice(0, 5).join('\\n') : 'ok');"
    )
}

#[test]
fn utf8_decode_matches_js_oracle() {
    let out = run_ok(&with_oracle(
        r#"
for (var iter = 0; iter < 700; iter++) {
  var b = rbytes();
  var tag = Array.from(b).join(',');
  for (var opts of [{}, { fatal: true }, { ignoreBOM: true }, { fatal: true, ignoreBOM: true }]) {
    same('decode ' + tag + JSON.stringify(opts),
      res(() => new TextDecoder('utf-8', opts).decode(b)), res(() => new O.TextDecoder('utf-8', opts).decode(b)));
  }
  // streaming: split at random points, then flush
  var cuts = [0, rnd(b.length + 1), rnd(b.length + 1)].sort((x, y) => x - y);
  var fatal = rnd(3) === 0, ig = rnd(3) === 0;
  var run = D => res(() => {
    var d = new D('utf-8', { fatal: fatal, ignoreBOM: ig }), s = '';
    s += d.decode(b.subarray(cuts[0], cuts[1]), { stream: true });
    s += d.decode(b.subarray(cuts[1], cuts[2]), { stream: true });
    s += d.decode(b.subarray(cuts[2]), { stream: true });
    s += d.decode();
    // the decoder is reusable after a flush
    s += '|' + d.decode(b);
    return s;
  });
  same('stream ' + tag + ' ' + cuts + fatal + ig, run(TextDecoder), run(O.TextDecoder));
}
"#,
    ));
    assert_eq!(out, "ok");
}

#[test]
fn utf8_encode_matches_js_oracle() {
    let out = run_ok(&with_oracle(
        r#"
var PIECES = ['a', 'é', '€', '\u{1F600}', '﻿', '\0', String.fromCharCode(0xd800), String.fromCharCode(0xdc00),
  String.fromCharCode(0xd83d), String.fromCharCode(0xde00), '\u{10FFFF}', '\u{10F800}'];
for (var iter = 0; iter < 3000; iter++) {
  var s = '', n = rnd(8);
  for (var i = 0; i < n; i++) s += PIECES[rnd(PIECES.length)];
  same('encode ' + JSON.stringify(s), Array.from(new TextEncoder().encode(s)).join(), Array.from(new O.TextEncoder().encode(s)).join());
  var cap = rnd(20);
  var a = new Uint8Array(cap), b = new Uint8Array(cap);
  var ra = new TextEncoder().encodeInto(s, a), rb = new O.TextEncoder().encodeInto(s, b);
  same('into ' + JSON.stringify(s) + cap, ra.read + '/' + ra.written + ':' + Array.from(a).join(), rb.read + '/' + rb.written + ':' + Array.from(b).join());
}
"#,
    ));
    assert_eq!(out, "ok");
}

#[test]
fn whatwg_vectors() {
    let out = run_ok(
        r#"
var d = new TextDecoder();
var h = b => (t => { var o = []; for (var i = 0; i < t.length; i++) o.push(t.charCodeAt(i).toString(16)); return o.join(' '); })(d.decode(new Uint8Array(b)));
console.log(h([0xf0, 0x9f, 0x98, 0x80]));             // d83d de00
console.log(h([0xc0, 0x80]));                          // fffd fffd
console.log(h([0xe0, 0x80, 0x80]));                    // fffd fffd fffd
console.log(h([0xed, 0xa0, 0x80]));                    // fffd fffd fffd
console.log(h([0xf0, 0x9f, 0x98]));                    // fffd
console.log(h([0xf4, 0x90, 0x80, 0x80]));              // fffd fffd fffd fffd
console.log(h([0xef, 0xbb, 0xbf, 0x41]));              // 41 (BOM stripped)
console.log(Array.from(new TextEncoder().encode('€')).join());        // 226,130,172
var s = new TextDecoder();
console.log(JSON.stringify([s.decode(new Uint8Array([0xe2, 0x82]), { stream: true }), s.decode(new Uint8Array([0xac]))]));
try { new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array([0xff])); } catch (e) { console.log(e.code); }
console.log(atob('aGk='), btoa('hi'), JSON.stringify(Buffer.from('aGk', 'base64').toString()), Buffer.from('hi').toString('hex'));
"#,
    );
    assert_eq!(
        out,
        "d83d de00\nfffd fffd\nfffd fffd fffd\nfffd fffd fffd\nfffd\nfffd fffd fffd fffd\n41\n226,130,172\n[\"\",\"\u{20ac}\"]\nERR_ENCODING_INVALID_ENCODED_DATA\nhi aGk= \"hi\" 6869"
    );
}

#[test]
fn base64_hex_latin1_match_js_loops() {
    let out = run_ok(&with_oracle(
        r#"
var B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
var B64URL = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
function oEnc(b, T, pad) {
  var s = '';
  for (var i = 0; i < b.length; i += 3) {
    var x = b[i], y = i + 1 < b.length ? b[i + 1] : 0, z = i + 2 < b.length ? b[i + 2] : 0, n = (x << 16) | (y << 8) | z;
    s += T[(n >> 18) & 63] + T[(n >> 12) & 63] + (i + 1 < b.length ? T[(n >> 6) & 63] : pad) + (i + 2 < b.length ? T[n & 63] : pad);
  }
  return s;
}
function oDec(s) {
  var out = [], acc = 0, bits = 0;
  for (var i = 0; i < s.length; i++) {
    var c = s[i], v = B64.indexOf(c);
    if (v < 0) v = B64URL.indexOf(c);
    if (c === '=') break;
    if (v < 0) continue;
    acc = (acc << 6) | v; bits += 6;
    if (bits >= 8) { bits -= 8; out.push((acc >> bits) & 255); }
  }
  return out.join();
}
function oHex(b) { var s = ''; for (var i = 0; i < b.length; i++) s += (b[i] < 16 ? '0' : '') + b[i].toString(16); return s; }
function oAtob(s) {
  s = String(s).replace(/[\t\n\f\r ]/g, '');
  if (s.length % 4 === 0) s = s.replace(/==?$/, '');
  if (s.length % 4 === 1 || /[^A-Za-z0-9+\/]/.test(s)) throw new Error('bad');
  var out = '', bits = 0, acc = 0;
  for (var i = 0; i < s.length; i++) { acc = (acc << 6) | B64.indexOf(s[i]); bits += 6; if (bits >= 8) { bits -= 8; out += String.fromCharCode((acc >> bits) & 255); } }
  return out;
}
var CH = 'AZaz09+/-_= \n\t*!é'.split('');
for (var iter = 0; iter < 2000; iter++) {
  var raw = []; for (var i = 0, n = rnd(40); i < n; i++) raw.push(rnd(256));
  var buf = Buffer.from(raw), lo = rnd(raw.length + 1), hi = lo + rnd(raw.length - lo + 1);
  var sub = raw.slice(lo, hi);
  same('b64 ' + raw, buf.toString('base64', lo, hi), oEnc(sub, B64, '='));
  same('b64url ' + raw, buf.toString('base64url', lo, hi), oEnc(sub, B64URL, ''));
  same('hex ' + raw, buf.toString('hex', lo, hi), oHex(sub));
  same('latin1 ' + raw, buf.toString('latin1', lo, hi), String.fromCharCode.apply(null, sub));
  same('ascii ' + raw, buf.toString('ascii', lo, hi), String.fromCharCode.apply(null, sub.map(x => x & 127)));
  var t = ''; for (var j = 0, m = rnd(30); j < m; j++) t += CH[rnd(CH.length)];
  same('fromb64 ' + JSON.stringify(t), Array.from(Buffer.from(t, 'base64')).join(), oDec(t));
  var kind = f => { try { return 'ok:' + f(); } catch (e) { return 'err'; } };
  same('atob ' + JSON.stringify(t), kind(() => atob(t)), kind(() => oAtob(t)));
  var l = ''; for (var k = 0, q = rnd(10); k < q; k++) l += String.fromCharCode(rnd(300));
  same('btoa ' + JSON.stringify(l), res(() => btoa(l)).slice(0, 3), l.split('').some(c => c.charCodeAt(0) > 255) ? 'err' : 'ok:');
  if (!l.split('').some(c => c.charCodeAt(0) > 255)) same('btoa v ' + JSON.stringify(l), btoa(l), oEnc(l.split('').map(c => c.charCodeAt(0)), B64, '='));
}
"#,
    ));
    assert_eq!(out, "ok");
}
