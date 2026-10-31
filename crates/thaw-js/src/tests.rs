use super::*;

/// Run `src` and return the console.log lines (or the uncaught error text).
fn run(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out,
        Err(Ctl::Throw(v)) => {
            let mut out = it.out.clone();
            out.push(format!("Uncaught {}", it.to_str(&v)));
            out
        }
        Err(Ctl::Unsupported(f)) => vec![format!("unsupported: {f}")],
        Err(_) => vec!["stray completion".into()],
    }
}

fn one(src: &str) -> String {
    run(src).join("|")
}

#[test]
fn arithmetic_strings_and_coercion() {
    assert_eq!(
        one("console.log(1 + 2 * 3, 7 % 4, 2 ** 10, 1 / 4, 0.1 + 0.2)"),
        "7 3 1024 0.25 0.30000000000000004"
    );
    assert_eq!(
        one("console.log('a' + 1, 1 + '2', '3' * '4', true + 1, null + 1, undefined + 1)"),
        "a1 12 12 2 1 NaN"
    );
    assert_eq!(
        one("console.log(5 / 0, -5 / 0, 0 / 0, -0, 1e21, 123456789012)"),
        "Infinity -Infinity NaN -0 1e+21 123456789012"
    );
    assert_eq!(
        one("console.log(5 >> 1, -5 >>> 28, 1 << 31, 6 & 3, 6 | 3, 6 ^ 3, ~5)"),
        "2 15 -2147483648 2 7 5 -6"
    );
    assert_eq!(one("console.log(`x${1 + 1}y${'z'}`)"), "x2yz");
}

#[test]
fn number_formatting_matches_js() {
    assert_eq!(one("console.log(1e21, 1e-7, 1.5e-7, 123e20, 0.000001, 1.5e300, -1e21, String(-0), 100, 1.5)"), "1e+21 1e-7 1.5e-7 1.23e+22 0.000001 1.5e+300 -1e+21 0 100 1.5");
}

#[test]
fn equality_and_typeof() {
    assert_eq!(one("console.log(1 == '1', 1 === '1', null == undefined, null === undefined, NaN == NaN, 'a' < 'b')"), "true false true false false true");
    assert_eq!(one("console.log(typeof 1, typeof 'a', typeof undefined, typeof null, typeof {}, typeof function(){}, typeof nope)"), "number string undefined object object function undefined");
}

#[test]
fn closures_and_per_iteration_bindings() {
    assert_eq!(one("function mk(){ var n = 0; return function(){ return ++n; }; } var c = mk(); c(); c(); console.log(c(), mk()())"), "3 1");
    assert_eq!(one("var fs = []; for (let i = 0; i < 3; i++) fs.push(() => i); console.log(fs[0](), fs[1](), fs[2]())"), "0 1 2");
    assert_eq!(one("var fs = []; for (var i = 0; i < 3; i++) fs.push(() => i); console.log(fs[0](), fs[2]())"), "3 3");
}

#[test]
fn recursion_hoisting_and_default_rest_params() {
    assert_eq!(
        one("console.log(fib(15)); function fib(n){ return n < 2 ? n : fib(n-1) + fib(n-2); }"),
        "610"
    );
    assert_eq!(one("function f(a, b = 10, ...r){ return a + b + r.length; } console.log(f(1), f(1, 2), f(1, 2, 3, 4))"), "11 3 5");
    assert_eq!(
        one("console.log(h()); var h = function(){ return 1; }"),
        "Uncaught TypeError: h is not a function"
    );
    assert_eq!(
        one("var x = 1; { var x = 2; } console.log(x); let y = 1; { let y = 2; } console.log(y)"),
        "2|1"
    );
}

#[test]
fn objects_arrays_prototypes_and_new() {
    assert_eq!(one("var o = {a: 1, 'b c': [1, 2], f(){ return this.a; }}; console.log(o.a, o['b c'].length, o.f()); o.z = 5; delete o.a; console.log(Object.keys(o).join())"), "1 2 1|b c,f,z");
    assert_eq!(one("function P(n){ this.n = n; } P.prototype.get = function(){ return this.n; }; var p = new P(7); console.log(p.get(), p instanceof P, p.constructor === P, 'get' in p)"), "7 true true true");
    assert_eq!(one("var a = [3, 1, 2]; a.push(4); a[6] = 9; console.log(a.length, a.join('-'), a.indexOf(2), [1,2,3].slice(1).join(), [1].concat([2],[3,4]).length)"), "7 3-1-2-4---9 2 2,3 4");
    assert_eq!(one("console.log([1,2,3].map(x => x * 2).filter(x => x > 2).reduce((s, x) => s + x, 0), [1,2,3].reduce((s, x) => s + x))"), "10 6");
    assert_eq!(one("var x = {a: 1, b: {c: [1, 'two']}}; console.log(x); console.log([], {}, 'top', ['in'])"), "{ a: 1, b: { c: [ 1, 'two' ] } }|[] {} top [ 'in' ]");
    assert_eq!(one("var o = Object.create({inherited: 1}); o.own = 2; console.log(o.inherited, o.own, Object.keys(o).join())"), "1 2 own");
}

#[test]
fn control_flow_labels_switch_and_loops() {
    assert_eq!(one("var s = 0; outer: for (var i = 0; i < 5; i++) { for (var j = 0; j < 5; j++) { if (j == 2) continue outer; if (i == 3) break outer; s += 1; } } console.log(s, i)"), "6 3");
    assert_eq!(one("function f(x){ switch (x) { case 1: return 'one'; case 2: case 3: return 'few'; default: return 'many'; } } console.log(f(1), f(3), f(9))"), "one few many");
    assert_eq!(one("var i = 0, s = ''; do { s += i; i++; } while (i < 3); while (true) { if (++i > 5) break; } for (var k in {p: 1, q: 2}) s += k; for (var ch of 'xy') s += ch; console.log(s, i)"), "012pqxy 6");
}

#[test]
fn exceptions_and_runtime_errors() {
    assert_eq!(one("try { throw new Error('boom'); } catch (e) { console.log(e.message, e instanceof Error, String(e)); } finally { console.log('fin'); }"), "boom true Error: boom|fin");
    assert_eq!(
        one("try { null.x } catch (e) { console.log(e.name + ': ' + e.message) }"),
        "TypeError: Cannot read properties of null (reading 'x')"
    );
    assert_eq!(
        one("try { nope } catch (e) { console.log(e instanceof ReferenceError, e.message) }"),
        "true nope is not defined"
    );
    assert_eq!(
        one("const c = 1; try { c = 2 } catch (e) { console.log(e.message) }"),
        "Assignment to constant variable."
    );
    assert_eq!(
        one("function f(){ try { return 1 } finally { console.log('cleanup') } } console.log(f())"),
        "cleanup|1"
    );
    assert_eq!(one("throw new TypeError('t')"), "Uncaught TypeError: t");
}

#[test]
fn stack_overflow_is_a_range_error_not_a_crash() {
    let src = "function r(n){ return r(n + 1); } try { r(0) } catch (e) { console.log(e instanceof RangeError, e.message) }";
    // deep native recursion needs a big stack in debug test threads
    let out = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || one(src))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(out, "true Maximum call stack size exceeded");
}

#[test]
fn strings_and_math_builtins() {
    assert_eq!(one("var s = 'Hello, World'; console.log(s.length, s.charAt(1), s.charCodeAt(0), s.indexOf('World'), s.slice(-5), s.toUpperCase(), s.split(', ').length, s[0])"), "12 e 72 7 World HELLO, WORLD 2 H");
    assert_eq!(one("console.log(Math.floor(2.7), Math.max(1, 5, 3), Math.min(), Math.round(2.5), Math.pow(2, 8), parseInt('42px'), parseInt('ff', 16), isNaN('x'), Number('  12 '))"), "2 5 Infinity 3 256 42 255 true 12");
}

#[test]
fn unsupported_constructs_name_the_feature() {
    // generators, BigInt, `new.target` and private names are implemented (tests/differential/lang)
    assert_eq!(one("var m = import.meta"), "unsupported: import.meta");
    assert_eq!(one("var r = import('x')"), "unsupported: import()");
    assert_eq!(one("import x from 'y'"), "unsupported: esm-syntax");
}

#[test]
fn destructuring_spread_and_accessors() {
    assert_eq!(one("var {a, b: [c, d = 4], ...r} = {a: 1, b: [2], x: 5, y: 6}; console.log(a, c, d, Object.keys(r).join())"), "1 2 4 x,y");
    assert_eq!(
        one("var [x, , y = 9, ...z] = [1, 2, undefined, 4, 5]; console.log(x, y, z.join())"),
        "1 9 4,5"
    );
    assert_eq!(
        one("function f({a, b = 2}, [c]) { return a + b + c } console.log(f({a: 1}, [3]))"),
        "6"
    );
    assert_eq!(
        one("var a = 1, b = 2; [a, b] = [b, a]; console.log(a, b)"),
        "2 1"
    );
    assert_eq!(one("var o = {get v(){ return this._v * 2 }, set v(x){ this._v = x }, _v: 1}; o.v = 21; console.log(o.v, Object.keys(o).join())"), "42 v,_v");
    assert_eq!(one("var base = {a: 1}; var o = {...base, b: 2, ...{c: 3}}; console.log(JSON.stringify(o)); console.log(Math.max(...[1, 5, 3]))"), "{\"a\":1,\"b\":2,\"c\":3}|5");
    assert_eq!(one("function f(){ return arguments.length + ':' + Array.prototype.slice.call(arguments, 1).join() } console.log(f(1, 2, 3))"), "3:2,3");
    assert_eq!(one("var o = {}; Object.defineProperty(o, 'h', {value: 1}); o.e = 2; console.log(Object.keys(o).join(), o.h, JSON.stringify(o))"), "e 1 {\"e\":2}");
}

#[test]
fn classes_inheritance_and_super() {
    let src = "class A { constructor(n) { this.n = n } get dbl() { return this.n * 2 } static make(n) { return new this(n) } hi() { return 'A' + this.n } }
               class B extends A { f = 10; constructor(n) { super(n + 1); this.g = this.f + this.n } hi() { return 'B' + super.hi() } }
               var b = B.make(1); console.log(b.hi(), b.dbl, b.g, b instanceof A, b instanceof B, typeof A, Object.keys(b).join());
               try { A(1) } catch (e) { console.log(e.message) }";
    assert_eq!(
        one(src),
        "BA2 4 12 true true function n,f,g|Class constructor A cannot be invoked without 'new'"
    );
    assert_eq!(one("class E extends Error { constructor(m) { super(m); this.name = 'E' } } try { throw new E('bad') } catch (e) { console.log(e instanceof E, e instanceof Error, String(e)) }"), "true true E: bad");
    assert_eq!(one("class S { static count = 0; static inc() { return ++S.count } } S.inc(); console.log(S.inc())"), "2");
}

#[test]
fn map_set_symbol_and_optional_chaining() {
    assert_eq!(one("var m = new Map([['a', 1]]); m.set('b', 2).set('a', 3); var s = []; m.forEach((v, k) => s.push(k + v)); console.log(m.size, m.get('a'), m.has('z'), s.join(), [...m.keys()].join(), m.keys().next().value)"), "2 3 false a3,b2 a,b a");
    assert_eq!(one("var s = new Set([1, 2, 2, 3]); s.add(4).delete(1); var t = 0; for (var v of s) t += v; console.log(s.size, t, s.has(2), [...s].join())"), "3 9 true 2,3,4");
    assert_eq!(one("var a = Symbol('x'), b = Symbol('x'); var o = {}; o[a] = 1; o[b] = 2; console.log(typeof a, a === b, o[a], o[b], Object.keys(o).length, Symbol.for('k') === Symbol.for('k'))"), "symbol false 1 2 0 true");
    assert_eq!(one("var o = {a: {b: null, f() { return 7 }}}; console.log(o?.a?.b?.c, o.a?.f?.(), o.x?.y.z, o.a.g?.(), o?.['a']?.b)"), "undefined 7 undefined undefined null");
}

#[test]
fn regexp_and_string_methods() {
    assert_eq!(one("var m = /(\\d+)-(?<w>[a-z]+)/i.exec('x 12-Ab y'); console.log(m[0], m[1], m.index, m.groups.w, m.length)"), "12-Ab 12 2 Ab 3");
    assert_eq!(one("var r = /a(b)?/g; var out = []; var m; while ((m = r.exec('ab a ab')) !== null) out.push(m[0] + '@' + m.index + ':' + r.lastIndex); console.log(out.join())"), "ab@0:2,a@3:4,ab@5:7");
    assert_eq!(one("console.log('a-b_c d'.replace(/[-_ ]/g, '.'), 'abc'.replace(/(b)/, '[$1$&]'), 'x1y22'.replace(/\\d+/g, d => d * 2), 'aaa'.replace('a', 'b'), 'aaa'.replaceAll('a', 'b'))"), "a.b.c.d a[bb]c x2y44 baa bbb");
    assert_eq!(one("console.log('a1b22c'.split(/\\d+/).join('|'), 'abc'.split('').length, 'a,b,,c'.split(',').length, 'a1b'.split(/(\\d)/).join('|'), 'ab'.split(/(?:)/).join('|'))"), "a|b|c 3 4 a|1|b a|b");
    assert_eq!(one("console.log('foo bar'.match(/o+/)[0], 'a1b2'.match(/\\d/g).join(), 'abc'.match(/z/), 'abc'.search(/c/), /^a.c$/.test('abc'), /^a.c$/.test('abcd'), new RegExp('x+', 'g').flags, String(/a\\/b/gi))"), "oo 1,2 null 2 true false g /a\\/b/gi");
}

#[test]
fn json_dates_and_misc_builtins() {
    assert_eq!(one("console.log(JSON.stringify({a: [1, 'x', null, undefined, function(){}], b: undefined, c: {d: true}}), JSON.stringify([1, {a: 2}], null, 2).split('\\n').length, JSON.stringify('a\"b\\n'))"), "{\"a\":[1,\"x\",null,null,null],\"c\":{\"d\":true}} 6 \"a\\\"b\\n\"");
    assert_eq!(one("var v = JSON.parse('{\"a\":[1,2,{\"b\":null}],\"s\":\"x\\\\u0041\",\"n\":-1.5e2}'); console.log(v.a[2].b, v.s, v.n, JSON.stringify(v))"), "null xA -150 {\"a\":[1,2,{\"b\":null}],\"s\":\"xA\",\"n\":-150}");
    assert_eq!(one("try { JSON.parse('{bad') } catch (e) { console.log(e instanceof SyntaxError, e.name) }"), "true SyntaxError");
    assert_eq!(one("var d = new Date(Date.UTC(2024, 1, 29, 13, 5, 9, 7)); console.log(d.toISOString(), d.getUTCDay(), d.getTime(), new Date('2020-02-03').getTime(), JSON.stringify({d: new Date(0)}))"), "2024-02-29T13:05:09.007Z 4 1709211909007 1580688000000 {\"d\":\"1970-01-01T00:00:00.000Z\"}");
    assert_eq!(one("console.log([3, 1, 2].sort().join(), [10, 9, 1].sort().join(), [3, 1, 2].sort((a, b) => b - a).join(), ['b', undefined, 'a'].sort().length, [1, 2, 3].some(x => x > 2), [1, 2, 3].every(x => x > 2), [1, 2, 3].find(x => x > 1), [[1], [2]].map(x => x[0]).reverse().join(), [1, 2, 3, 4].splice(1, 2).join())"), "1,2,3 1,10,9 3,2,1 3 true false 2 2,1 2,3");
    assert_eq!(one("console.log((255).toString(16), (0.5).toFixed(2), (1234.5678).toFixed(1), Number.isInteger(5), parseFloat('3.14abc'), parseInt('0x1f'), encodeURIComponent('a b&c/d'), decodeURIComponent('%E3%81%82'), 'abc'.padStart(5, '*'), 'abc'.substring(2, 0))"), "ff 0.50 1234.6 true 3.14 31 a%20b%26c%2Fd あ **abc ab");
    assert_eq!(one("var f = function(a, b){ return this.x + a + b }; console.log(f.call({x: 1}, 2, 3), f.apply({x: 1}, [2, 3]), f.bind({x: 1}, 2)(3), Object.prototype.toString.call([]), (function(){ return typeof this }).call(undefined))"), "6 6 6 [object Array] object");
}

/// The shape of the real use case: a CommonJS package bundle whose exports are called from
/// "compiled" code with plain values, no JSON text and no handle table. Source mirrors the `tj`
/// fixture used in the Phase 0 measurements (`/home/fedora/work/thaw-tmp/tmp-run/jsr/proj/tj`).
#[test]
fn commonjs_package_exports_are_callable() {
    let bundle = r#"
        function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
        exports.fib = fib;
        exports.fibq = function (n) {
          const f = (k) => (k < 2 ? k : f(k - 1) + f(k - 2));
          return [n].map((x) => f(x))[0];
        };
        exports.wc = function (s) {
          const m = {};
          for (const w of s.split(" ")) m[w] = (m[w] || 0) + 1;
          return Object.keys(m).length;
        };
        // an ms-like parser, without regexp
        exports.secs = function (str) {
          var parts = String(str).trim().split(' ');
          var n = parseInt(parts[0], 10);
          switch (parts[1]) { case 'min': return n * 60; case 'h': return n * 3600; case 'd': return n * 86400; default: return undefined; }
        };
    "#;
    let mut it = Interp::new();
    let exports = it.run_commonjs(bundle).expect("bundle evaluates");
    let n = |v: R<Value>| v.expect("call ok").as_number().expect("number");
    assert_eq!(
        n(it.call_export(&exports, "fib", &[Value::Number(20.0)])),
        6765.0
    );
    assert_eq!(
        n(it.call_export(&exports, "fibq", &[Value::Number(15.0)])),
        610.0
    );
    assert_eq!(
        n(it.call_export(&exports, "wc", &[Value::str("a b c a b d e f g a")])),
        7.0
    );
    assert_eq!(
        n(it.call_export(&exports, "secs", &[Value::str("2 d")])),
        172800.0
    );
    // a missing export is a TypeError, like in JS
    assert!(matches!(
        it.call_export(&exports, "nope", &[]),
        Err(Ctl::Throw(_))
    ));
}

const MINI: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"p/index.js": function(module, exports, require, requireAsync, __filename, __dirname, __thaw_require) {
  var h = require('./h');
  exports.twice = function (x) { return h.dbl(x); };
  exports.greet = function (o) { return { msg: 'hi ' + o.name, tags: o.tags.map(function (t) { return t.toUpperCase(); }) }; };
  exports.boom = function () { throw new RangeError('nope'); };
  exports.VERSION = '1.0';
},
"p/h.js": function(module, exports) { exports.dbl = function (x) { return x * 2; }; },
"p/unused.js": function(module, exports) { function* g() {} },
};
var __thaw_bundle_require_maps = { "p/index.js": {"./h": "p/h.js"}, "p/h.js": {}, };
var __thaw_bundle_entry_key = "p/index.js";
})(globalThis.require);"#;

#[test]
fn package_host_loads_modules_and_exchanges_plain_data() {
    use crate::host::{HostError, Native, Package};
    let mut p = Package::load(MINI).expect("loads");
    assert_eq!(
        p.call("twice", &[Native::Number(21.0)]).ok(),
        Some(Native::Number(42.0))
    );
    let arg = Native::from_json(r#"{"name":"bo","tags":["a","b"]}"#)
        .ok()
        .unwrap();
    assert_eq!(
        p.call("greet", &[arg]).ok().unwrap().to_json(),
        r#"{"msg":"hi bo","tags":["A","B"]}"#
    );
    assert_eq!(p.get("VERSION").ok(), Some(Native::Str("1.0".into())));
    match p.call("boom", &[]) {
        Err(HostError::Thrown(m)) => assert_eq!(m, "RangeError: nope"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(p.call("twice", &[]), Ok(Native::Number(n)) if n.is_nan()));
}

#[test]
fn package_host_reports_unsupported_constructs_by_name() {
    use crate::host::{HostError, Package};
    let bundle = MINI.replace("exports.VERSION = '1.0';", "exports.VERSION = import('x');");
    match Package::load(&bundle) {
        Err(HostError::Unsupported(f)) => assert_eq!(f, "import()"),
        other => panic!("{:?}", other.err()),
    }
}

#[test]
fn c_abi_round_trips_json() {
    use std::ffi::{CStr, CString};
    let bundle = CString::new(MINI).unwrap();
    let handle = crate::host::thaw_tjs_load(bundle.as_ptr());
    assert!(handle >= 0);
    let (path, args) = (CString::new("twice").unwrap(), CString::new("[4]").unwrap());
    let out = crate::host::thaw_tjs_call(handle, path.as_ptr(), args.as_ptr());
    assert!(!out.is_null());
    // SAFETY: `out` was just returned by thaw_tjs_call and is freed exactly once below.
    unsafe {
        assert_eq!(CStr::from_ptr(out).to_str().unwrap(), "8");
        crate::host::thaw_tjs_free(out);
    }
    let missing = CString::new("missing").unwrap();
    assert!(crate::host::thaw_tjs_call(handle, missing.as_ptr(), args.as_ptr()).is_null());
    crate::host::thaw_tjs_unload(handle);
}

#[test]
fn scan_reports_reachable_blockers_only() {
    let clean = crate::scan::scan_bundle(MINI).ok().unwrap();
    assert!(clean.is_clean(), "{:?}", clean.blockers);
    assert_eq!((clean.factories_total, clean.factories_reachable), (3, 2)); // the unused generator is unreachable
    let blocked = MINI.replace("exports.VERSION = '1.0';", "exports.V = async () => await File.from('x'); var fs = require('fs'); label: for (;;) { break label; } if (typeof process !== 'undefined') {}");
    let r = crate::scan::scan_bundle(&blocked).ok().unwrap();
    let names: Vec<&str> = r.blockers.keys().map(String::as_str).collect();
    assert_eq!(names, ["global:File", "require:fs"]); // typeof-probed `process` and labels are not reported
    assert!(crate::scan::scan_bundle("var x = 1;").is_err());
}

/// Run `src` as a program (event loop included); console lines plus the exit code.
fn main_run(src: &str) -> (Vec<String>, i32) {
    let mut it = Box::new(Interp::new());
    let r = it.run_main(src);
    it.flush_partial();
    match r {
        Ok(code) => (it.out.clone(), code),
        Err(Ctl::Throw(v)) => {
            let mut out = it.out.clone();
            out.push(format!("Uncaught {}", it.to_str(&v)));
            (out, 1)
        }
        Err(_) => (it.out.clone(), -1),
    }
}

#[test]
fn nexttick_runs_before_promise_jobs_and_timers_last() {
    let (out, code) = main_run(
        "setTimeout(() => console.log('timer'), 0);
         setImmediate(() => console.log('immediate'));
         Promise.resolve().then(() => console.log('promise'));
         process.nextTick(() => console.log('tick'));
         queueMicrotask(() => console.log('microtask'));
         (async () => { console.log('async-sync'); await 0; console.log('async-resumed'); })();
         console.log('main');",
    );
    assert_eq!(
        out.join("|"),
        "async-sync|main|tick|promise|microtask|async-resumed|timer|immediate"
    );
    assert_eq!(code, 0);
}

#[test]
fn await_unwraps_values_and_throws_rejections() {
    let (out, _) = main_run(
        "async function f(x) { if (x) throw new RangeError('r'); return await Promise.resolve(7); }
         (async () => {
           console.log(await f(false));
           try { await f(true); } catch (e) { console.log(e.name); }
           try { await Promise.reject(3); } catch (e) { console.log('rejected', e); }
         })();",
    );
    assert_eq!(out.join("|"), "7|RangeError|rejected 3");
}

#[test]
fn many_concurrent_async_calls_and_never_settled_ones() {
    let (out, _) = main_run(
        "var n = 0; var gate = new Promise(function () {});
         for (var i = 0; i < 300; i++) (async () => { await 0; await null; n++; })();
         (async () => { await gate; console.log('unreachable'); })();
         setTimeout(() => console.log(n), 1);",
    );
    assert_eq!(out.join("|"), "300");
}

#[test]
fn exit_codes_and_unhandled_rejections() {
    assert_eq!(main_run("process.exit(4); console.log('no')"), (vec![], 4));
    assert_eq!(
        main_run("process.exitCode = 2; setTimeout(() => {}, 1)").1,
        2
    );
    let (out, code) = main_run("Promise.reject(new Error('boom'))");
    assert_eq!(
        (out.join("|"), code),
        ("Uncaught Error: boom".to_string(), 1)
    );
    // handled later in the same tick: not reported
    assert_eq!(
        main_run("var p = Promise.reject(1); p.catch(() => {})").1,
        0
    );
}

#[test]
fn timers_fire_in_expiry_order_and_intervals_repeat() {
    let (out, _) = main_run(
        "var n = 0; var h = setInterval(() => { console.log('i' + ++n); if (n == 2) clearInterval(h); }, 5);
         setTimeout(() => console.log('late'), 400);
         setTimeout(() => console.log('early'), 1);
         clearTimeout(setTimeout(() => console.log('never'), 2));",
    );
    assert_eq!(out.join("|"), "early|i1|i2|late");
}

#[test]
fn async_package_exports_are_awaited_by_the_host() {
    use crate::host::{HostError, Native, Package};
    let bundle = MINI.replace(
        "exports.VERSION = '1.0';",
        "exports.slow = async function (x) { await new Promise(function (r) { setTimeout(r, 2); }); return x + 1; };
         exports.bad = async function () { throw new TypeError('nope'); };",
    );
    let mut p = Package::load(&bundle).unwrap();
    match p.call("slow", &[Native::Number(41.0)]) {
        Ok(Native::Number(n)) => assert_eq!(n, 42.0),
        other => panic!("{:?}", other.err()),
    }
    assert!(matches!(p.call("bad", &[]), Err(HostError::Thrown(m)) if m.contains("nope")));
}

#[test]
fn user_iterables_work_in_for_of_spread_destructuring_and_array_from() {
    let src = "var it = { [Symbol.iterator]() { var i = 0; return { next() { return i < 3 ? { value: i++, done: false } : { value: undefined, done: true }; } }; } };
               var out = []; for (var x of it) out.push(x);
               var [a, b] = it;
               console.log(out.join(), [...it].join(), a + b, Array.from(it).join(), new Set(it).size, Math.max(...it));";
    assert_eq!(one(src), "0,1,2 0,1,2 1 0,1,2 3 2");
}

#[test]
fn objects_convert_through_to_primitive() {
    assert_eq!(
        one("var o = { valueOf() { return 42; }, toString() { return 's'; } };
             var p = { [Symbol.toPrimitive](h) { return h; } };
             var d = new Date(1000);
             console.log(o + 1, `${o}`, o > 41, o == 42, +p, `${p}`, p + '', d < new Date(2000), typeof (d + 1), d - 1)"),
        "43 s true true NaN string default true string 999"
    );
}

#[test]
fn typed_arrays_wrap_values_and_share_their_buffer() {
    assert_eq!(
        one("var b = new ArrayBuffer(4), u8 = new Uint8Array(b), u32 = new Uint32Array(b);
             u32[0] = 0x01020304; var c = new Uint8ClampedArray([300, -1, 1.5]);
             console.log(u8.join(), c.join(), new Int8Array([200])[0], u8.subarray(1, 3).length, u8.buffer === b, Buffer.from('hi').toString('hex'), new TextDecoder().decode(new TextEncoder().encode('é')))"),
        "4,3,2,1 255,0,2 -56 2 true 6869 é"
    );
}

#[test]
fn url_parses_and_resolves_relative_references() {
    assert_eq!(
        one(
            "var u = new URL('../x?a=1#h', 'http://User@Example.com:80/p/q/r');
             u.searchParams.append('b', 'c d');
             console.log(u.href, u.host, new URL('http://a.b:8080').port, URL.canParse('nope'))"
        ),
        "http://User@example.com/p/x?a=1&b=c+d#h example.com 8080 false"
    );
}

#[test]
fn methods_do_not_bind_their_own_name_but_named_function_expressions_do() {
    assert_eq!(
        one("function clone(x) { return 'outer' + x; }
             class A { clone(x) { return clone(x); } }
             var o = { clone(x) { return clone(x); } };
             var f = function self(n) { return n ? self(n - 1) + 1 : 0; };
             console.log(new A().clone(1), o.clone(2), f(3))"),
        "outer1 outer2 3"
    );
}

#[test]
fn native_base_classes_can_be_extended() {
    assert_eq!(
        one("class S extends Set { total() { var t = 0; this.forEach(function (v) { t += v; }); return t; } }
             var s = new S([1, 2, 3]); s.add(4);
             class E extends Error { constructor(m) { super(m); this.name = 'E'; } }
             class P extends Promise {}
             console.log(s.size, s.total(), s instanceof Set, s instanceof S, new E('x').message, new E('x') instanceof Error)"),
        "4 10 true true x true"
    );
}

#[test]
fn date_local_time_follows_the_process_zone_and_normalises_overflow() {
    // the zone is process-wide, so only zone-independent identities are asserted here
    assert_eq!(
        one("var d = new Date(2020, 0, 31, 12); d.setMonth(1);
             console.log(d.getMonth(), d.getDate(), new Date(2020, 11, 31, 23, 59).getFullYear(), new Date(d.getTime()).getTime() === d.getTime(),
               Date.UTC(2020, 0, 1), Date.parse('2020-01-02T03:04:05Z'), isNaN(Date.parse('2020-13-01')), new Date(2020, 0, 1).getTimezoneOffset() === -new Date(2020, 0, 1).getTimezoneOffset() * -1)"),
        "2 2 2020 true 1577836800000 1577934245000 true true"
    );
}

/// A bundle whose `module.exports` is itself the function, like `ms` or `minimist`.
const CALLABLE_EXPORTS: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"p/index.js": function(module, exports, require) {
  module.exports = function pkg(x) { return x + 1; };
  module.exports.named = function (o) { return { n: o.n * 2 }; };
},
};
var __thaw_bundle_require_maps = { "p/index.js": {}, };
var __thaw_bundle_entry_key = "p/index.js";
})(globalThis.require);"#;

fn typed_call(symbol: &str, args: &str) -> Result<String, String> {
    use crate::host::{thaw_tjs_call_typed_result, ThawResult};
    use std::ffi::{CStr, CString};
    let (symbol, args) = (CString::new(symbol).unwrap(), CString::new(args).unwrap());
    let ThawResult { value, error } = thaw_tjs_call_typed_result(symbol.as_ptr(), args.as_ptr());
    let take = |p: *mut std::ffi::c_char| {
        // SAFETY: `thaw_arena::owned_string` returns a NUL-terminated string we own.
        let text = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        unsafe { thaw_arena::destroy_string(p) };
        text
    };
    if error.is_null() {
        Ok(take(value))
    } else {
        Err(take(error))
    }
}

#[test]
fn typed_abi_registers_packages_and_calls_exports_by_symbol() {
    use crate::host::thaw_tjs_register;
    use std::ffi::CString;
    let (mini, callable) = (
        CString::new(MINI).unwrap(),
        CString::new(CALLABLE_EXPORTS).unwrap(),
    );
    assert_eq!(thaw_tjs_register(c"mini".as_ptr(), mini.as_ptr()), 1);
    assert_eq!(
        thaw_tjs_register(c"callable".as_ptr(), callable.as_ptr()),
        1
    );
    assert_eq!(typed_call("mini::twice", "[21]"), Ok("42".into()));
    assert_eq!(
        typed_call("mini::greet", r#"[{"name":"bo","tags":["a"]}]"#),
        Ok(r#"{"msg":"hi bo","tags":["A"]}"#.into())
    );
    // `module.exports` is the function itself: the export name does not exist as a property
    assert_eq!(typed_call("callable::pkg", "[1]"), Ok("2".into()));
    assert_eq!(
        typed_call("callable::named", r#"[{"n":4}]"#),
        Ok(r#"{"n":8}"#.into())
    );
}

#[test]
fn typed_abi_reports_js_exceptions_and_unroutable_calls_as_errors() {
    use crate::host::thaw_tjs_register;
    use std::ffi::CString;
    let mini = CString::new(MINI).unwrap();
    assert_eq!(thaw_tjs_register(c"mini-errors".as_ptr(), mini.as_ptr()), 1);
    assert_eq!(
        typed_call("mini-errors::boom", "[]"),
        Err("RangeError: nope".into())
    );
    assert!(typed_call("never-loaded::f", "[]")
        .unwrap_err()
        .contains("not loaded"));
    assert!(typed_call("mini-errors::twice", "{}")
        .unwrap_err()
        .contains("array"));
    assert_eq!(
        typed_call("\u{1}thaw-js: `x` cannot cross", "[]"),
        Err("thaw-js: `x` cannot cross".into())
    );
}

// ----- compiled callbacks (`thaw_tjs_register_callback`) -----

/// A stand-in for a generated `(context, args_json) -> result_json` adapter: sums the numbers.
unsafe extern "C" fn sum_adapter(
    _context: *const std::ffi::c_void,
    args: *const std::ffi::c_char,
) -> *mut std::ffi::c_char {
    let text = unsafe { std::ffi::CStr::from_ptr(args) }.to_str().unwrap();
    let crate::host::Native::Array(items) = crate::host::Native::from_json(text).unwrap() else {
        panic!("arguments are a JSON array")
    };
    let total: f64 = items
        .iter()
        .map(|n| {
            if let crate::host::Native::Number(x) = n {
                *x
            } else {
                0.0
            }
        })
        .sum();
    thaw_arena::owned_string(crate::number_to_string(total))
}

/// Reports a thrown error the way generated adapters do (`\u{2}` + `\u{1}Name\u{1}message`).
unsafe extern "C" fn throwing_adapter(
    _context: *const std::ffi::c_void,
    _args: *const std::ffi::c_char,
) -> *mut std::ffi::c_char {
    thaw_arena::owned_string("\u{2}\u{1}RangeError\u{1}from native")
}

/// Echoes whether the first argument was `undefined` (the compiled side's marker).
unsafe extern "C" fn undefined_probe_adapter(
    _context: *const std::ffi::c_void,
    args: *const std::ffi::c_char,
) -> *mut std::ffi::c_char {
    let text = unsafe { std::ffi::CStr::from_ptr(args) }.to_str().unwrap();
    thaw_arena::owned_string(if text.contains("$__thaw_napi_undefined$") {
        "true"
    } else {
        "false"
    })
}

const CALLBACK_BUNDLE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"p/index.js": function(module, exports, require) {
  exports.apply = function (f, x) { return f(x, 1, undefined) * 2; };
  exports.try = function (f) { try { return f(); } catch (e) { return e.name + ':' + e.message; } };
  exports.mark = function (f) { return f(undefined); };
  exports.same = function (f) { return f === f; };
},
};
var __thaw_bundle_require_maps = { "p/index.js": {}, };
var __thaw_bundle_entry_key = "p/index.js";
})(globalThis.require);"#;

/// What compiled code gets for an error string through `thaw_tjs_frame_error`.
fn frame_text(display: &str) -> String {
    use std::ffi::{CStr, CString};
    let input = thaw_arena::owned_string(CString::new(display).unwrap().as_bytes());
    let framed = unsafe { crate::host::thaw_tjs_frame_error(input) };
    // SAFETY: `framed` was just returned by `thaw_tjs_frame_error` and is freed exactly once.
    unsafe {
        let text = CStr::from_ptr(framed).to_str().unwrap().to_string();
        crate::host::thaw_tjs_free(framed);
        text
    }
}

fn callback_marker(id: i64) -> crate::host::Native {
    crate::host::Native::Object(vec![(
        "__thaw_tjs_cb__".into(),
        crate::host::Native::Number(id as f64),
    )])
}

#[test]
fn compiled_callbacks_are_callable_from_package_code() {
    use crate::host::{thaw_tjs_register_callback, HostError, Native, Package};
    let sum = thaw_tjs_register_callback(sum_adapter as *const _, std::ptr::null(), 3);
    let throwing = thaw_tjs_register_callback(throwing_adapter as *const _, std::ptr::null(), 0);
    let probe =
        thaw_tjs_register_callback(undefined_probe_adapter as *const _, std::ptr::null(), 1);
    // the same closure registers once: stable function identity
    assert_eq!(
        sum,
        thaw_tjs_register_callback(sum_adapter as *const _, std::ptr::null(), 3)
    );
    let mut p = Package::load(CALLBACK_BUNDLE).expect("loads");
    // f(x, 1, undefined): `undefined` arrives as the marker object (0 in the sum); the package doubles the result
    assert_eq!(
        p.call("apply", &[callback_marker(sum), Native::Number(20.0)])
            .ok(),
        Some(Native::Number(42.0))
    );
    // a native throw becomes a catchable JS error with its name and message
    assert_eq!(
        p.call("try", &[callback_marker(throwing)]).ok(),
        Some(Native::Str("RangeError:from native".into()))
    );
    // `undefined` reaches the compiled side as its marker; the marker comes back as `undefined`
    assert_eq!(
        p.call("mark", &[callback_marker(probe)]).ok(),
        Some(Native::Bool(true))
    );
    assert_eq!(
        p.call("same", &[callback_marker(sum)]).ok(),
        Some(Native::Bool(true))
    );
    // the compiled side's `undefined` marker as an argument is `undefined` for the package
    let absent = Native::Object(vec![("$__thaw_napi_undefined$".into(), Native::Bool(true))]);
    assert_eq!(
        p.call("apply", &[callback_marker(sum), absent]).ok(),
        Some(Native::Number(2.0))
    );
    // an uncaught native throw surfaces as a framed error for the caller
    match p.call("apply", &[callback_marker(throwing), Native::Number(1.0)]) {
        Err(HostError::Thrown(display)) => {
            assert_eq!(display, "RangeError: from native");
            assert_eq!(frame_text(&display), "\u{1}RangeError\u{1}from native");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn thrown_errors_are_framed_with_name_message_code_and_properties() {
    use crate::host::{HostError, Package};
    let bundle = MINI.replace(
        "exports.VERSION = '1.0';",
        "exports.fail = function () { var e = new TypeError('bad'); e.code = 'E_BAD'; e.status = 400; throw e; };\nexports.fail2 = function () { throw 'plain'; };",
    );
    let mut p = Package::load(&bundle).expect("loads");
    match p.call("fail", &[]) {
        Err(HostError::Thrown(display)) => {
            assert_eq!(display, "TypeError: bad");
            assert_eq!(
                frame_text(&display),
                "\u{1}TypeError\u{1}bad\u{3}E_BAD\u{5}{\"code\":\"E_BAD\",\"status\":400}"
            );
        }
        other => panic!("{other:?}"),
    }
    match p.call("fail2", &[]) {
        Err(HostError::Thrown(m)) => assert_eq!(m, "plain"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_package_can_be_called_again_from_its_own_callback() {
    use crate::host::{
        thaw_tjs_call_typed_result, thaw_tjs_free, thaw_tjs_register, thaw_tjs_register_callback,
    };
    use std::ffi::{CStr, CString};
    // The callback calls `nest::inc(1)` through the C ABI while `nest::run` is still evaluating.
    unsafe extern "C" fn reenter(
        _context: *const std::ffi::c_void,
        _args: *const std::ffi::c_char,
    ) -> *mut std::ffi::c_char {
        let symbol = CString::new("nest::inc").unwrap();
        let args = CString::new("[1]").unwrap();
        let result = thaw_tjs_call_typed_result(symbol.as_ptr(), args.as_ptr());
        let text = if result.value.is_null() {
            let error = unsafe { CStr::from_ptr(result.error) }
                .to_str()
                .unwrap()
                .to_string();
            unsafe { thaw_tjs_free(result.error) };
            format!("{error:?}")
        } else {
            let value = unsafe { CStr::from_ptr(result.value) }
                .to_str()
                .unwrap()
                .to_string();
            unsafe { thaw_tjs_free(result.value) };
            value
        };
        thaw_arena::owned_string(text)
    }
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = { "n/index.js": function(module, exports, require) {
  exports.inc = function (x) { return x + 1; };
  exports.run = function (f) { return f() + 10; };
}, };
var __thaw_bundle_require_maps = { "n/index.js": {}, };
var __thaw_bundle_entry_key = "n/index.js";
})(globalThis.require);"#;
    let (name, source) = (CString::new("nest").unwrap(), CString::new(bundle).unwrap());
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    let id = thaw_tjs_register_callback(reenter as *const _, std::ptr::null(), 0);
    let symbol = CString::new("nest::run").unwrap();
    let args = CString::new(format!("[{{\"__thaw_tjs_cb__\":{id}}}]")).unwrap();
    let result = thaw_tjs_call_typed_result(symbol.as_ptr(), args.as_ptr());
    assert!(result.error.is_null(), "{:?}", unsafe {
        CStr::from_ptr(result.error)
    });
    let value = unsafe { CStr::from_ptr(result.value) }
        .to_str()
        .unwrap()
        .to_string();
    unsafe { thaw_tjs_free(result.value) };
    assert_eq!(value, "12");
}

// ----- class instances as handles (`thaw_tjs_class_call`) -----

const CLASS_BUNDLE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"k/index.js": function(module, exports, require) {
  class Box {
    constructor(v) { if (v === 'bad') throw new TypeError('bad box'); this.v = v; }
    get() { return this.v; }
    add(n) { this.v += n; return this; }
    peek(other) { return other.v; }
    static of(v) { return new Box(v); }
  }
  exports.Box = Box;
},
};
var __thaw_bundle_require_maps = { "k/index.js": {}, };
var __thaw_bundle_entry_key = "k/index.js";
})(globalThis.require);"#;

fn class_call(symbol: &str, args: &str, receiver: i64, handle: bool) -> Result<String, String> {
    use std::ffi::{CStr, CString};
    let (symbol, args) = (CString::new(symbol).unwrap(), CString::new(args).unwrap());
    let result = crate::host::thaw_tjs_class_call(
        symbol.as_ptr(),
        args.as_ptr(),
        receiver,
        u8::from(handle),
    );
    // SAFETY: both strings come from `thaw_tjs_class_call` and are freed exactly once.
    unsafe {
        if result.value.is_null() {
            let error = CStr::from_ptr(result.error).to_str().unwrap().to_string();
            crate::host::thaw_tjs_free(result.error);
            Err(error)
        } else {
            let value = CStr::from_ptr(result.value).to_str().unwrap().to_string();
            crate::host::thaw_tjs_free(result.value);
            Ok(value)
        }
    }
}

#[test]
fn class_instances_live_behind_handles_and_can_be_passed_back() {
    use std::ffi::CString;
    let (name, source) = (
        CString::new("k").unwrap(),
        CString::new(CLASS_BUNDLE).unwrap(),
    );
    assert_eq!(
        crate::host::thaw_tjs_register(name.as_ptr(), source.as_ptr()),
        1
    );
    let id: i64 = class_call("$new$k::Box$arity1", "[40]", 0, true)
        .unwrap()
        .parse()
        .unwrap();
    assert!(id > 0);
    assert_eq!(
        class_call("$method$k::Box$get$overload0$arity0", "[]", id, false).unwrap(),
        "40"
    );
    // a method returning `this` hands out a second handle to the same object
    let same: i64 = class_call("$method$k::Box$add$overload0$arity1", "[2]", id, true)
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(same, id);
    assert_eq!(
        class_call("$method$k::Box$get$overload0$arity0", "[]", same, false).unwrap(),
        "42"
    );
    // an instance handle passed as an argument arrives as the object itself
    let other: i64 = class_call("$staticmethod$k::Box$of$overload0$arity1", "[7]", 0, true)
        .unwrap()
        .parse()
        .unwrap();
    let marker = format!("[{{\"__thaw_tjs_handle__\":{other}}}]");
    assert_eq!(
        class_call("$method$k::Box$peek$overload0$arity1", &marker, id, false).unwrap(),
        "7"
    );
    // errors keep their name and message; an unknown handle is an error, not a crash
    let error = class_call("$new$k::Box$arity1", "[\"bad\"]", 0, true).unwrap_err();
    assert_eq!(error, "TypeError: bad box");
    assert_eq!(frame_text(&error), "\u{1}TypeError\u{1}bad box");
    assert!(class_call("$method$k::Box$get$overload0$arity0", "[]", 9999, false).is_err());
    // retain/release: the value is dropped with the last reference
    assert_eq!(crate::host::thaw_tjs_retain_handle(id), 1);
    assert_eq!(crate::host::thaw_tjs_release_handle(id), 1);
    assert_eq!(
        class_call("$method$k::Box$get$overload0$arity0", "[]", id, false).unwrap(),
        "42"
    );
    assert_eq!(crate::host::thaw_tjs_release_handle(id), 1);
    assert_eq!(crate::host::thaw_tjs_release_handle(id), 0);
}

#[test]
fn global_construct_and_call_handles() {
    use crate::host::*;
    use std::ffi::CString;
    let (name, source) = (
        CString::new("g").unwrap(),
        CString::new(
            CLASS_BUNDLE
                .replace("\"k/", "\"g/")
                .replace("k/index", "g/index"),
        )
        .unwrap(),
    );
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    let q = |s: &str| CString::new(s).unwrap();
    let ctor = thaw_tjs_get_global(q("g::Box").as_ptr());
    assert!(ctor > 0);
    assert!(thaw_tjs_get_global(q("Math.max").as_ptr()) > 0);
    let args = q(r#"{"root":{"r":0},"nodes":[{"a":[{"v":5}]}]}"#);
    let made = thaw_tjs_construct_handle_graph_args_result(ctor as u64, args.as_ptr());
    assert!(made.error.is_null() && made.value > 0);
    let max = thaw_tjs_get_global(q("Math.max").as_ptr());
    let args = q(r#"{"root":{"r":0},"nodes":[{"a":[{"v":3},{"v":9}]}]}"#);
    let r = thaw_tjs_call_handle_handle_graph_args_result(max as u64, args.as_ptr(), false);
    assert!(r.error.is_null() && r.value > 0);
}

// ----- WeakMap / WeakSet / WeakRef / FinalizationRegistry (expected text recorded from Node 22) -----

#[test]
fn weak_collections_match_node() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/differential/weak.js"
    ))
    .unwrap();
    let want = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/differential/weak.out"
    ))
    .unwrap();
    assert_eq!(run(&src).join("\n") + "\n", want);
    assert!(
        Interp::new().is_builtin_global("WeakMap") && Interp::new().is_builtin_global("WeakRef")
    );
}

#[test]
fn symbol_keyed_define_property_and_to_string_tag() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, Symbol.toStringTag, { value: 'X' }); console.log(Object.prototype.toString.call(o), Object.getOwnPropertySymbols(o).length, o[Symbol.toStringTag], Object.getOwnPropertyDescriptor(o, Symbol.toStringTag).value)"),
        "[object X] 1 X X"
    );
}

// ----- static blocks (expected text recorded from Node 22) -----

#[test]
fn static_blocks_and_the_scan_features() {
    assert_eq!(
        one("class S { static n = 1; static { S.m = S.n + 1; var t = 5; S.t = t; } } console.log(S.m, S.t)"),
        "2 5"
    );
    let with = |body: &str| MINI.replace("exports.VERSION = '1.0';", body);
    let r = crate::scan::scan_bundle(&with("class C { static { C.y = 2; } }"))
        .ok()
        .unwrap();
    assert!(r.is_clean(), "{:?}", r.blockers);
    // not implemented: the scan must say so (the evaluator would stop at load time)
    let r = crate::scan::scan_bundle(&with(
        "class C { x = 1; } exports.u = import.meta.url; exports.n = require('fs');",
    ))
    .ok()
    .unwrap();
    let names: Vec<&str> = r.blockers.keys().map(String::as_str).collect();
    assert_eq!(names, ["import.meta", "require:fs"]);
}

// ----- user iterators are pulled lazily and closed on early exit (expected text recorded from Node 22) -----

#[test]
fn user_iterators_are_pulled_lazily_and_closed_on_early_exit() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/differential/lazy_iteration.js"
    ))
    .unwrap();
    let want = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/differential/lazy_iteration.out"
    ))
    .unwrap();
    assert_eq!(run(&src).join("\n") + "\n", want);
}

#[test]
fn scan_accepts_generators_bigint_private_names_and_new_target() {
    let bundle = MINI.replace(
        "exports.VERSION = '1.0';",
        "class C { #p = 1n; get p() { return this.#p; } static *items() { yield new.target; } } \
         exports.V = async function* () { yield* [C.items()]; for await (const x of []) {} };",
    );
    let r = crate::scan::scan_bundle(&bundle).ok().unwrap();
    assert!(r.is_clean(), "{:?}", r.blockers);
}

#[test]
fn evaluator_runs_generators_bigint_and_private_names() {
    assert_eq!(
        one("function* g() { const x = yield 1; yield x * 2; } var it = g(); console.log(it.next().value, it.next(21).value, it.next().done)"),
        "1 42 true"
    );
    assert_eq!(
        one("console.log(2n ** 64n, typeof 1n, 7n / 2n, BigInt('0x10') + 1n)"),
        "18446744073709551616n bigint 3n 17n"
    );
    assert_eq!(
        one("class A { #x = 2; static #n = 3; get v() { return this.#x * A.#n; } static has(o) { return #x in o; } } console.log(new A().v, A.has(new A()), A.has({}))"),
        "6 true false"
    );
    assert_eq!(
        one("function F() { return new.target === F } console.log(new F() instanceof F, F())"),
        "true false"
    );
    assert_eq!(
        one("var p = new Proxy({}, { get: (t, k) => 'got ' + String(k) }); console.log(p.a)"),
        "got a"
    );
}

#[test]
fn large_maps_and_sets_agree_with_linear_semantics() {
    let src = "var m = new Map(), s = new Set();
        for (var i = 0; i < 200; i++) { m.set('k' + i, i); s.add(i % 50); m.set(i, 'n' + i); }
        m.set(NaN, 'nan'); m.set(-0, 'zero'); s.add(NaN); s.add(NaN); s.add(-0); s.add(0);
        var del = m.delete('k7'), miss = m.delete('nope');
        console.log(m.size, s.size, m.get('k8'), m.get(NaN), m.get(0), m.has('k7'), m.has(199), del, miss, s.has(NaN), s.has(49), s.has(50));
        s.delete(3); m.set('k8', 'again');
        console.log(s.has(3), s.size, m.get('k8'), [...m.keys()].slice(0, 3).join(), [...s].slice(0, 4).join());
        var o = {}, a = {}; m.set(o, 1); m.set(a, 2); m.set(1n, 'big'); m.set(1, 'num');
        console.log(m.get(o), m.get(a), m.get({}), m.get(1n), m.get(1), m.get('1'));";
    assert_eq!(
        one(src),
        "400 51 8 nan zero false true true false true true false|false 50 again k0,0,k1 0,1,2,4|1 2 undefined big num undefined"
    );
}

#[test]
fn crypto_global_fills_random_values_and_makes_uuids() {
    assert_eq!(
        one("var a = new Uint8Array(32), r = crypto.getRandomValues(a);
             var u = crypto.randomUUID(), m = globalThis.__thaw_crypto_module;
             var big = 0; try { crypto.getRandomValues(new Uint8Array(65537)); } catch (e) { big = e.name; }
             var bad = ''; try { crypto.getRandomValues(new Float64Array(1)); } catch (e) { bad = e.name; }
             console.log(r === a, a.some(function (x) { return x !== 0; }),
               /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(u),
               m.randomBytes(5).length, m.randomInt(3, 4), big, bad)"),
        "true true true 5 3 QuotaExceededError TypeMismatchError"
    );
}

#[test]
fn crypto_create_hash_matches_node() {
    assert_eq!(
        one("var m = globalThis.__thaw_crypto_module;
             console.log(m.createHash('sha1').update('a').update(Buffer.from('bc')).digest('hex'),
               m.createHash('md5').update('abc').digest('base64'),
               m.createHash('sha256').digest('hex'), m.createHash('sha1').digest().length)"),
        "a9993e364706816aba3e25717850c26c9cd0d89d kAFQmDzST7DWlj99KOF/cg== e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 20"
    );
}

#[test]
fn blob_and_file_basics() {
    assert_eq!(
        one("var f = new File(['ab', new Uint8Array([99])], 'x.txt', { type: 'Text/Plain' });
             console.log(f instanceof Blob, f instanceof File, f.name, f.size, f.type, new Blob([]) instanceof File,
               f.slice(1).size)"),
        "true true x.txt 3 text/plain false 2"
    );
}

#[test]
fn scan_accepts_esm_origin_calls_and_self_made_global_slots() {
    let src = "module.exports = (function(require) {
var __thaw_bundle_factories = {
\"p/index.js\": function(__thaw_esm_origin_0) { return function(module, exports, require) {
  if (typeof __thaw_esm_origin_0 !== 'undefined') __thaw_esm_origin_0.edge('./a', 1);
  var _a; (_a = globalThis).__slot ?? (_a.__slot = {});
  var tx = 1; exports.o = { tx }; exports.f = function () { return exports.o.tx(1); };
  var s = globalThis.__slot;
}; },
};
var __thaw_bundle_require_maps = { \"p/index.js\": {}, };
var __thaw_bundle_entry_key = \"p/index.js\";
})(globalThis.require);";
    let report = crate::scan::scan_bundle(src).ok().unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
}

// ----- live-handle intrinsics (`thaw_tjs_get_property_result`, selected method calls, ...) -----

#[test]
fn handle_intrinsics_read_properties_call_methods_and_capture_graphs() {
    use crate::host::*;
    use std::ffi::{CStr, CString};
    let (name, source) = (
        CString::new("kh").unwrap(),
        CString::new(CLASS_BUNDLE).unwrap(),
    );
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    let text =
        |p: *mut std::ffi::c_char| unsafe { CStr::from_ptr(p).to_str().unwrap().to_string() };
    let id: u64 = class_call("$new$kh::Box$arity1", "[40]", 0, true)
        .unwrap()
        .parse()
        .unwrap();

    // a property read is a new handle; resolving it yields the plain wire
    let key = CString::new("v").unwrap();
    let property = thaw_tjs_get_property_result(id, key.as_ptr());
    assert!(property.error.is_null());
    let resolved = thaw_tjs_resolve_handle_graph_result(property.value);
    assert_eq!(
        text(resolved.value),
        r#"{"root":{"v":40},"nodes":[],"leases":[],"napiLeases":[]}"#
    );

    // a selected method call returning `this` comes back as the receiver's own handle
    let method = thaw_tjs_get_property_result(id, CString::new("add").unwrap().as_ptr());
    let args =
        CString::new(r#"{"root":{"r":0},"nodes":[{"a":[{"v":2}]}],"leases":[],"napiLeases":[]}"#)
            .unwrap();
    let call =
        thaw_tjs_call_selected_method_graph_result(id, method.value, key.as_ptr(), args.as_ptr());
    assert!(call.error.is_null());
    assert_eq!(
        text(call.value),
        format!(r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{id}}}],"leases":[{id}],"napiLeases":[]}}"#)
    );

    // an object stays a handle when captured; a missing handle reports an error
    let captured = thaw_tjs_capture_handle_graph_result(id);
    assert!(text(captured.value).contains(r#""hdl""#));
    assert!(!thaw_tjs_get_property_result(9999, key.as_ptr())
        .error
        .is_null());
    assert_eq!(thaw_tjs_exception_native_token(id), 0);
}

// ----- slice 4: mixed bundles, host os/tty/fs pieces, web streams -----

#[test]
fn mixed_bundle_runs_native_modules_with_origin_reads_and_opaque_edges() {
    use crate::host::{Native, Package};
    let payload = r#"{"main":"m/u.js","modules":[{"key":"m/u.js","source":"import { origin as __thaw_esm_origin_0 } from \"__thaw_private_origin_spec/m/u.js\";\nimport \"node:os\";\nexport function shout(s) { return s.toUpperCase() + __thaw_esm_origin_0.readDefault(\"node:os\").EOL; }\nexport default 7;","imports":{"__thaw_private_origin_spec/m/u.js":"__thaw_private_origin_key/m/u.js","node:os":"__thaw_private_opaque_edge/m/u.js/0"}}],"opaqueEdges":[{"key":"__thaw_private_opaque_edge/m/u.js/0","importer":"m/u.js","specifier":"node:os","target":"node:os","factory":"node:os","asynchronous":false}]}"#;
    let mut literal = String::new();
    crate::builtins::json_quote(payload, &mut literal);
    let bundle = format!(
        r#"module.exports = (function(require) {{
var __thaw_bundle_factories = {{
"m/index.js": function(module, exports, require) {{ var u = require("./u.js"); exports.up = function (s) {{ return u.shout(s); }}; exports.seven = u.default; }},
"node:os": function(module, exports, require) {{ module.exports = {{ EOL: "!" }}; module.exports.default = module.exports; module.exports.__esModule = true; }},
"m/u.js": function() {{ throw new TypeError('Native ESM must be loaded by its registered entry'); }},
}};
var __thaw_bundle_require_maps = {{ "m/index.js": {{"./u.js": "m/u.js"}}, "node:os": {{}}, "m/u.js": {{}}, }};
var __thaw_bundle_entry_key = "m/index.js";
var __thaw_native_registered = globalThis.__thaw_register_native_bundle({literal});
}})(globalThis.require);"#
    );
    assert!(crate::scan::scan_bundle(&bundle).unwrap().is_clean());
    let mut p = Package::load(&bundle).expect("loads");
    assert_eq!(
        p.call("up", &[Native::Str("ab".into())]).unwrap(),
        Native::Str("AB!".into())
    );
    assert_eq!(p.get("seven").unwrap(), Native::Number(7.0));
}

#[test]
fn os_tty_process_and_fs_host_functions() {
    assert_eq!(
        one("var i = JSON.parse(__thaw_os_identity()); var o = JSON.parse(__thaw_os_info());
             console.log(i.platform, typeof i.userInfo.uid, o.cpus.length > 0, o.totalmem > 0, o.availableParallelism >= 1,
               JSON.parse(__thaw_os_get_priority(0)).ok, JSON.parse(__thaw_os_get_priority(-5)).code,
               __thaw_process_is_tty(-1))"),
        "linux number true true true true ESRCH false"
    );
    let dir = std::env::temp_dir().join(format!("thaw-js-sysio-{}", std::process::id()));
    let d = dir.to_string_lossy().replace('\\', "/");
    let src = format!(
        "var d = '{d}'; var f = (op, p, v, r) => JSON.parse(__thaw_fs(op, p, v || '', !!r));
         console.log(f('mkdir', d + '/a/b', '', true).ok, f('exists', d + '/a/b').exists);
         console.log(f('write', d + '/a/b/x.txt', '68690a').ok, f('read', d + '/a/b/x.txt').data);
         var s = f('stat', d + '/a/b/x.txt'); console.log(s.file, s.directory, s.length, typeof s.exact.ino);
         console.log(f('readdir', d + '/a/b', 'typed').entries.map(e => e.stat.file).join());
         var o = f('fd_open', d + '/a/b/x.txt', 'r,438'); var rd = f('fd_read_all', String(o.fd));
         console.log(rd.data, f('fd_close', String(o.fd)).ok, f('fd_close', String(o.fd)).code);
         console.log(f('read', d + '/nope').code, f('chown', d, '').code);
         console.log(f('rmdir', d, '', true).ok, f('exists', d).exists);"
    );
    assert_eq!(
        run(&src).join("|"),
        "true true|true 68690a|true false 3 string|true|68690a true EBADF|ENOENT ENOSYS|true false"
    );
}

#[test]
fn web_streams_read_write_and_iterate() {
    let (out, _) = main_run(
        "var chunks = [];
         var rs = new ReadableStream({ start(c) { c.enqueue('a'); c.enqueue('b'); c.close(); } });
         (async () => {
           for await (const v of rs) chunks.push(v);
           const ws = new WritableStream({ write(v) { chunks.push(v + '!'); }, close() { chunks.push('closed'); } });
           const w = ws.getWriter(); await w.write('x'); await w.close();
           const src = new ReadableStream({ pull(c) { c.enqueue(1); c.close(); } });
           const r = src.getReader(); const first = await r.read(); const second = await r.read();
           chunks.push(first.value, first.done, second.done, src.locked);
           console.log(chunks.join());
         })();",
    );
    assert_eq!(out.join("|"), "a,b,x!,closed,1,false,true,true");
}

#[test]
fn scan_accepts_global_presence_probes_and_the_globalthis_polyfill() {
    let src = "module.exports = (function(require) {
var __thaw_bundle_factories = {
\"p/index.js\": function(module, exports, require) {
  exports.a = globalThis.Deno ? 1 : 2;
  exports.b = typeof globalThis.Bun === 'undefined';
  exports.c = globalThis.Zed && 1;
  exports.poly = function () {
    if (typeof globalThis === 'object') return;
    __magic__.globalThis = __magic__;
  };
},
};
var __thaw_bundle_require_maps = { \"p/index.js\": {}, };
var __thaw_bundle_entry_key = \"p/index.js\";
})(globalThis.require);";
    let report = crate::scan::scan_bundle(src).ok().unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
    let called = src.replace(
        "exports.c = globalThis.Zed && 1;",
        "exports.c = globalThis.Zed();",
    );
    assert!(!crate::scan::scan_bundle(&called).unwrap().is_clean());
    // generated `with (..)` source cannot be parsed by `new Function` here
    let with_code = src.replace(
        "exports.a = ",
        "exports.w = '  with (locals || {}) {'; exports.a = ",
    );
    assert!(crate::scan::scan_bundle(&with_code)
        .unwrap()
        .blockers
        .contains_key("dynamic-with-code"));
}

#[test]
fn scan_accepts_method_calls_guarded_by_a_truthiness_test_of_the_same_name() {
    let src = "module.exports = (function(require) {
var __thaw_bundle_factories = {
\"p/index.js\": function(module, exports, require) {
  exports.a = function (options, date) { return options?.in ? options.in(date) : date; };
  exports.b = function (locale, parts) { if (locale.localize.preprocessor) { parts = locale.localize.preprocessor(parts); } return parts; };
},
};
var __thaw_bundle_require_maps = { \"p/index.js\": {}, };
var __thaw_bundle_entry_key = \"p/index.js\";
})(globalThis.require);";
    let report = crate::scan::scan_bundle(src).ok().unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
    let unguarded = src.replace("options?.in ? options.in(date) : date", "options.in(date)");
    assert!(!crate::scan::scan_bundle(&unguarded).unwrap().is_clean());
}

const DATES: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"d/index.js": function(module, exports, require) {
  exports.iso = function (d) { return d.toISOString(); };
  exports.plus = function (d, n) { return new Date(d.getTime() + n); };
  exports.bad = function (d) { return isNaN(d.getTime()); };
  exports.nested = function () { return { at: new Date(5), n: [new Date(7)], none: new Date(NaN) }; };
},
};
var __thaw_bundle_require_maps = { "d/index.js": {}, };
var __thaw_bundle_entry_key = "d/index.js";
})(globalThis.require);"#;

#[test]
fn a_date_crosses_the_typed_abi_as_a_wire_node_both_ways() {
    use std::ffi::CString;
    let bundle = CString::new(DATES).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"dt".as_ptr(), bundle.as_ptr()),
        1
    );
    let date = |ms: &str| format!("{{\"__thaw_tjs_date__\":{ms}}}");
    assert_eq!(
        typed_call("dt::iso", &format!("[{}]", date("86400000"))).unwrap(),
        "\"1970-01-02T00:00:00.000Z\""
    );
    // an Invalid Date crosses as "NaN" (or null, how compiled JSON spells NaN)
    for nan in ["\"NaN\"", "null"] {
        assert_eq!(
            typed_call("dt::bad", &format!("[{}]", date(nan))).unwrap(),
            "true"
        );
    }
    // a Date result is a live handle, unless the caller asks for the wire node (`\u{3}`)
    assert!(typed_call("dt::plus", &format!("[{},5]", date("86400000")))
        .unwrap()
        .starts_with("{\"__thaw_tjs_handle__\":"));
    assert_eq!(
        typed_call("\u{3}dt::plus", &format!("[{},5]", date("86400000"))).unwrap(),
        date("86400005")
    );
    assert_eq!(
        typed_call("dt::nested", "[]").unwrap(),
        format!(
            "{{\"at\":{},\"n\":[{}],\"none\":{}}}",
            date("5"),
            date("7"),
            date("\"NaN\"")
        )
    );
}

#[test]
fn graph_wire_date_bigint_regexp_map_and_set_nodes_decode() {
    use crate::host::*;
    use std::ffi::{CStr, CString};
    let bundle = CString::new(MINI).unwrap();
    assert_eq!(thaw_tjs_register(c"gw".as_ptr(), bundle.as_ptr()), 1);
    let text_of = |wire: &str| {
        let wire = CString::new(wire).unwrap();
        let handle = thaw_tjs_retain_graph_result(wire.as_ptr());
        assert!(handle.error.is_null(), "{wire:?}");
        // SAFETY: a NUL-terminated string owned by the handle table.
        unsafe { CStr::from_ptr(thaw_tjs_handle_to_string(handle.value)) }
            .to_string_lossy()
            .into_owned()
    };
    let one = |node: &str| {
        format!(r#"{{"root":{{"r":0}},"nodes":[{node}],"leases":[],"napiLeases":[]}}"#)
    };
    assert!(text_of(&one(r#"{"d":17280000000}"#)).contains("1970"));
    assert_eq!(text_of(&one(r#"{"d":null}"#)), "Invalid Date");
    assert_eq!(
        text_of(&one(r#"{"bi":"123456789012345678901234567890"}"#)),
        "123456789012345678901234567890"
    );
    assert_eq!(text_of(&one(r#"{"re":["a+","gi",0]}"#)), "/a+/gi");
    let set = r#"{"root":{"r":0},"nodes":[{"s":{"r":1}},{"a":[{"v":1},{"v":2},{"v":2}]}],"leases":[],"napiLeases":[]}"#;
    assert_eq!(text_of(set), "[object Set]");
    let map = r#"{"root":{"r":0},"nodes":[{"m":{"r":1}},{"a":[{"r":2}]},{"a":[{"v":"k"},{"v":1}]}],"leases":[],"napiLeases":[]}"#;
    assert_eq!(text_of(map), "[object Map]");
}

#[cfg(feature = "intl")]
#[test]
fn intl_formats_numbers_dates_and_lists_through_the_native_icu_data() {
    let out = run(
        "console.log(new Intl.NumberFormat('de-DE').format(1234567.5));
         console.log(new Intl.PluralRules('en').select(1));
         console.log(new Intl.ListFormat('en').format(['a','b','c']));
         console.log(new Intl.DateTimeFormat('en-US',{timeZone:'UTC'}).format(new Date(0)));
         console.log(new Intl.Collator('en',{numeric:true}).compare('a2','a10'));",
    );
    assert_eq!(out.join("|"), "1.234.567,5|one|a, b, and c|1/1/1970|-1");
}

#[cfg(feature = "intl")]
#[test]
fn intl_date_time_format_reports_parts_range_and_resolved_options() {
    let out = run(
        "const f = new Intl.DateTimeFormat('en-US',{timeZone:'UTC',year:'numeric',month:'short',day:'2-digit'});
         const d = new Date(Date.UTC(2024,0,5));
         console.log(f.formatToParts(d).map(p => p.type).join());
         console.log(typeof f.formatRange(d, new Date(Date.UTC(2024,0,9))));
         console.log(f.resolvedOptions().timeZone);",
    );
    assert_eq!(out.join("|"), "month,literal,day,literal,year|string|UTC");
}

#[cfg(feature = "intl")]
#[test]
fn scan_knows_intl_globals_and_namespace_prototype_methods() {
    let src = "module.exports = (function(require) {
var __thaw_bundle_factories = {
\"p/index.js\": function(module, exports, require) {
  exports.a = function (d) { return new Intl.DateTimeFormat('en').formatRange(d, d) + Intl.NumberFormat.supportedLocalesOf; };
},
};
var __thaw_bundle_require_maps = { \"p/index.js\": {}, };
var __thaw_bundle_entry_key = \"p/index.js\";
})(globalThis.require);";
    let report = crate::scan::scan_bundle(src).ok().unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
}
