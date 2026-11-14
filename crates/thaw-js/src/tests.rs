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
fn interned_declared_names_are_bounded_by_distinct_names() {
    // 10k distinct `let` names, each in its own block, inside a loop that runs twice
    let mut src = String::from("for (let r = 0; r < 2; r++) {\n");
    for i in 0..10_000 {
        src.push_str(&format!("{{ let v{i} = {i}; }}\n"));
    }
    src.push_str("}\n");
    let mut it = Interp::new();
    let before = it.names.len();
    assert!(it.run(&src).is_ok(), "first run");
    let after_one = it.names.len();
    assert_eq!(after_one - before, 10_001, "10k block names plus `r`");
    assert!(it.run(&src).is_ok(), "second run");
    assert_eq!(
        it.names.len(),
        after_one,
        "re-executing declarations interns nothing new"
    );
}

#[test]
fn shadowing_and_closures_resolve_with_interned_names() {
    assert_eq!(
        one(
            "let x = 1; function f(){ return x } { let x = 2; console.log(f(), x) } console.log(x)"
        ),
        "1 2|1"
    );
    assert_eq!(one("var fs = []; for (let i = 0; i < 3; i++) { let j = i * 2; fs.push(() => i + j) } console.log(fs[0](), fs[1](), fs[2]())"), "0 3 6");
    assert_eq!(one("let e = 'outer'; try { throw 'inner' } catch (e) { let e2 = e; console.log(e, e2) } console.log(e)"), "inner inner|outer");
    assert_eq!(one("function mk(){ let n = 0; class C { static k = n } let g = () => n; n = 5; return [g(), C.k] } console.log(mk().join(','))"), "5,0");
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
fn string_replace_uses_utf16_units_with_astral() {
    // match indices and $` $' offsets are UTF-16 units; an astral char before/inside/after shifts nothing
    assert_eq!(
        one("console.log('x😀y1'.replace(/\\d/, '[$&]'))"),
        "x😀y[1]"
    );
    assert_eq!(one("console.log('a😀b'.replace(/😀/, '<$&>'))"), "a<😀>b");
    assert_eq!(
        one("console.log('1😀'.replace(/1/, \"[$'$&]\"))"),
        "[😀1]😀"
    );
    assert_eq!(
        one("console.log('😀1😀2'.replace(/\\d/g, '<$&>'))"),
        "😀<1>😀<2>"
    );
    assert_eq!(one("console.log('😀a'.replace(/a/, '[$`]'))"), "😀[😀]");
    assert_eq!(
        one("console.log('😀a1'.replace(/(?<d>\\d)/, '[$<d>]'))"),
        "😀a[1]"
    );
    // function replacer receives the UTF-16 offset (and the subject)
    assert_eq!(one("console.log('😀a'.replace(/a/, (m, o) => o))"), "😀2");
    assert_eq!(
        one("console.log('😀ab'.replace(/b/g, (m, o, s) => '' + o + s.length))"),
        "😀a34"
    );
    // string pattern (replace and replaceAll) and global regexp with $`
    assert_eq!(
        one("console.log('😀a😀a'.replaceAll('a', \"[$`]\"))"),
        "😀[😀]😀[😀a😀]"
    );
    assert_eq!(
        one("console.log('😀a😀a'.replace(/a/g, \"[$`]\"))"),
        "😀[😀]😀[😀a😀]"
    );
    // empty matches at the end of an astral subject
    assert_eq!(one("console.log('a😀'.replace(/$/g, '-'))"), "a😀-");
}

#[test]
fn regexp_string_index_ascii_and_non_ascii_subjects() {
    // indexOf over non-ASCII text (BMP and astral), with and without a start index
    assert_eq!(one("var a='héllo wörld é'; var b='a😀b'; console.log(a.indexOf('ö'), a.indexOf('é', 2), b.indexOf('b'), b.indexOf('b', 2), a.indexOf('zz'), a.lastIndexOf('l'))"), "7 12 3 3 -1 9");
    // long subjects (over the ASCII-cache threshold): ASCII and non-ASCII, same shapes
    assert_eq!(one("var big='x'.repeat(300)+'é'+'y'.repeat(10); var big2='abc'.repeat(100); console.log(big.indexOf('y'), big.indexOf('é'), big2.indexOf('c', 250), big.indexOf('y', 302), big.lastIndexOf('x'))"), "301 300 251 302 299");
    assert_eq!(one("var s='ab'.repeat(200)+'c'; console.log(s.indexOf('c'), s.indexOf('b', 399), s.match(/c/).index, s.replace(/b/g,'').length)"), "400 399 400 201");
    // global exec loop with lastIndex over non-ASCII text
    assert_eq!(one("var r=/b/g; var out=[]; var m; var t='é b ébé b'; while((m=r.exec(t))!==null) out.push(m.index+':'+r.lastIndex); console.log(out.join(','), r.lastIndex)"), "2:3,5:6,8:9 0");
    // astral char: index/lastIndex in UTF-16 units, and a lastIndex inside a surrogate pair
    assert_eq!(one("var r2=/x/g; var m2=r2.exec('a😀x'); console.log(m2.index, r2.lastIndex); r2.lastIndex=2; console.log(r2.exec('a😀x').index, r2.lastIndex)"), "3 4|3 4");
    // sticky with lastIndex, ASCII and non-ASCII
    assert_eq!(one("var sy=/b/y; sy.lastIndex=1; console.log(sy.exec('ab')!==null, sy.lastIndex); var sy2=/b/y; sy2.lastIndex=2; console.log(sy2.exec('éb')===null, sy2.lastIndex)"), "true 2|true 0");
    // match / replace / search over non-ASCII subjects
    assert_eq!(one("console.log('é1 é22'.match(/\\d+/g).join(), 'ü-ö-ü'.replace(/-/g, '+'), 'é1 é22'.match(/\\d+/).index, 'ü1ö'.replace(/\\d/, '[$&]'), 'ü1ö'.search(/1/))"), "1,22 ü+ö+ü 1 ü[1]ö 1");
    assert_eq!(one("var rep='é-'.repeat(150)+'z'; console.log(rep.replace(/-/g,'+').length, rep.match(/z/).index, rep.replaceAll('é','e').indexOf('e-'))"), "301 300 0");
    // charCodeAt walks UTF-16 units of a non-ASCII string (surrogate halves, out of range)
    assert_eq!(one("var u='é😀x'; console.log(u.charCodeAt(0), u.charCodeAt(1), u.charCodeAt(2), u.charCodeAt(3), u.charCodeAt(4))"), "233 55357 56832 120 NaN");
}

#[test]
fn ascii_flag_cache_tracks_the_subject() {
    let ascii: crate::JsStr = "a".repeat(300).into();
    assert!(ascii.is_ascii_cached());
    let mixed: crate::JsStr = format!("{}é", "a".repeat(299)).into();
    assert!(!mixed.is_ascii_cached());
    // the cache now holds `mixed`; the ASCII subject must not inherit its answer
    assert!(ascii.is_ascii_cached());
    assert!(!mixed.is_ascii_cached());
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

/// A native-addon package bundle: `.node` is loaded through thaw-js `require` (host.rs `addon_of`), and the
/// node-gyp-build style probes come with it (`__non_webpack_require__`, a dynamic require).
const NATIVE_ADDON_BUNDLE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"addon/index.js": function(module, exports, require) {
  var nodeRequire = typeof __webpack_require__ === 'function' ? __non_webpack_require__ : require;
  var native = require('./build/Release/addon.node');
  var other = require(exports.p);
  exports.value = function () { return [native, other, nodeRequire]; };
},
};
var __thaw_bundle_require_maps = { "addon/index.js": {}, };
var __thaw_bundle_entry_key = "addon/index.js";
})(globalThis.require);"#;

#[test]
fn scan_addon_mode_tolerates_native_loader_keys_only() {
    let blocked = crate::scan::scan_bundle(NATIVE_ADDON_BUNDLE).ok().unwrap();
    let names: Vec<&str> = blocked.blockers.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "global:__non_webpack_require__",
            "require:./build/Release/addon.node",
            "require:dynamic"
        ]
    );
    // addon mode: the `.node` require, the webpack probe and the dynamic require beside it are the loader
    let addon = crate::scan::scan_addon_bundle(NATIVE_ADDON_BUNDLE)
        .ok()
        .unwrap();
    assert!(addon.is_clean(), "{:?}", addon.blockers);
    // a non-addon package with the same `.node` require stays blocked on the plain scan
    let plain = "module.exports = (function(require){ var __thaw_bundle_factories = { \"p/index.js\": function(module, exports, require) { exports.v = require('./build/Release/addon.node'); } }; var __thaw_bundle_require_maps = { \"p/index.js\": {} }; var __thaw_bundle_entry_key = \"p/index.js\"; })(globalThis.require);";
    let plain_report = crate::scan::scan_bundle(plain).ok().unwrap();
    assert!(!plain_report.is_clean());
    assert_eq!(
        plain_report
            .blockers
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["require:./build/Release/addon.node"]
    );
    // the addon-mode tolerance is scoped to native loader keys: a dynamic require with no `.node`
    // require beside it (and an unrelated `eval`) stays blocked in addon mode
    let dynamic_only = plain.replace(
        "require('./build/Release/addon.node')",
        "require(exports.p)",
    );
    let report = crate::scan::scan_addon_bundle(&dynamic_only).ok().unwrap();
    assert_eq!(
        report
            .blockers
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["require:dynamic"]
    );
    let with_eval = NATIVE_ADDON_BUNDLE.replace(
        "exports.value = function ()",
        "exports.e = function (s) { return eval(s); };\n  exports.value = function ()",
    );
    let report = crate::scan::scan_addon_bundle(&with_eval).ok().unwrap();
    assert_eq!(
        report
            .blockers
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["direct-eval"]
    );
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
    main_run_after_gap(src, 0)
}

/// `main_run` with `gap_ms` of wall-clock time between interpreter install and the program's start
/// (what a loaded runner can stretch: prelude work, thread descheduling).
fn main_run_after_gap(src: &str, gap_ms: u64) -> (Vec<String>, i32) {
    let mut it = Box::new(Interp::new());
    std::thread::sleep(std::time::Duration::from_millis(gap_ms));
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
fn rejected_await_in_try_catch_inside_timer_driven_async_listener() {
    // chokidar-shaped: an async listener awaits a stat-like promise that rejects from a
    // microtask, catches it, and defers a follow-up event through a timer (TJ168 section 2).
    // Expected output is what Node prints for the same program.
    let (out, code) = main_run(
        "var out = [];
         function statLike(p) {
           return new Promise(function (res, rej) {
             queueMicrotask(function () {
               if (p === 'missing') rej(Object.assign(new Error('ENOENT: no such file'), { code: 'ENOENT' }));
               else res({ size: 1 });
             });
           });
         }
         var listener = async function (path) {
           try {
             var s = await statLike(path);
             out.push('stat:' + s.size);
           } catch (e) {
             out.push('catch:' + e.code);
             setTimeout(function () { out.push('unlink:' + path); }, 150);
           }
         };
         setTimeout(function () { listener('missing'); }, 10);
         setTimeout(function () { listener('present'); }, 20);
         setTimeout(function () { console.log(out.join('|')); }, 400);",
    );
    assert_eq!(out.join("|"), "catch:ENOENT|stat:1|unlink:missing");
    assert_eq!(code, 0);
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
fn timer_base_is_script_start_not_interpreter_install() {
    // same program as timers_fire_in_expiry_order_and_intervals_repeat, but the program starts 450 ms
    // after the interpreter was installed: its timers must count from the program's start, so the
    // 5 ms interval still fires before the 400 ms timer
    let (out, _) = main_run_after_gap(
        "var n = 0; var h = setInterval(() => { console.log('i' + ++n); if (n == 2) clearInterval(h); }, 5);
         setTimeout(() => console.log('late'), 400);
         setTimeout(() => console.log('early'), 1);
         clearTimeout(setTimeout(() => console.log('never'), 2));",
        450,
    );
    assert_eq!(out.join("|"), "early|i1|i2|late");
}

#[test]
fn rejected_await_timers_count_from_script_start_after_a_gap() {
    // same program as rejected_await_in_try_catch_inside_timer_driven_async_listener, started 300 ms
    // after install: the 150 ms follow-up is armed after the 10/20 ms listeners run, so it must
    // still come before the 400 ms printer
    let (out, code) = main_run_after_gap(
        "var out = [];
         function statLike(p) {
           return new Promise(function (res, rej) {
             queueMicrotask(function () {
               if (p === 'missing') rej(Object.assign(new Error('ENOENT: no such file'), { code: 'ENOENT' }));
               else res({ size: 1 });
             });
           });
         }
         var listener = async function (path) {
           try {
             var s = await statLike(path);
             out.push('stat:' + s.size);
           } catch (e) {
             out.push('catch:' + e.code);
             setTimeout(function () { out.push('unlink:' + path); }, 150);
           }
         };
         setTimeout(function () { listener('missing'); }, 10);
         setTimeout(function () { listener('present'); }, 20);
         setTimeout(function () { console.log(out.join('|')); }, 400);",
        300,
    );
    assert_eq!(out.join("|"), "catch:ENOENT|stat:1|unlink:missing");
    assert_eq!(code, 0);
}

#[test]
fn timer_start_is_the_live_clock_at_the_call() {
    // the 2 ms timer is armed after 60 ms of busy work, so it is due 62 ms in: it runs after the
    // 5 ms timer armed before the busy work (Node prints A B)
    let (out, _) = main_run(
        "var s = Date.now();
         setTimeout(function () { console.log('A'); }, 5);
         while (Date.now() - s < 60);
         setTimeout(function () { console.log('B'); }, 2);",
    );
    assert_eq!(out.join("|"), "A|B");
}

#[test]
fn interval_rearm_start_is_read_before_the_callback() {
    // the interval's first run busy-waits 30 ms: its re-arm starts from the clock read before that
    // callback (due 20 ms), so the second run precedes the 30 ms timer (Node prints iv1 iv2 t)
    let (out, _) = main_run(
        "var n = 0; var h = setInterval(function () {
           console.log('iv' + ++n);
           if (n == 1) { var s = Date.now(); while (Date.now() - s < 30); }
           if (n == 2) clearInterval(h);
         }, 10);
         setTimeout(function () { console.log('t'); }, 30);",
    );
    assert_eq!(out.join("|"), "iv1|iv2|t");
}

#[test]
fn interval_rearm_start_is_live_not_turn_cached() {
    // a 1 ms timer busy-waits 400 ms in the first turn; the 2 ms interval runs after it in that same
    // turn and re-arms from the live clock (due ~402 ms in), so the 300 ms timer runs between its two
    // runs (Node prints iv1 t iv2). A re-arm from the turn's cached clock would be due at ~4 ms
    let (out, _) = main_run(
        "setTimeout(function () { var s = Date.now(); while (Date.now() - s < 400); }, 1);
         var n = 0; var h = setInterval(function () {
           console.log('iv' + ++n);
           if (n == 2) clearInterval(h);
         }, 2);
         setTimeout(function () { console.log('t'); }, 300);",
    );
    assert_eq!(out.join("|"), "iv1|t|iv2");
}

#[test]
fn timer_created_in_callback_after_busy_wait_uses_live_clock() {
    // inside a 1 ms callback: arm a 40 ms timer, busy-wait 60 ms, then arm a 5 ms timer. The 5 ms one
    // is due ~65 ms in, after the 40 ms timer (Node prints t40 x)
    let (out, _) = main_run(
        "setTimeout(function () {
           setTimeout(function () { console.log('t40'); }, 40);
           var s = Date.now(); while (Date.now() - s < 60);
           setTimeout(function () { console.log('x'); }, 5);
         }, 1);",
    );
    assert_eq!(out.join("|"), "t40|x");
}

#[test]
fn immediate_runs_before_a_one_ms_interval_second_tick() {
    // a 1 ms interval armed by the main script re-arms from the live clock, so its second tick is due
    // only in a later loop turn, after the check phase runs the immediate (Node: the immediate is
    // recorded before tick 2)
    let (out, _) = main_run(
        "var n = 0; var h = setInterval(function () {
           console.log('tick' + (++n));
           if (n == 2) clearInterval(h);
         }, 1);
         setImmediate(function () { console.log('imm'); });",
    );
    let pos = |name: &str| out.iter().position(|l| l == name).unwrap();
    assert!(pos("imm") < pos("tick2"), "order was {}", out.join("|"));
}

#[test]
fn zero_delay_timer_from_main_still_runs_before_immediate() {
    // a main-script timer of 0 ms is due on the first turn, ahead of setImmediate
    let (out, _) = main_run(
        "setImmediate(function () { console.log('imm'); });
         setTimeout(function () { console.log('t0'); }, 0);",
    );
    assert_eq!(out.join("|"), "t0|imm");
}

#[test]
fn refresh_uses_live_clock() {
    // inside a 1 ms callback: arm a 20 ms timer and a 40 ms timer, busy-wait 30 ms, then refresh the
    // 20 ms one: it is due ~51 ms in, after the 40 ms timer (Node prints b a)
    let (out, _) = main_run(
        "setTimeout(function () {
           var a = setTimeout(function () { console.log('a'); }, 20);
           setTimeout(function () { console.log('b'); }, 40);
           var s = Date.now(); while (Date.now() - s < 30);
           a.refresh();
         }, 1);",
    );
    assert_eq!(out.join("|"), "b|a");
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
fn typed_array_canonical_numeric_keys_are_integer_indexed_not_ordinary() {
    assert_eq!(
        one("var s = new Uint8Array(3); s[-1] = 5; s['1.5'] = 1; s['-0'] = 2; s.foo = 4;
             console.log(s[-1], s['1.5'], s['-0'], Object.keys(s).join(), -1 in s, '1.5' in s, s.hasOwnProperty('-1'), s.hasOwnProperty(0), s.foo, s['01']);"),
        "undefined undefined undefined 0,1,2,foo false false false true 4 undefined"
    );
}

#[test]
fn typed_array_invalid_numeric_key_define_throws_and_delete_follows_validity() {
    assert_eq!(
        one("var t = new Uint8Array(2);
             try { Object.defineProperty(t, '-1', {value: 1}); } catch (e) { console.log(e.constructor.name, e.message); }
             console.log(Reflect.defineProperty(t, '1.5', {value: 1}), Reflect.set(t, '-1', 3), Reflect.get(t, '-1'), delete t[-1], delete t[0]);"),
        "TypeError Invalid typed array index|false true undefined true false"
    );
}

#[test]
fn bigint_and_buffer_canonical_numeric_keys_are_integer_indexed() {
    assert_eq!(
        one("var b = new BigInt64Array(2); b[-1] = 5n; b['1.5'] = 1n;
             console.log(b[-1], b['1.5'], Object.keys(b).join(), -1 in b);
             var f = Buffer.from([1, 2]); f[-1] = 9; f.x = 3;
             console.log(f[-1], Object.keys(f).join(), JSON.stringify(f));"),
        "undefined undefined 0,1 false|undefined 0,1,x {\"type\":\"Buffer\",\"data\":[1,2]}"
    );
}

#[test]
fn dataview_is_an_array_buffer_view() {
    assert_eq!(
        one("var b = new ArrayBuffer(4), d = new DataView(b, 1, 2);
             console.log(ArrayBuffer.isView(d), ArrayBuffer.isView(new Uint8Array(1)), ArrayBuffer.isView(b), ArrayBuffer.isView({}), ArrayBuffer.isView(null), ArrayBuffer.isView.name, ArrayBuffer.isView.length)"),
        "true true false false false isView 1"
    );
}

#[test]
fn dataview_util_types_agree_with_is_view() {
    assert_eq!(
        one(
            "var d = new DataView(new ArrayBuffer(4), 1, 2), t = globalThis.__thaw_util_types;
             console.log(t.isArrayBufferView(d), t.isTypedArray(d), t.isDataView(d))"
        ),
        "true false true"
    );
}

#[test]
fn dataview_structured_clone_keeps_window_and_shares_cloned_buffer() {
    assert_eq!(
        one("var b = new ArrayBuffer(8), d = new DataView(b, 2, 4); d.setUint8(0, 9);
             var c = structuredClone(d);
             console.log(c instanceof DataView, c.byteOffset, c.byteLength, c.getUint8(0), c.buffer !== b, c.buffer.byteLength)"),
        "true 2 4 9 true 8"
    );
}

#[test]
fn dataview_constructor_nan_offset_is_zero_and_ranges_throw() {
    assert_eq!(
        one("var b = new ArrayBuffer(8);
             console.log(new DataView(b, NaN).byteOffset, new DataView(b, NaN, NaN).byteLength);
             try { new DataView(b, 9); } catch (e) { console.log(e.message); }
             try { new DataView(b, 0, 20); } catch (e) { console.log(e.message); }"),
        "0 0|Start offset 9 is outside the bounds of the buffer|Invalid DataView length 20"
    );
}

#[test]
fn typed_arrays_iterate_in_for_of_spread_array_from_and_entries() {
    assert_eq!(
        one("var u = new Uint8Array([7, 8]), out = [];
             for (var x of u) out.push(x);
             var f = new Float64Array([1.5, 2.5]);
             console.log(out.join(), [...f].join(), Array.from(u).join(), Array.from(u.entries()).join(';'), [...u.keys()].join())"),
        "7,8 1.5,2.5 7,8 0,7;1,8 0,1"
    );
}

#[test]
fn typed_array_symbol_iterator_is_the_values_function() {
    assert_eq!(
        one("var it = new Uint8Array([7, 8])[Symbol.iterator]();
             var a = it.next(), b = it.next(), c = it.next();
             var TP = Object.getPrototypeOf(Uint8Array.prototype);
             console.log(a.value, b.value, c.done, TP[Symbol.iterator] === TP.values, Float32Array.prototype[Symbol.iterator] === TP.values)"),
        "7 8 true true true"
    );
}

#[test]
fn buffer_iterates_like_a_uint8array() {
    assert_eq!(
        one("var b = Buffer.from([104, 105]), out = [];
             for (var x of b) out.push(x);
             console.log(out.join(), [...b].join(), Array.from(b).join())"),
        "104,105 104,105 104,105"
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

const NULL_PROTO_EXPORTS: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"p/index.js": function(module, exports, require) {
  module.exports.mk = function () { var o = Object.create(null); o.a = 1; o.n = Object.create(null); return o; };
  module.exports.plain = function () { return { a: 1 }; };
},
};
var __thaw_bundle_require_maps = { "p/index.js": {}, };
var __thaw_bundle_entry_key = "p/index.js";
})(globalThis.require);"#;

#[test]
fn typed_abi_marks_null_prototype_objects_on_the_wire() {
    use crate::host::thaw_tjs_register;
    use std::ffi::CString;
    let bundle = CString::new(NULL_PROTO_EXPORTS).unwrap();
    assert_eq!(thaw_tjs_register(c"nullp".as_ptr(), bundle.as_ptr()), 1);
    // nested and top-level null-prototype objects carry the marker; a plain object does not
    assert_eq!(
        typed_call("nullp::mk", "[]"),
        Ok(r#"{"a":1,"n":{"__thaw_tjs_nullproto__":true},"__thaw_tjs_nullproto__":true}"#.into())
    );
    assert_eq!(typed_call("nullp::plain", "[]"), Ok(r#"{"a":1}"#.into()));
}

#[test]
fn value_op_skips_a_namespace_segment_that_module_exports_lacks() {
    use crate::host::thaw_tjs_register;
    use std::ffi::CString;
    // `export = NS`: module.exports is the namespace, so `NS.enc.Hex` must read `enc.Hex`
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = { "ns/index.js": function(module, exports, require) {
  module.exports = { enc: { Hex: { x: 1 } } };
}, };
var __thaw_bundle_require_maps = { "ns/index.js": {}, };
var __thaw_bundle_entry_key = "ns/index.js";
})(globalThis.require);"#;
    let (name, source) = (CString::new("ns").unwrap(), CString::new(bundle).unwrap());
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    assert_eq!(
        class_call("$value$ns::NS.enc.Hex$arity0", "[]", 0, false).unwrap(),
        r#"{"x":1}"#
    );
}

#[test]
fn error_wire_framed_strings_revive_as_error_objects() {
    use crate::host::thaw_tjs_register;
    use std::ffi::CString;
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = { "fr/index.js": function(module, exports, require) {
  exports.t = function (e) { return (e instanceof Error) + "|" + e.name + "|" + e.message + "|" + e.code; };
  exports.kind = function (e) { return typeof e; };
  exports.rethrow = function (e) { if (e instanceof Error) throw e; return "kept"; };
}, };
var __thaw_bundle_require_maps = { "fr/index.js": {}, };
var __thaw_bundle_entry_key = "fr/index.js";
})(globalThis.require);"#;
    let (name, source) = (CString::new("fr").unwrap(), CString::new(bundle).unwrap());
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    // legacy `\u{1}Name\u{1}message` framing
    assert_eq!(
        typed_call("fr::t", r#"["\u0001Error\u0001boom"]"#),
        Ok(r#""true|Error|boom|undefined""#.into())
    );
    // `\u{3}code` suffix becomes `.code`
    assert_eq!(
        typed_call("fr::t", r#"["\u0001Error\u0001boom\u0003E42"]"#),
        Ok(r#""true|Error|boom|E42""#.into())
    );
    // tagged `E1` frame (`\u{1e}E1:<units>:-:<message>`)
    assert_eq!(
        typed_call("fr::t", r#"["\u0001Error\u0001\u001eE1:4:-:boom"]"#),
        Ok(r#""true|Error|boom|undefined""#.into())
    );
    // a framed error thrown by a handler is the same Error the compiled side sent (`emit('error', e)` rethrow)
    assert!(typed_call("fr::rethrow", r#"["\u0001Error\u0001boom"]"#)
        .unwrap_err()
        .contains("boom"));
    // plain strings stay strings: no leading U+0001, or a lone U+0001 with no closing frame delimiter
    assert_eq!(
        typed_call("fr::kind", r#"["a\u0001Error\u0001boom"]"#),
        Ok(r#""string""#.into())
    );
    assert_eq!(
        typed_call("fr::kind", r#"["\u0001x"]"#),
        Ok(r#""string""#.into())
    );
}

/// Node 22.23.1 records for an Error with an extra own property and a `cause`:
/// `Object.keys` is `["extra"]`, `JSON.stringify` is `{"extra":1}`, and the cause descriptor is
/// `{"value":"x","writable":true,"enumerable":false,"configurable":true}`.
#[test]
fn native_error_frames_keep_one_object_per_id_and_hide_the_id_and_restore_cause() {
    use crate::host::thaw_tjs_register;
    use std::ffi::CString;
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = { "ei/index.js": function(module, exports, require) {
  exports.same = function (a, b) { return String(a === b); };
  exports.keys = function (e) { return Object.keys(e).join(","); };
  exports.json = function (e) { return JSON.stringify(e); };
  exports.cause = function (e) { return String(e.cause); };
  exports.causeEnum = function (e) { return String(Object.prototype.propertyIsEnumerable.call(e, "cause")); };
  exports.causeMsg = function (e) { return String(e.cause && e.cause.message) + "|" + String(e.cause instanceof Error); };
}, };
var __thaw_bundle_require_maps = { "ei/index.js": {}, };
var __thaw_bundle_entry_key = "ei/index.js";
})(globalThis.require);"#;
    let (name, source) = (CString::new("ei").unwrap(), CString::new(bundle).unwrap());
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    // `\u{5}` bag with the native identity `\0id` (JSON-escaped in the argument text)
    let frame = |id: u32, bag: &str| {
        format!(r#""\u0001Error\u0001\u001eE1:4:-:boom\u0005{{\"\\u0000id\":{id}{bag}}}""#)
    };
    // the same native Error crossing twice is one object
    let args = format!("[{},{}]", frame(4096, ""), frame(4096, ""));
    assert_eq!(typed_call("ei::same", &args), Ok(r#""true""#.into()));
    let args = format!("[{},{}]", frame(4096, ""), frame(8192, ""));
    assert_eq!(typed_call("ei::same", &args), Ok(r#""false""#.into()));
    // the id is not an own enumerable key and not JSON; the extra property is
    let extra = r#",\"extra\":1"#;
    assert_eq!(
        typed_call("ei::keys", &format!("[{}]", frame(4096, extra))),
        Ok(r#""extra""#.into())
    );
    assert_eq!(
        typed_call("ei::json", &format!("[{}]", frame(4096, extra))),
        Ok(r#""{\"extra\":1}""#.into())
    );
    // the `\u{2}cause` bag entry becomes a non-enumerable own `cause`
    let cause = r#",\"\\u0002cause\":\"why\""#;
    assert_eq!(
        typed_call("ei::cause", &format!("[{}]", frame(4097, cause))),
        Ok(r#""why""#.into())
    );
    assert_eq!(
        typed_call("ei::causeEnum", &format!("[{}]", frame(4097, cause))),
        Ok(r#""false""#.into())
    );
    // a `\u{2}` cause segment in the frame suffix is restored as a tagged Error
    let nested = r#""\u0001Error\u0001\u001eE1:4:-:boom\u0002\u0001Error\u0001\u001eE1:2:-:in""#;
    assert_eq!(
        typed_call("ei::causeMsg", &format!("[{nested}]")),
        Ok(r#""in|true""#.into())
    );
}

/// `thaw_tjs_exception_native_token` reads the hidden native id of a thrown Error back to the live
/// native packet it was built from; 0 for anything else, and 0 once the packet is gone.
#[test]
fn native_error_token_round_trips_through_the_hidden_id() {
    use crate::host::{thaw_tjs_error_from_tagged_handle, thaw_tjs_exception_native_token};
    use std::ffi::CString;
    let identity = 0x7f00_0010_usize;
    let (id, packet) =
        thaw_arena::exception::register_packet(thaw_arena::exception::ExceptionPacket {
            id: 0,
            pending: 0,
            native_text: 0,
            native_error_identity: identity,
            object: 0,
            native: 0,
            aggregate: 0,
            tag: 0,
            number: 0,
            bigint: 0,
            boolean: 0,
            _roots: Vec::new(),
            _owned_bytes: Vec::new(),
            _leases: Vec::new(),
        })
        .unwrap();
    let framed = CString::new(format!(
        "\u{1}Error\u{1}\u{1e}E1:4:-:boom\u{5}{{\"\\u0000id\":{identity}}}"
    ))
    .unwrap();
    let made = thaw_tjs_error_from_tagged_handle(framed.as_ptr());
    assert!(made.error.is_null());
    assert_eq!(thaw_tjs_exception_native_token(made.value), id);
    // a plain Error (no hidden id) has no native token
    let plain = CString::new("\u{1}Error\u{1}boom").unwrap();
    let plain = thaw_tjs_error_from_tagged_handle(plain.as_ptr());
    assert_eq!(thaw_tjs_exception_native_token(plain.value), 0);
    // not an object at all
    assert_eq!(thaw_tjs_exception_native_token(0), 0);
    drop(packet);
    assert_eq!(thaw_tjs_exception_native_token(made.value), 0);
}

/// Node 22.23.1: `Object.keys(e)` is `["extra"]`, `JSON.stringify(e)` is `{"extra":1}` and the cause
/// is a non-enumerable own data property (the source of the `keys`/`json`/`cause` checks above).
#[test]
fn error_extra_props_and_cause_enumerate_like_node() {
    assert_eq!(
        one(r#"const e = new Error('m', { cause: 'x' }); e.extra = 1;
Object.defineProperty(e, 'hid', { value: 2, enumerable: false });
console.log(JSON.stringify(Object.keys(e)), JSON.stringify(e), JSON.stringify(Object.getOwnPropertyDescriptor(e, 'cause')));"#),
        r#"["extra"] {"extra":1} {"value":"x","writable":true,"enumerable":false,"configurable":true}"#
    );
    assert_eq!(
        one("const c = new Error('m2', { cause: new Error('in') }); console.log(JSON.stringify(Object.keys(c)), c.cause.message, Object.getOwnPropertyDescriptor(c, 'cause').enumerable)"),
        "[] in false"
    );
}

thread_local! {
    /// The argument JSON of the last `capture_adapter` call.
    static CAPTURED_ARGS: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Records its argument JSON, so a test can see what compiled code is handed.
unsafe extern "C" fn capture_adapter(
    _context: *const std::ffi::c_void,
    args: *const std::ffi::c_char,
) -> *mut std::ffi::c_char {
    let text = unsafe { std::ffi::CStr::from_ptr(args) }
        .to_str()
        .unwrap()
        .to_string();
    CAPTURED_ARGS.with(|c| *c.borrow_mut() = text);
    thaw_arena::owned_string("null")
}

#[test]
fn error_frame_handed_back_to_compiled_callback_keeps_frame() {
    use crate::host::{thaw_tjs_register_callback, Native, Package};
    let capture = thaw_tjs_register_callback(capture_adapter as *const _, std::ptr::null(), 1);
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = { "rt/index.js": function(module, exports, require) {
  exports.pass = function (cb, e) { cb(e); return 1; };
}, };
var __thaw_bundle_require_maps = { "rt/index.js": {}, };
var __thaw_bundle_entry_key = "rt/index.js";
})(globalThis.require);"#;
    let mut p = Package::load(bundle).expect("loads");
    // a compiled Error (framed string) emitted through JS and handed back to a compiled callback:
    // the callback gets the same frame, which the compiled side revives as an Error again
    let frame = "\u{1}RangeError\u{1}boom";
    assert_eq!(
        p.call(
            "pass",
            &[callback_marker(capture), Native::Str(frame.into())]
        )
        .ok(),
        Some(Native::Number(1.0))
    );
    let got = CAPTURED_ARGS.with(|c| c.borrow().clone());
    assert_eq!(
        Native::from_json(&got).unwrap(),
        Native::Array(vec![Native::Str(frame.into())])
    );
    // a plain object with a `message` is not an Error: it crosses as an object
    let plain = Native::Object(vec![("message".into(), Native::Str("boom".into()))]);
    p.call("pass", &[callback_marker(capture), plain.clone()])
        .ok();
    let got = CAPTURED_ARGS.with(|c| c.borrow().clone());
    assert_eq!(Native::from_json(&got).unwrap(), Native::Array(vec![plain]));
}

thread_local! {
    /// `(json, error)` of each `record_settle` call, in order.
    static SETTLED: RefCell<Vec<(Option<String>, Option<String>)>> = const { RefCell::new(Vec::new()) };
}

/// Records what a watched promise settled with.
extern "C" fn record_settle(
    _context: *mut std::ffi::c_void,
    json: *const std::ffi::c_char,
    error: *const std::ffi::c_char,
) {
    let text = |p: *const std::ffi::c_char| {
        (!p.is_null()).then(|| {
            unsafe { std::ffi::CStr::from_ptr(p) }
                .to_string_lossy()
                .into_owned()
        })
    };
    SETTLED.with(|s| s.borrow_mut().push((text(json), text(error))));
}

#[test]
fn pending_promise_settles_through_its_watch_after_a_later_call() {
    use crate::host::{thaw_tjs_register, thaw_tjs_watch_promise};
    use std::ffi::CString;
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = { "pend/index.js": function(module, exports, require) {
  exports.later = function () { return new Promise(function (r) { globalThis.fire = r; }); };
  exports.fire = function (v) { globalThis.fire(v); };
  exports.later2 = function () { return new Promise(function (_, j) { globalThis.fail = j; }); };
  exports.boom = function (v) { globalThis.fail(new Error(v)); };
  exports.now = function () { return Promise.resolve(1); };
}, };
var __thaw_bundle_require_maps = { "pend/index.js": {}, };
var __thaw_bundle_entry_key = "pend/index.js";
})(globalThis.require);"#;
    let (name, source) = (CString::new("pend").unwrap(), CString::new(bundle).unwrap());
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    // a still-pending promise comes back as its handle, not as "never settled"
    let marker = typed_call("\u{4}pend::later", "[]").expect("pending marker");
    let id: i64 = marker
        .strip_prefix(r#"{"__thaw_tjs_pending__":"#)
        .and_then(|rest| rest.strip_suffix('}'))
        .expect("marker shape")
        .parse()
        .expect("handle id");
    SETTLED.with(|s| s.borrow_mut().clear());
    thaw_tjs_watch_promise(id, std::ptr::null_mut(), record_settle);
    // nothing settles until a later thaw-js call resolves the promise; the call drains the watch
    assert!(SETTLED.with(|s| s.borrow().is_empty()));
    typed_call("pend::fire", "[42]").expect("fire");
    assert_eq!(
        SETTLED.with(|s| s.borrow().clone()),
        vec![(Some("42".to_string()), None)]
    );
    // a rejection settles with the framed Error
    let marker = typed_call("\u{4}pend::later2", "[]").expect("pending marker");
    let id: i64 = marker
        .strip_prefix(r#"{"__thaw_tjs_pending__":"#)
        .and_then(|rest| rest.strip_suffix('}'))
        .expect("marker shape")
        .parse()
        .expect("handle id");
    SETTLED.with(|s| s.borrow_mut().clear());
    thaw_tjs_watch_promise(id, std::ptr::null_mut(), record_settle);
    typed_call("pend::boom", r#"["boom"]"#).expect("boom");
    let settled = SETTLED.with(|s| s.borrow().clone());
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].0, None);
    assert!(
        settled[0]
            .1
            .as_deref()
            .is_some_and(|e| e.starts_with("\u{1}Error\u{1}boom")),
        "{settled:?}"
    );
    // an already-resolved promise is awaited as before: the marker never appears
    assert_eq!(typed_call("\u{4}pend::now", "[]"), Ok("1".into()));
}

#[test]
fn pending_promise_settles_in_the_event_loop_without_a_later_call() {
    use crate::host::{thaw_tjs_register, thaw_tjs_run_event_loop, thaw_tjs_watch_promise};
    use std::ffi::CString;
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = { "loop/index.js": function(module, exports, require) {
  exports.later = function () { return new Promise(function (r) { globalThis.waiters = (globalThis.waiters || []).concat([r]); }); };
  exports.arm = function (v) { setTimeout(function () { globalThis.waiters.shift()(v); }, 0); };
}, };
var __thaw_bundle_require_maps = { "loop/index.js": {}, };
var __thaw_bundle_entry_key = "loop/index.js";
})(globalThis.require);"#;
    let (name, source) = (CString::new("loop").unwrap(), CString::new(bundle).unwrap());
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    let marker = typed_call("\u{4}loop::later", "[]").expect("pending marker");
    let id: i64 = marker
        .strip_prefix(r#"{"__thaw_tjs_pending__":"#)
        .and_then(|rest| rest.strip_suffix('}'))
        .expect("marker shape")
        .parse()
        .expect("handle id");
    SETTLED.with(|s| s.borrow_mut().clear());
    thaw_tjs_watch_promise(id, std::ptr::null_mut(), record_settle);
    // a timer resolves the promise during the loop, after the last call: only the loop can settle the watch
    typed_call("loop::arm", "[7]").expect("arm");
    thaw_tjs_run_event_loop();
    assert_eq!(
        SETTLED.with(|s| s.borrow().clone()),
        vec![(Some("7".to_string()), None)]
    );
}

thread_local! {
    /// Set once `chain_settle` has issued its follow-up watched promise.
    static CHAINED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Records like `record_settle`, and on the first settle starts a second pending promise, arms its timer
/// and watches it, as compiled code does when a continuation awaits again.
extern "C" fn chain_settle(
    context: *mut std::ffi::c_void,
    json: *const std::ffi::c_char,
    error: *const std::ffi::c_char,
) {
    use crate::host::thaw_tjs_watch_promise;
    record_settle(context, json, error);
    if CHAINED.with(|c| c.replace(true)) {
        return;
    }
    let marker = typed_call("\u{4}loop2::later", "[]").expect("second pending marker");
    let id: i64 = marker
        .strip_prefix(r#"{"__thaw_tjs_pending__":"#)
        .and_then(|rest| rest.strip_suffix('}'))
        .expect("marker shape")
        .parse()
        .expect("handle id");
    typed_call("loop2::arm", "[8]").expect("second arm");
    thaw_tjs_watch_promise(id, std::ptr::null_mut(), record_settle);
}

#[test]
fn watch_registered_by_a_settle_callback_settles_in_the_same_event_loop() {
    use crate::host::{thaw_tjs_register, thaw_tjs_run_event_loop, thaw_tjs_watch_promise};
    use std::ffi::CString;
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = { "loop2/index.js": function(module, exports, require) {
  exports.later = function () { return new Promise(function (r) { globalThis.waiters = (globalThis.waiters || []).concat([r]); }); };
  exports.arm = function (v) { setTimeout(function () { globalThis.waiters.shift()(v); }, 0); };
}, };
var __thaw_bundle_require_maps = { "loop2/index.js": {}, };
var __thaw_bundle_entry_key = "loop2/index.js";
})(globalThis.require);"#;
    let (name, source) = (
        CString::new("loop2").unwrap(),
        CString::new(bundle).unwrap(),
    );
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    let marker = typed_call("\u{4}loop2::later", "[]").expect("pending marker");
    let id: i64 = marker
        .strip_prefix(r#"{"__thaw_tjs_pending__":"#)
        .and_then(|rest| rest.strip_suffix('}'))
        .expect("marker shape")
        .parse()
        .expect("handle id");
    SETTLED.with(|s| s.borrow_mut().clear());
    CHAINED.with(|c| c.set(false));
    thaw_tjs_watch_promise(id, std::ptr::null_mut(), chain_settle);
    typed_call("loop2::arm", "[1]").expect("arm");
    // the first settle registers a second watch from inside the pump; the loop must keep pumping until both settle
    thaw_tjs_run_event_loop();
    assert_eq!(
        SETTLED.with(|s| s.borrow().clone()),
        vec![(Some("1".to_string()), None), (Some("8".to_string()), None)]
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
fn compiled_callbacks_nested_in_objects_and_arrays_decode_to_callables() {
    use crate::host::{thaw_tjs_register_callback, Native, Package};
    let sum = thaw_tjs_register_callback(sum_adapter as *const _, std::ptr::null(), 3);
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"o/index.js": function(module, exports, require) {
  exports.obj = function (o) { return typeof o.init === 'function' ? o.init(20, 1, undefined) * 2 : -1; };
  exports.arr = function (a) { return typeof a[0].cb === 'function' ? a[0].cb(20, 1, undefined) * 2 : -1; };
},
};
var __thaw_bundle_require_maps = { "o/index.js": {}, };
var __thaw_bundle_entry_key = "o/index.js";
})(globalThis.require);"#;
    let mut p = Package::load(bundle).expect("loads");
    // `{init: marker, n: 1}`: the marker field decodes to a callable next to a plain value
    let object = Native::Object(vec![
        ("init".to_string(), callback_marker(sum)),
        ("n".to_string(), Native::Number(1.0)),
    ]);
    assert_eq!(p.call("obj", &[object]).ok(), Some(Native::Number(42.0)));
    // the same marker one level down, inside an array of objects
    let array = Native::Array(vec![Native::Object(vec![(
        "cb".to_string(),
        callback_marker(sum),
    )])]);
    assert_eq!(p.call("arr", &[array]).ok(), Some(Native::Number(42.0)));
}

#[test]
fn compiled_callback_invoked_from_an_async_function_runs_on_its_coroutine_thread() {
    use crate::host::{thaw_tjs_register_callback, to_value};
    let sum = thaw_tjs_register_callback(sum_adapter as *const _, std::ptr::null(), 3);
    let mut it = crate::Interp::new();
    let cb = to_value(&mut it, &callback_marker(sum));
    // An async body runs on its own coroutine thread up to the first await, so the callback is
    // looked up and dispatched there, not on the thread that registered it.
    let f = it
        .eval_function("(async function (cb) { console.log('sum=' + cb(20, 1, 2)); })")
        .expect("compiles");
    assert!(it.call(&f, &Value::Undefined, &[cb]).is_ok());
    assert_eq!(it.out, vec!["sum=23".to_string()]);
}

/// The compiled prologue's stack check (`sp < slot` is an overflow), registered as the process limit.
static STACK_PROBE_SLOT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// Reports `true` when the calling stack is above the registered limit, else the `RangeError` a
/// compiled function prologue would throw.
unsafe extern "C" fn stack_probe_adapter(
    _context: *const std::ffi::c_void,
    _args: *const std::ffi::c_char,
) -> *mut std::ffi::c_char {
    let marker = 0u8;
    let sp = std::hint::black_box(&marker) as *const u8 as u64;
    if sp < STACK_PROBE_SLOT.load(std::sync::atomic::Ordering::Acquire) {
        thaw_arena::owned_string("\u{2}\u{1}RangeError\u{1}Maximum call stack size exceeded")
    } else {
        thaw_arena::owned_string("true")
    }
}

#[test]
fn compiled_listener_on_a_coroutine_thread_gets_that_threads_stack_limit() {
    use crate::host::{thaw_tjs_register_callback, to_value};
    thaw_arena::stack_guard::thaw_stack_guard_register(STACK_PROBE_SLOT.as_ptr());
    let probe = thaw_tjs_register_callback(stack_probe_adapter as *const _, std::ptr::null(), 0);
    let mut it = crate::Interp::new();
    let cb = to_value(&mut it, &callback_marker(probe));
    // Shaped like `Readable.from`: an async function driving an async generator, so the callback
    // runs on a coroutine thread whose stack lies outside the registered process limit.
    let f = it
        .eval_function(
            "(async function (cb) { async function* gen() { yield 'done'; } \
             for await (const v of gen()) { try { cb(v); console.log('ok'); } \
             catch (e) { console.log(e.name + ':' + e.message); } } })",
        )
        .expect("compiles");
    assert!(it.call(&f, &Value::Undefined, &[cb]).is_ok());
    it.run_event_loop().ok().unwrap();
    assert_eq!(it.out, vec!["ok".to_string()]);
}

const CO_EXPORTS: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"p/index.js": function(module, exports, require) {
  exports.val = 5;
  exports.run = async function (cb) { return cb(); };
},
};
var __thaw_bundle_require_maps = { "p/index.js": {}, };
var __thaw_bundle_entry_key = "p/index.js";
})(globalThis.require);"#;

static CO_PROBE: std::sync::Mutex<Option<Result<String, String>>> = std::sync::Mutex::new(None);

/// A callback that compiled code makes from an async body, so it runs on that body's coroutine thread.
unsafe extern "C" fn co_probe_adapter(
    _context: *const std::ffi::c_void,
    _args: *const std::ffi::c_char,
) -> *mut std::ffi::c_char {
    *CO_PROBE.lock().unwrap() = Some(class_call("$value$co::val$arity0", "[]", 0, false));
    thaw_arena::owned_string("null")
}

#[test]
fn class_ops_from_a_callback_on_a_coroutine_thread_see_the_host_packages() {
    use crate::host::{thaw_tjs_register, thaw_tjs_register_callback};
    use std::ffi::CString;
    let bundle = CString::new(CO_EXPORTS).unwrap();
    assert_eq!(thaw_tjs_register(c"co".as_ptr(), bundle.as_ptr()), 1);
    let probe = thaw_tjs_register_callback(co_probe_adapter as *const _, std::ptr::null(), 0);
    let args = format!("[{{\"__thaw_tjs_cb__\":{probe}}}]");
    assert_eq!(typed_call("co::run", &args), Ok("null".into()));
    assert_eq!(CO_PROBE.lock().unwrap().take(), Some(Ok("5".into())));
}

/// A string built in an async body lives in that body's coroutine-thread arena. It is read back
/// on the host after the coroutine thread has exited, so it must survive the thread.
#[test]
fn async_body_string_result_survives_its_coroutine_thread_exit() {
    let mut it = crate::Interp::new();
    let src = "const out = [];
        async function f() { await null; out.push('a' + 'b'); }
        f().then(() => console.log(out.join(',')));
        async function g() { await null; return 'a' + 'b'; }
        g().then(v => console.log(v));";
    assert!(it.run_main(src).is_ok());
    assert_eq!(it.out, ["ab", "ab"]);
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
fn vm_sub_realm_errors_are_framed_as_foreign() {
    use crate::host::{HostError, Package};
    let bundle = MINI.replace(
        "exports.VERSION = '1.0';",
        "exports.sub = function () { var id = __thaw_vm_create(); __thaw_vm_run(id, 'var e = new TypeError(\"x\"); e.code = \"C\"; throw e', 'f', 0, undefined, undefined); };\nexports.here = function () { __thaw_vm_run(-1, 'throw new TypeError(\"y\")', 'f', 0, undefined, undefined); };",
    );
    let mut p = Package::load(&bundle).expect("loads");
    match p.call("sub", &[]) {
        Err(HostError::Thrown(display)) => {
            assert_eq!(display, "TypeError: x");
            assert_eq!(
                frame_text(&display),
                "\u{1}\u{1e}TypeError\u{1f}\u{18}\u{1}x\u{3}C\u{5}{\"code\":\"C\"}"
            );
        }
        other => panic!("{other:?}"),
    }
    match p.call("here", &[]) {
        Err(HostError::Thrown(display)) => {
            assert_eq!(frame_text(&display), "\u{1}TypeError\u{1}y");
        }
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

#[test]
fn a_plain_object_held_by_a_handle_comes_back_as_the_same_handle() {
    use crate::host::thaw_tjs_register;
    use std::ffi::CString;
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = { "h/index.js": function(module, exports, require) {
  exports.make = function () { return { a: 1 }; };
  exports.identity = function (v) { return v; };
}, };
var __thaw_bundle_require_maps = { "h/index.js": {}, };
var __thaw_bundle_entry_key = "h/index.js";
})(globalThis.require);"#;
    let (name, source) = (CString::new("held").unwrap(), CString::new(bundle).unwrap());
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    // `\u{2}` asks for the result as a handle id, which compiled code then holds
    let held: u64 = typed_call("\u{2}held::make", "[]")
        .unwrap()
        .parse()
        .unwrap();
    let marker = format!("{{\"__thaw_tjs_handle__\":{held}}}");
    // the held object goes in as a live handle and comes back as that same handle
    assert_eq!(
        typed_call("held::identity", &format!("[{marker}]")).unwrap(),
        marker
    );
    // a fresh plain result that compiled code does not hold is still a copy
    assert_eq!(typed_call("held::make", "[]").unwrap(), r#"{"a":1}"#);
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
    // a method returning `this` leases the same handle: one live object, one id
    let same: i64 = class_call("$method$k::Box$add$overload0$arity1", "[2]", id, true)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(same, id);
    assert_eq!(
        class_call("$method$k::Box$get$overload0$arity0", "[]", same, false).unwrap(),
        "42"
    );
    // drop the lease `add` returned: the id still holds the object for `id`
    assert_eq!(crate::host::thaw_tjs_release_handle(same), 1);
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

#[test]
fn with_this_call_and_event_key_handle_match_the_quickjs_contract() {
    use crate::host::*;
    use std::ffi::{CStr, CString};
    let q = |s: &str| CString::new(s).unwrap();
    // `function () { return this.k + arguments[0]; }` called with the leading array element as receiver.
    let made = thaw_tjs_new_function(q(r#"["return this.k + arguments[0]"]"#).as_ptr());
    assert!(made.error.is_null() && made.value > 0);
    let args = q(r#"{"root":{"r":0},"nodes":[{"a":[{"r":1},{"v":2}]},{"o":[["k",{"v":40}]]}]}"#);
    let r = thaw_tjs_call_handle_with_this_graph_wire_result(made.value, args.as_ptr());
    assert!(r.error.is_null());
    // SAFETY: a non-null result is a NUL-terminated wire.
    let wire = unsafe { CStr::from_ptr(r.value) }.to_str().unwrap();
    assert!(wire.contains("42"), "{wire}");
    let empty = q(r#"{"root":{"r":0},"nodes":[{"a":[]}]}"#);
    assert!(
        !thaw_tjs_call_handle_with_this_graph_wire_result(made.value, empty.as_ptr())
            .error
            .is_null()
    );

    let key = thaw_tjs_event_key_handle_result(q(r#"["evt"]"#).as_ptr(), 0);
    assert!(key.error.is_null() && key.value > 0);
    // SAFETY: a NUL-terminated string owned by the handle table.
    let text = unsafe { CStr::from_ptr(thaw_tjs_handle_to_string(key.value)) }
        .to_str()
        .unwrap();
    assert_eq!(text, "evt");
    assert!(thaw_tjs_event_key_handle_result(q(r#"[null]"#).as_ptr(), 0)
        .error
        .is_null());
    assert!(!thaw_tjs_event_key_handle_result(q(r#"[1]"#).as_ptr(), 0)
        .error
        .is_null());
    assert!(!thaw_tjs_event_key_handle_result(q(r#"["s"]"#).as_ptr(), 1)
        .error
        .is_null());
}

#[test]
fn user_dynamic_code_runs_in_a_bare_realm() {
    use crate::host::*;
    use std::ffi::{CStr, CString};
    let q = |s: &str| CString::new(s).unwrap();
    let made = thaw_tjs_new_function(q(r#"["a","b","return a * b"]"#).as_ptr());
    assert!(made.error.is_null() && made.value > 0);
    let args = q(r#"{"root":{"r":0},"nodes":[{"a":[{"v":6},{"v":7}]}]}"#);
    let product = thaw_tjs_call_handle_graph_result(made.value, args.as_ptr());
    assert!(product.error.is_null());
    // SAFETY: a non-null result is a NUL-terminated wire.
    let wire = unsafe { CStr::from_ptr(product.value) }.to_str().unwrap();
    assert!(wire.contains("42"), "{wire}");
    let args = q(r#"{"root":{"r":0},"nodes":[{"a":[{"v":"1 + 2"}]}]}"#);
    let evaluated = thaw_tjs_call_graph_result(q("eval").as_ptr(), args.as_ptr());
    assert!(evaluated.error.is_null());
    // SAFETY: as above.
    let wire = unsafe { CStr::from_ptr(evaluated.value) }.to_str().unwrap();
    assert!(wire.contains('3'), "{wire}");
    let bad = thaw_tjs_new_function(q(r#"["return ("]"#).as_ptr());
    assert!(!bad.error.is_null());
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

#[test]
fn namespace_globals_carry_to_string_tag_and_inspect_as_tagged_objects() {
    assert_eq!(
        one("console.log(Object.prototype.toString.call(Math), Object.prototype.toString.call(JSON), Object.prototype.toString.call(Reflect))"),
        "[object Math] [object JSON] [object Reflect]"
    );
    assert_eq!(
        one("console.log(Math, JSON, Reflect)"),
        "Object [Math] {} Object [JSON] {} Object [Reflect] {}"
    );
    assert_eq!(one("console.log([Math])"), "[ Object [Math] {} ]");
    assert_eq!(
        one("console.log([1, 'a', [2, { b: 3 }]])"),
        "[ 1, 'a', [ 2, { b: 3 } ] ]"
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
fn buffer_concat_accepts_an_omitted_or_nan_total_and_byte_swaps() {
    assert_eq!(
        one("var a = Buffer.from('a'), b = Buffer.from('b');
             console.log(Buffer.concat([a, b]).toString(), Buffer.concat([a, b], NaN).toString(),
               Buffer.concat([a, b], 1).toString(),
               Buffer.from([0, 1, 2, 3]).swap16().toString('hex'),
               Buffer.from([0, 1, 2, 3]).swap32().toString('hex'),
               Buffer.from([0, 1, 2, 3, 4, 5, 6, 7]).swap64().toString('hex'))"),
        "ab ab a 01000302 03020100 0706050403020100"
    );
}

#[test]
fn buffer_byte_swaps_reject_partial_units() {
    assert_eq!(
        one("try { Buffer.from([1, 2, 3]).swap16(); } catch (e) { console.log(e.name, e.message); }
             try { Buffer.from([1, 2, 3, 4, 5]).swap32(); } catch (e) { console.log(e.name, e.message); }
             try { Buffer.alloc(9).swap64(); } catch (e) { console.log(e.name, e.message); }"),
        "RangeError Buffer size must be a multiple of 16-bits|RangeError Buffer size must be a multiple of 32-bits|RangeError Buffer size must be a multiple of 64-bits"
    );
}

#[test]
fn buffer_concat_accepts_items_in_the_tagged_abi_shape() {
    assert_eq!(
        one("console.log(Buffer.concat([{ type: 'Buffer', data: [97] }, { type: 'Buffer', data: [98, 99] }]).toString())"),
        "abc"
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

// Dynamic calls on a native `http.Server` pick Node's overload by method name and argument shape.
#[test]
fn native_server_overloads_are_selected_by_method_name_and_arguments() {
    use crate::host::*;
    use std::ffi::{c_char, CStr, CString};
    let q = |t: &str| CString::new(t).unwrap();
    let text = |p: *mut c_char| unsafe { CStr::from_ptr(p).to_str().unwrap().to_string() };
    let src = r#"
      function tagged(tag) { return function () { return 'ran:' + tag; }; }
      globalThis.srv = { listen: tagged('plain'), __listenWithCallback: tagged('withCallback'),
        __listenWithHostCallback: tagged('host'), close: tagged('plainClose'),
        __closeWithCallback: tagged('closeCallback'), on: tagged('plainOn'), __onError: tagged('onError') };
      globalThis.bare = { listen: tagged('plain'), close: tagged('plainClose'), on: tagged('plainOn') };
      globalThis.cbFn = function () {};
    "#;
    assert_eq!(thaw_tjs_load_script(q(src).as_ptr()), 1);
    // `items` is the argument array body; node 1 is a fresh callback handle (`{"r":1}` in the items).
    // A decoded wire releases its handle lease, so each call takes its own handle.
    let wire = |items: &str| {
        let cb = thaw_tjs_get_global(q("cbFn").as_ptr()) as u64;
        assert!(cb > 0);
        format!(r#"{{"root":{{"r":0}},"nodes":[{{"a":[{items}]}},{{"hdl":{cb}}}]}}"#)
    };
    let cases = [
        (
            "srv",
            "listen",
            r#"{"v":0},{"v":"127.0.0.1"},{"r":1}"#,
            "host",
        ),
        ("srv", "listen", r#"{"v":0},{"r":1}"#, "withCallback"),
        ("srv", "close", r#"{"r":1}"#, "closeCallback"),
        ("srv", "on", r#"{"v":"error"},{"r":1}"#, "onError"),
        // negatives: one-argument listen, a non-error event, no-arg close, and an object without the `__` fields
        ("srv", "listen", r#"{"v":0}"#, "plain"),
        ("srv", "on", r#"{"v":"data"},{"r":1}"#, "plainOn"),
        ("srv", "close", "", "plainClose"),
        (
            "bare",
            "listen",
            r#"{"v":0},{"v":"127.0.0.1"},{"r":1}"#,
            "plain",
        ),
        ("bare", "close", r#"{"r":1}"#, "plainClose"),
    ];
    for (global, name, items, tag) in cases {
        let receiver = thaw_tjs_get_global(q(global).as_ptr()) as u64;
        assert!(receiver > 0, "no global {global}");
        let method = thaw_tjs_get_property_result(receiver, q(name).as_ptr());
        assert!(method.error.is_null());
        let args = wire(items);
        let call = thaw_tjs_call_selected_method_graph_result(
            receiver,
            method.value,
            q(name).as_ptr(),
            q(&args).as_ptr(),
        );
        assert!(call.error.is_null(), "{}", text(call.error));
        let out = text(call.value);
        assert!(
            out.contains(&format!("\"ran:{tag}\"")),
            "{global}.{name}({items}) ran the wrong field: {out}"
        );
        // the call_method variant (name + property lookup) must select the same field
        let args = wire(items);
        let via_name =
            thaw_tjs_call_method_graph_result(receiver, q(name).as_ptr(), q(&args).as_ptr());
        assert!(via_name.error.is_null(), "{}", text(via_name.error));
        assert!(
            text(via_name.value).contains(&format!("\"ran:{tag}\"")),
            "{global}.{name}: {}",
            text(via_name.value)
        );
    }
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
    // generated `with (..)` source is supported (sloppy `new Function`), no longer a blocker
    let with_code = src.replace(
        "exports.a = ",
        "exports.w = '  with (locals || {}) {'; exports.a = ",
    );
    assert!(crate::scan::scan_bundle(&with_code).unwrap().is_clean());
}

#[test]
fn sloppy_with_statement_via_new_function() {
    assert_eq!(
        one("var f = new Function('o', 'with (o) { b = a + 1; return typeof a + typeof zz + say(b) }'); \
             var o = { a: 1, b: 0, say(x) { return this === o ? 'T' + x : 'F' } }; \
             console.log(f(o), o.b)"),
        "numberundefinedT2 2"
    );
    // outer names still resolve, a local shadows nothing the object lacks
    assert_eq!(
        one("var y = 5; var f = new Function('o', 'with (o) { return a + y }'); console.log(f({ a: 1 }))"),
        "6"
    );
    // Symbol.unscopables hides a property
    assert_eq!(
        one(
            "var x = 'outer'; var o = { x: 'in', [Symbol.unscopables]: { x: true } }; \
             console.log(new Function('o', 'with (o) { return x }').call(null, o))"
        ),
        "outer"
    );
}

#[test]
fn dynamic_code_parse_errors_are_syntax_errors() {
    assert_eq!(
        one("try { new Function('}') } catch (e) { console.log(e instanceof SyntaxError, e.name, !!e.message) }"),
        "true SyntaxError true"
    );
    assert_eq!(
        one(
            "try { Function('a b', 'return') } catch (e) { console.log(e instanceof SyntaxError) }"
        ),
        "true"
    );
    assert_eq!(
        one("const ev = eval; try { ev('var = ;') } catch (e) { console.log(e instanceof SyntaxError, !!e.message) }"),
        "true true"
    );
    // valid dynamic code still runs
    assert_eq!(
        one("console.log(new Function('a', 'b', 'return a + b')(2, 3))"),
        "5"
    );
    // a construct the evaluator does not run stays Unsupported, not a SyntaxError
    assert_eq!(
        one("new Function('return import(\"x\")')()"),
        "unsupported: import()"
    );
}

#[test]
fn string_literal_with_text_is_not_code() {
    assert_eq!(one("console.log('with (x) {')"), "with (x) {");
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

const COLLECTIONS: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"c/index.js": function(module, exports, require) {
  exports.describe = function (m) { return m instanceof Map ? "map:" + m.size + ":" + m.get("a") : "?"; };
  exports.setd = function (s) { return s instanceof Set ? "set:" + s.size : "?"; };
  exports.nested = function (m) { return m instanceof Map && m.get("k") instanceof Date ? "date" : "?"; };
},
};
var __thaw_bundle_require_maps = { "c/index.js": {}, };
var __thaw_bundle_entry_key = "c/index.js";
})(globalThis.require);"#;

#[test]
fn a_map_and_set_cross_the_typed_abi_as_wire_nodes() {
    use std::ffi::CString;
    let bundle = CString::new(COLLECTIONS).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"cm".as_ptr(), bundle.as_ptr()),
        1
    );
    assert_eq!(
        typed_call("cm::describe", r#"[{"__thaw_tjs_map__":[["a",1]]}]"#).unwrap(),
        "\"map:1:1\""
    );
    assert_eq!(
        typed_call("cm::setd", r#"[{"__thaw_tjs_set__":[2,3]}]"#).unwrap(),
        "\"set:2\""
    );
    // a Date value inside a Map entry is still revived as a real Date
    assert_eq!(
        typed_call(
            "cm::nested",
            r#"[{"__thaw_tjs_map__":[["k",{"__thaw_tjs_date__":5}]]}]"#
        )
        .unwrap(),
        "\"date\""
    );
}

const BIG_AND_ZERO: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"bz/index.js": function(module, exports, require) {
  exports.show = function (x) { return typeof x + ":" + String(x); };
  exports.neg = function (x) { return Object.is(x, -0); };
},
};
var __thaw_bundle_require_maps = { "bz/index.js": {}, };
var __thaw_bundle_entry_key = "bz/index.js";
})(globalThis.require);"#;

const NON_FINITE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"nf/index.js": function(module, exports, require) {
  exports.kind = function (x) { return Number.isNaN(x) ? "nan" : x === Infinity ? "inf" : x === -Infinity ? "-inf" : "finite:" + x; };
  exports.opts = function (o) { return String(o.breakLength); };
},
};
var __thaw_bundle_require_maps = { "nf/index.js": {}, };
var __thaw_bundle_entry_key = "nf/index.js";
})(globalThis.require);"#;

#[test]
fn non_finite_numbers_cross_the_typed_abi_as_wire_nodes() {
    use std::ffi::CString;
    let bundle = CString::new(NON_FINITE).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"nf".as_ptr(), bundle.as_ptr()),
        1
    );
    assert_eq!(
        typed_call("nf::kind", r#"[{"__thaw_tjs_num__":"NaN"}]"#).unwrap(),
        "\"nan\""
    );
    assert_eq!(
        typed_call("nf::kind", r#"[{"__thaw_tjs_num__":"Infinity"}]"#).unwrap(),
        "\"inf\""
    );
    assert_eq!(
        typed_call("nf::kind", r#"[{"__thaw_tjs_num__":"-Infinity"}]"#).unwrap(),
        "\"-inf\""
    );
    assert_eq!(
        typed_call(
            "nf::opts",
            r#"[{"breakLength":{"__thaw_tjs_num__":"Infinity"}}]"#
        )
        .unwrap(),
        "\"Infinity\""
    );
}

#[test]
fn a_bigint_and_negative_zero_cross_the_typed_abi_as_wire_nodes() {
    use std::ffi::CString;
    let bundle = CString::new(BIG_AND_ZERO).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"bz".as_ptr(), bundle.as_ptr()),
        1
    );
    assert_eq!(
        typed_call(
            "bz::show",
            r#"[{"__thaw_tjs_bigint__":"12345678901234567890"}]"#
        )
        .unwrap(),
        "\"bigint:12345678901234567890\""
    );
    assert_eq!(
        typed_call("bz::neg", r#"[{"__thaw_tjs_num__":"-0"}]"#).unwrap(),
        "true"
    );
    assert_eq!(typed_call("bz::neg", "[0]").unwrap(), "false");
}

const CONTEXTS: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"cx/index.js": function(module, exports, require) {
  var seen = new WeakSet();
  exports.create = function (o) { var c = o === undefined ? {} : o; seen.add(c); return c; };
  exports.has = function (c) { return seen.has(c); };
},
};
var __thaw_bundle_require_maps = { "cx/index.js": {}, };
var __thaw_bundle_entry_key = "cx/index.js";
})(globalThis.require);"#;

#[test]
fn a_returned_object_keeps_its_identity_only_behind_a_handle() {
    use std::ffi::CString;
    let bundle = CString::new(CONTEXTS).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"cx".as_ptr(), bundle.as_ptr()),
        1
    );
    // `\u{2}` = a JsValue result: the object stays in the thaw-js heap, the caller gets its handle id
    let id: i64 = typed_call("\u{2}cx::create", "[{\"a\":1}]")
        .unwrap()
        .parse()
        .unwrap();
    let marker = format!("[{{\"__thaw_tjs_handle__\":{id}}}]");
    assert_eq!(typed_call("cx::has", &marker).unwrap(), "true");
    // a plain-data (Json) result is a copy: the realm does not recognise it when it comes back
    assert_eq!(
        typed_call("cx::create", "[{\"a\":1}]").unwrap(),
        "{\"a\":1}"
    );
    assert_eq!(typed_call("cx::has", "[{\"a\":1}]").unwrap(), "false");
    assert_eq!(crate::host::thaw_tjs_release_handle(id), 1);
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

/// Run `src` through the full event loop (child processes are polled there) and return the log lines.
fn run_loop(src: &str) -> String {
    let mut it = Interp::new();
    it.run(src).expect("script runs");
    it.run_event_loop().expect("loop runs");
    it.out.join("|")
}

const CHILD_SHIM: &str = r#"
function hexs(h) { var s = ''; for (var i = 0; i < h.length; i += 2) s += String.fromCharCode(parseInt(h.slice(i, i + 2), 16)); return s; }
var logs = [];
globalThis.__thaw_poll_platform_events = function () {
  JSON.parse(__thaw_child_process_poll()).forEach(function (e) {
    logs.push(e.type + (e.value !== undefined ? ':' + hexs(e.value) : '') + (e.type === 'exit' || e.type === 'exited' ? ':' + e.code + '/' + e.signal : '') + (e.code && e.type === 'error' ? ':' + e.code : ''));
  });
};
"#;

#[test]
fn child_process_async_orders_output_before_close() {
    let src = format!(
        "{CHILD_SHIM}
__thaw_child_process_spawn('/bin/sh', JSON.stringify(['-c', 'echo out; echo err >&2; exit 3']), JSON.stringify({{stdio: ['ignore','pipe','pipe']}}));
process.on('exit', function () {{ console.log(logs.join(',')); }});"
    );
    let out = run_loop(&src);
    assert!(out.starts_with("spawn,"), "{out}");
    assert!(
        out.contains("stdout:out|") && out.contains("stderr:err|"),
        "{out}"
    );
    // `exited` (the process ended) may precede the last pipe data; `exit` (close) always comes last.
    assert!(out.ends_with(",exit:3/null"), "{out}");
    let exited = out.find("exited:3/null").expect("exited event");
    assert!(exited < out.rfind(",exit:3/null").unwrap(), "{out}");
}

#[test]
fn child_process_keeps_loop_alive_and_reports_signal_and_spawn_error() {
    let src = format!(
        "{CHILD_SHIM}
var h = __thaw_child_process_spawn('/bin/sleep', JSON.stringify(['5']), JSON.stringify({{}}));
setTimeout(function () {{ console.log(__thaw_child_process_kill(h, 15)); }}, 50);
__thaw_child_process_spawn('/nonexistent/x', '[]', '{{}}');
process.on('exit', function () {{ console.log(logs.join(',')); }});"
    );
    let out = run_loop(&src);
    assert!(out.starts_with("true|"), "{out}");
    assert!(
        out.contains("error:ENOENT") && out.contains("exit:null/15"),
        "{out}"
    );
}

/// The v3 shape: a realm WITHOUT the child_process hook starts a child (its own native spawn) while the hook lives in
/// another realm. The loop enters the first-registered realm first, so before the fix it spun forever.
#[test]
fn child_spawn_from_a_realm_without_the_hook_is_drained_by_the_hook_realm() {
    let code = run_loop_or_fail(|| {
        register_exit_package(
            "cpb",
            "exports.go = function () { __thaw_child_process_spawn('/nonexistent/x', '[]', '{}'); };",
        );
        register_exit_package(
            "cpa",
            &format!(
                "{CHILD_SHIM}
                 var inner = globalThis.__thaw_poll_platform_events;
                 globalThis.__thaw_poll_platform_events = function () {{
                   inner();
                   if (logs.indexOf('error:ENOENT') >= 0) process.exitCode = 0;
                 }};
                 process.exitCode = 5;"
            ),
        );
        typed_call("cpb::go", "[]").expect("spawn");
    });
    assert_eq!(code, 0);
}

#[test]
fn child_process_stdin_env_cwd_and_sync() {
    let src = format!(
        "{CHILD_SHIM}
var h = __thaw_child_process_spawn('/bin/cat', '[]', JSON.stringify({{cwd: '/'}}));
__thaw_child_process_stdin(h, '68690a', false);
__thaw_child_process_stdin(h, '', true);
var r = JSON.parse(__thaw_child_process_sync('/bin/sh', JSON.stringify(['-c', 'echo $FOO; pwd; cat; exit 2']), JSON.stringify({{env: {{FOO: 'bar'}}, cwd: '/tmp', input: '78'}})));
console.log(r.status, hexs(r.stdout).replace(/\\n/g, '+'));
var t = JSON.parse(__thaw_child_process_sync('/bin/sleep', JSON.stringify(['5']), JSON.stringify({{timeout: 50, killSignal: 9}})));
console.log(t.timedOut, t.signal);
setTimeout(function () {{ console.log(logs.join(',')); }}, 300);"
    );
    let out = run_loop(&src);
    assert!(out.starts_with("2 bar+/tmp+x|true 9|"), "{out}");
    assert!(out.contains("stdout:hi|"), "{out}");
}

#[test]
fn scan_knows_fetch_globals_and_tolerates_service_worker_listener() {
    let blocked = MINI.replace(
        "exports.VERSION = '1.0';",
        "exports.F = () => { addEventListener('fetch', () => {}); return [new Request('http://a/'), new Response(''), new Headers(), new FormData(), new TransformStream(), new TextEncoderStream(), crypto.subtle.digest]; };",
    );
    let r = crate::scan::scan_bundle(&blocked).ok().unwrap();
    assert!(r.is_clean(), "{:?}", r.blockers);
}

#[test]
fn in_operator_sees_array_length() {
    assert_eq!(
        one("console.log('length' in [1,2], 'length' in [], 1 in [5,6], 2 in [5,6], 'x' in [])"),
        "true true true false false"
    );
}

const CHEERIO_UNDICI: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"cheerio/index.js": function(module, exports, require) {
  var undici = require('undici');
  exports.ok = function () { return 1; };
  exports.fromURL = function () { return new undici.Client('http://a'); };
},
"undici/index.js": function(module, exports, require) { WebAssembly.compile; new MessageChannel(); },
};
var __thaw_bundle_require_maps = { "cheerio/index.js": {"undici": "undici/index.js"}, "undici/index.js": {}, };
var __thaw_bundle_entry_key = "cheerio/index.js";
})(globalThis.require);"#;

#[test]
fn cheerio_undici_is_stubbed_with_a_clear_error() {
    // the scan no longer sees undici's WebAssembly/MessageChannel use ...
    let r = crate::scan::scan_bundle(CHEERIO_UNDICI).ok().unwrap();
    assert!(r.is_clean(), "{:?}", r.blockers);
    // ... the rest of the package loads and runs, and fromURL fails loudly instead of silently
    let mut p = crate::host::Package::load(CHEERIO_UNDICI).unwrap();
    assert!(p.call("ok", &[]).is_ok());
    let e = format!("{:?}", p.call("fromURL", &[]));
    assert!(e.contains("undici is not available"), "{e}");
}

// ----- graph callbacks and awaited Promises -----

const GRAPH_BUNDLE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"gr/index.js": function(module, exports, require) {
  exports.echo = function (f) { var o = { k: 3 }; return f(o) === o; };
  exports.number = function (f) { return f({ k: 1 }) + 1; };
  exports.later = function () { return new Promise(function (r) { setTimeout(function () { r(42); }, 1); }); };
  exports.laterThrow = function () { return new Promise(function (_, j) { setTimeout(function () { j(new Error('late')); }, 1); }); };
},
};
var __thaw_bundle_require_maps = { "gr/index.js": {}, };
var __thaw_bundle_entry_key = "gr/index.js";
})(globalThis.require);"#;

/// A graph-callback adapter: consumes the `[this, object]` wire like `thaw_json_graph_decode`, then
/// answers with the object lease it received (identity echo) or the number 7.
unsafe extern "C" fn graph_adapter(
    context: *const std::ffi::c_void,
    wire: *const std::ffi::c_char,
) -> *mut std::ffi::c_char {
    let text = unsafe { std::ffi::CStr::from_ptr(wire) }
        .to_str()
        .unwrap()
        .to_string();
    assert!(unsafe { thaw_arena::take_owned_graph_wire(wire) });
    let id = text
        .split("\"hdl\":")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .and_then(|n| n.parse::<i64>().ok())
        .expect("the object argument is a handle lease");
    let echo = !context.is_null();
    let answer = if echo {
        crate::host::thaw_tjs_retain_handle(id);
        format!("{{\"root\":{{\"r\":0}},\"nodes\":[{{\"hdl\":{id}}}],\"leases\":[{id}],\"napiLeases\":[]}}")
    } else {
        "{\"root\":{\"v\":7},\"nodes\":[],\"leases\":[],\"napiLeases\":[]}".to_string()
    };
    crate::host::thaw_tjs_release_handle(id);
    let ptr = thaw_arena::owned_string(answer);
    assert!(unsafe { thaw_arena::register_owned_graph_wire(ptr) });
    ptr
}

#[test]
fn graph_callbacks_receive_live_objects_and_return_graph_wires() {
    use crate::host::*;
    let source = std::ffi::CString::new(GRAPH_BUNDLE).unwrap();
    assert_eq!(thaw_tjs_register(c"gr".as_ptr(), source.as_ptr()), 1);
    let handle_of = |context: *const std::ffi::c_void| {
        let made = thaw_tjs_register_native_callback_graph(
            graph_adapter as *const _,
            context,
            0,
            1,
            0,
            std::ptr::null(),
            0,
        );
        assert!(made.error.is_null() && made.value > 0);
        made.value
    };
    let echo = handle_of(std::ptr::dangling());
    let number = handle_of(std::ptr::null());
    let call = |export: &str, callback: u64| {
        let function = thaw_tjs_get_global(
            std::ffi::CString::new(format!("gr::{export}"))
                .unwrap()
                .as_ptr(),
        );
        assert!(function > 0);
        let wire = std::ffi::CString::new(format!(
            "{{\"root\":{{\"r\":0}},\"nodes\":[{{\"a\":[{{\"r\":1}}]}},{{\"hdl\":{callback}}}]}}"
        ))
        .unwrap();
        thaw_tjs_retain_handle(callback as i64);
        let result =
            thaw_tjs_call_handle_handle_graph_args_result(function as u64, wire.as_ptr(), false);
        assert!(result.error.is_null(), "{export}");
        result.value
    };
    assert_eq!(
        unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(call("echo", echo))) }
            .to_str()
            .unwrap(),
        "true"
    );
    assert_eq!(
        unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(call("number", number))) }
            .to_str()
            .unwrap(),
        "8"
    );
}

const MASK_BUNDLE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"mk/index.js": function(module, exports, require) {
  exports.make = function () { return new Error('boom'); };
},
};
var __thaw_bundle_require_maps = { "mk/index.js": {}, };
var __thaw_bundle_entry_key = "mk/index.js";
})(globalThis.require);"#;

thread_local! {
    /// What `mask_adapter` saw per call: the wire text and `String(value)` of each handle lease it carried.
    static MASK_SEEN: std::cell::RefCell<Vec<(String, Vec<String>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A graph-callback adapter that records its wire, reads each handle lease the way the compiled side
/// would, then releases it. It answers `undefined`.
unsafe extern "C" fn mask_adapter(
    _context: *const std::ffi::c_void,
    wire: *const std::ffi::c_char,
) -> *mut std::ffi::c_char {
    let text = unsafe { std::ffi::CStr::from_ptr(wire) }
        .to_str()
        .unwrap()
        .to_string();
    assert!(unsafe { thaw_arena::take_owned_graph_wire(wire) });
    let mut shown = Vec::new();
    for part in text.split("\"hdl\":").skip(1) {
        let id: i64 = part.split('}').next().unwrap().parse().unwrap();
        let value = crate::host::thaw_tjs_handle_to_string(id as u64);
        shown.push(
            unsafe { std::ffi::CStr::from_ptr(value) }
                .to_str()
                .unwrap()
                .to_string(),
        );
        unsafe { thaw_arena::destroy_string(value) };
        crate::host::thaw_tjs_release_handle(id);
    }
    MASK_SEEN.with(|s| s.borrow_mut().push((text, shown)));
    let ptr = thaw_arena::owned_string(
        "{\"root\":{\"u\":1},\"nodes\":[],\"leases\":[],\"napiLeases\":[]}",
    );
    assert!(unsafe { thaw_arena::register_owned_graph_wire(ptr) });
    ptr
}

fn mask_callback(mask: u64) -> u64 {
    let made = crate::host::thaw_tjs_register_native_callback_graph(
        mask_adapter as *const _,
        std::ptr::null(),
        mask,
        1,
        0,
        std::ptr::null(),
        0,
    );
    assert!(made.error.is_null() && made.value > 0);
    made.value
}

/// Call the callback `callback` with one argument given as wire nodes `nodes` (root = node 0's array).
fn call_mask_callback(callback: u64, nodes: &str) {
    let wire =
        std::ffi::CString::new(format!("{{\"root\":{{\"r\":0}},\"nodes\":[{nodes}]}}")).unwrap();
    let result =
        crate::host::thaw_tjs_call_handle_handle_graph_args_result(callback, wire.as_ptr(), false);
    assert!(result.error.is_null());
}

fn seen_calls() -> Vec<(String, Vec<String>)> {
    MASK_SEEN.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

#[test]
fn graph_mask_passes_primitive_jsvalue_arguments_as_handles() {
    use crate::host::*;
    let source = std::ffi::CString::new(MASK_BUNDLE).unwrap();
    assert_eq!(thaw_tjs_register(c"mk".as_ptr(), source.as_ptr()), 1);
    // bit 0: the callback's first declared parameter is a JsValue
    let callback = mask_callback(1);
    for (arg, shown) in [
        (r#"{"v":null}"#, "null"),
        (r#"{"u":1}"#, "undefined"),
        (r#"{"v":"hello"}"#, "hello"),
    ] {
        call_mask_callback(callback, &format!(r#"{{"a":[{arg}]}}"#));
        let seen = seen_calls();
        assert_eq!(seen.len(), 1);
        // items: [this, arg]; `this` stays inline, the argument is a handle lease
        assert!(
            seen[0].0.contains(r#""a":[{"u":1},{"r":1}]"#),
            "{}",
            seen[0].0
        );
        assert_eq!(seen[0].1, [shown]);
    }
}

#[test]
fn graph_mask_zero_keeps_primitive_arguments_inline() {
    use crate::host::*;
    let source = std::ffi::CString::new(MASK_BUNDLE).unwrap();
    assert_eq!(thaw_tjs_register(c"mk".as_ptr(), source.as_ptr()), 1);
    let callback = mask_callback(0);
    call_mask_callback(callback, r#"{"a":[{"v":null}]}"#);
    let seen = seen_calls();
    assert_eq!(
        seen[0].0,
        r#"{"root":{"r":0},"nodes":[{"a":[{"u":1},{"v":null}]}],"leases":[],"napiLeases":[]}"#
    );
    assert!(seen[0].1.is_empty());
}

#[test]
fn graph_mask_frames_error_arguments_at_str_positions() {
    use crate::host::*;
    let source = std::ffi::CString::new(MASK_BUNDLE).unwrap();
    assert_eq!(thaw_tjs_register(c"mk".as_ptr(), source.as_ptr()), 1);
    // `make()` returns a live `Error`; hand it to the callback as a lease
    let make = thaw_tjs_get_global(c"mk::make".as_ptr());
    assert!(make > 0);
    let error = thaw_tjs_call_handle_handle_graph_args_result(
        make as u64,
        c"{\"root\":{\"r\":0},\"nodes\":[{\"a\":[]}]}".as_ptr(),
        false,
    );
    assert!(error.error.is_null() && error.value > 0);
    thaw_tjs_retain_handle(error.value as i64);
    // bit 32: the callback's first declared parameter is a `Str` (an Error arrives framed)
    let callback = mask_callback(1 << 32);
    call_mask_callback(
        callback,
        &format!(r#"{{"a":[{{"r":1}}]}},{{"hdl":{}}}"#, error.value),
    );
    let seen = seen_calls();
    assert!(
        seen[0].0.contains(r#"{"v":"\u0001Error\u0001boom"#),
        "{}",
        seen[0].0
    );
    assert!(seen[0].1.is_empty());
}

#[test]
fn graph_mask_method_variant_counts_the_receiver_as_parameter_zero() {
    use crate::host::*;
    let source = std::ffi::CString::new(MASK_BUNDLE).unwrap();
    assert_eq!(thaw_tjs_register(c"mk".as_ptr(), source.as_ptr()), 1);
    // A method has the receiver as declared parameter 0 (wire item 0), so bit 1 is the first argument.
    let method = |mask: u64| {
        let made = thaw_tjs_register_native_method_callback_graph(
            mask_adapter as *const _,
            std::ptr::null(),
            mask,
            2,
            0,
            std::ptr::null(),
            0,
        );
        assert!(made.error.is_null() && made.value > 0);
        made.value as u64
    };
    call_mask_callback(method(1 << 1), r#"{"a":[{"v":null}]}"#);
    let seen = seen_calls();
    assert!(
        seen[0].0.contains(r#""a":[{"u":1},{"r":1}]"#),
        "{}",
        seen[0].0
    );
    assert_eq!(seen[0].1, ["null"]);
    // bit 0 is the receiver: `this` (undefined here) becomes the lease, the argument stays inline
    call_mask_callback(method(1 << 0), r#"{"a":[{"v":null}]}"#);
    let seen = seen_calls();
    assert!(
        seen[0].0.contains(r#""a":[{"r":1},{"v":null}]"#),
        "{}",
        seen[0].0
    );
    assert_eq!(seen[0].1, ["undefined"]);
}

#[test]
fn graph_mask_pads_missing_masked_argument() {
    use crate::host::*;
    let source = std::ffi::CString::new(MASK_BUNDLE).unwrap();
    assert_eq!(thaw_tjs_register(c"mk".as_ptr(), source.as_ptr()), 1);
    // two declared parameters, only the second is a JsValue; the caller passes one argument (`cb(err)`)
    let made = thaw_tjs_register_native_callback_graph(
        mask_adapter as *const _,
        std::ptr::null(),
        1 << 1,
        2,
        0,
        std::ptr::null(),
        0,
    );
    assert!(made.error.is_null() && made.value > 0);
    call_mask_callback(made.value as u64, r#"{"a":[{"v":"x"}]}"#);
    let seen = seen_calls();
    assert!(
        seen[0].0.contains(r#""a":[{"u":1},{"v":"x"},{"r":1}]"#),
        "{}",
        seen[0].0
    );
    assert_eq!(seen[0].1, ["undefined"]);
}

#[test]
fn exact_calls_await_returned_promises_only_when_resolving() {
    use crate::host::*;
    let source = std::ffi::CString::new(GRAPH_BUNDLE.replace("gr/", "pr/")).unwrap();
    assert_eq!(thaw_tjs_register(c"pr".as_ptr(), source.as_ptr()), 1);
    let wire = c"{\"root\":{\"r\":0},\"nodes\":[{\"a\":[]}]}";
    let none: [u64; 1] = [0];
    let call = |export: &str, resolve: bool| {
        let function = thaw_tjs_get_global(
            std::ffi::CString::new(format!("pr::{export}"))
                .unwrap()
                .as_ptr(),
        );
        let handles = none.as_ptr().cast::<u8>();
        unsafe {
            if resolve {
                thaw_tjs_call_handle_mixed_exact_resolved_consuming_result(
                    function as u64,
                    wire.as_ptr(),
                    handles,
                )
            } else {
                thaw_tjs_call_handle_mixed_exact_consuming_result(
                    function as u64,
                    wire.as_ptr(),
                    handles,
                )
            }
        }
    };
    let text = |h: u64| {
        unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(h)) }
            .to_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(text(call("later", true).value), "42");
    assert_eq!(text(call("later", false).value), "[object Promise]");
    let rejected = call("laterThrow", true);
    assert_eq!(rejected.value, 0);
    assert!(rejected.exception_handle > 0 && rejected.exception_object_like == 1);
}

#[test]
fn class_method_promise_results_are_awaited_when_the_flag_is_set() {
    use std::ffi::{CStr, CString};
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = { "tj/index.js": function(module, exports, require) {
  setTimeout(function () {}, 60000);
  function Svc() {}
  Svc.prototype.run = async function (n) { await new Promise(function (r) { setTimeout(r, 1); }); return n * 2; };
  Svc.prototype.fail = async function () { throw new TypeError('nope'); };
  exports.Svc = Svc;
}, };
var __thaw_bundle_require_maps = { "tj/index.js": {}, };
var __thaw_bundle_entry_key = "tj/index.js";
})(globalThis.require);"#;
    let (name, source) = (CString::new("tj").unwrap(), CString::new(bundle).unwrap());
    assert_eq!(
        crate::host::thaw_tjs_register(name.as_ptr(), source.as_ptr()),
        1
    );
    // flags: bit 0 = handle id result, bit 1 = await a Promise result
    let call = |symbol: &str, args: &str, receiver: i64, flags: u8| -> Result<String, String> {
        let (symbol, args) = (CString::new(symbol).unwrap(), CString::new(args).unwrap());
        let result =
            crate::host::thaw_tjs_class_call(symbol.as_ptr(), args.as_ptr(), receiver, flags);
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
    };
    let id: i64 = call("$new$tj::Svc$arity0", "[]", 0, 1)
        .unwrap()
        .parse()
        .unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        call("$method$tj::Svc$run$overload0$arity1", "[21]", id, 2),
        Ok("42".into())
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(call("$method$tj::Svc$fail$overload0$arity0", "[]", id, 2)
        .unwrap_err()
        .contains("nope"));
}

#[test]
fn typed_call_awaits_its_own_promise_without_draining_the_idle_loop() {
    use crate::host::thaw_tjs_register;
    use std::ffi::CString;
    // A pending 60 s timer must not delay the call: only the awaited promise's 1 ms timer runs.
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = { "aw/index.js": function(module, exports, require) {
  setTimeout(function () {}, 60000);
  exports.f = async function (n) { await new Promise(function (r) { setTimeout(r, 1); }); return n + 1; };
  exports.bad = async function () { throw new TypeError('nope'); };
}, };
var __thaw_bundle_require_maps = { "aw/index.js": {}, };
var __thaw_bundle_entry_key = "aw/index.js";
})(globalThis.require);"#;
    let (name, source) = (CString::new("aw").unwrap(), CString::new(bundle).unwrap());
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    let started = std::time::Instant::now();
    assert_eq!(typed_call("aw::f", "[2]"), Ok("3".into()));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(typed_call("aw::bad", "[]").unwrap_err().contains("nope"));
}

#[test]
fn node_builtin_script_is_scanned_and_run_as_a_one_module_bundle() {
    let script = "module.exports = { twice: function (n) { return n * 2; } };";
    assert!(crate::scan::scan_bundle(script).is_err()); // a plain script is not a registry bundle
    let wrapped = crate::scan::script_as_bundle(script);
    assert!(crate::scan::scan_bundle(&wrapped).unwrap().is_clean());
    let mut p = crate::host::Package::load(&wrapped).expect("loads");
    assert_eq!(
        p.call("twice", &[crate::host::Native::Number(21.0)]).ok(),
        Some(crate::host::Native::Number(42.0))
    );
    // a registry bundle passes through unchanged
    let registry = "module.exports = (function(require){ var __thaw_bundle_entry_key = \"a\"; var __thaw_bundle_factories = { \"a\": function(module){ module.exports = 1; } }; var __thaw_bundle_require_maps = { \"a\": {} }; })(globalThis.require);";
    assert_eq!(crate::scan::script_as_bundle(registry), registry);
}

#[test]
fn error_subclasses_inherit_from_error_and_native_error_tokens_keep_their_message() {
    let src =
        "console.log(Object.getPrototypeOf(TypeError) === Error, Error.isPrototypeOf(RangeError));";
    assert_eq!(one(src), "true true");
    let mut it = crate::Interp::new();
    let ctl = crate::host::tagged_error(&mut it, "\u{1}RangeError\u{1}\u{1e}E1:3:-:abc");
    let crate::Ctl::Throw(error) = ctl else {
        panic!("expected a throw")
    };
    let got = it.get(&error, "message").unwrap();
    assert_eq!(&*it.to_str(&got), "abc");
    let got = it.get(&error, "name").unwrap();
    assert_eq!(&*it.to_str(&got), "RangeError");
}

#[test]
fn tagged_error_keeps_bag_key_order_with_code_after_errno() {
    let mut it = crate::Interp::new();
    let frame = "\u{1}Error\u{1}\u{1e}E1:1:-:m\u{3}ENOENT\u{5}{\"errno\":-2,\"code\":\"ENOENT\",\"syscall\":\"s\"}";
    let crate::Ctl::Throw(error) = crate::host::tagged_error(&mut it, frame) else {
        panic!("expected a throw")
    };
    let global = crate::Value::Object(it.global_object.clone());
    it.set(&global, "e", error).unwrap();
    let keys = it.run("Object.keys(e).join()").unwrap();
    assert_eq!(&*it.to_str(&keys), "errno,code,syscall");
    // a frame with a code but no bag still gets `code`
    let crate::Ctl::Throw(error) = crate::host::tagged_error(&mut it, "\u{1}Error\u{1}m\u{3}X")
    else {
        panic!("expected a throw")
    };
    it.set(&global, "e", error).unwrap();
    let keys = it.run("Object.keys(e).join()").unwrap();
    assert_eq!(&*it.to_str(&keys), "code");
}

#[test]
fn os_network_interfaces_are_grouped_by_name_with_real_addresses() {
    let src = "var all = JSON.parse(__thaw_os_network_interfaces()); var names = Object.keys(all);\
               console.log(names.length > 0, names.every(function (n) { return all[n].every(function (a) { return typeof a.address === 'string' && (a.family === 'IPv4' || a.family === 'IPv6') && typeof a.internal === 'boolean'; }); }));";
    assert_eq!(one(src), "true true");
}

#[test]
fn registering_the_same_closure_identity_twice_returns_one_callback_id() {
    use crate::host::{thaw_tjs_callback_identity, thaw_tjs_register_callback};
    let identity = 0x1000 as *const std::ffi::c_void;
    thaw_tjs_callback_identity(identity);
    let first = thaw_tjs_register_callback(sum_adapter as *const _, 0x10 as *const _, 3);
    // a different wrapper (adapter and context) of the same closure
    thaw_tjs_callback_identity(identity);
    let second = thaw_tjs_register_callback(throwing_adapter as *const _, 0x20 as *const _, 3);
    assert_eq!(first, second);
    thaw_tjs_callback_identity(0x2000 as *const _);
    assert_ne!(
        first,
        thaw_tjs_register_callback(sum_adapter as *const _, 0x30 as *const _, 3)
    );
}

#[test]
fn a_regexp_marker_argument_becomes_a_real_regexp() {
    use crate::host::Native;
    let mut it = crate::Interp::new();
    let marker = Native::Object(vec![(
        "__thaw_regexp__".into(),
        Native::Object(vec![
            ("source".into(), Native::Str("b+".into())),
            ("flags".into(), Native::Str("i".into())),
        ]),
    )]);
    let value = crate::host::to_value(&mut it, &marker);
    let got = it.get(&value, "source").unwrap();
    assert_eq!(&*it.to_str(&got), "b+");
    let got = it.get(&value, "flags").unwrap();
    assert_eq!(&*it.to_str(&got), "i");
}

#[test]
fn async_generator_return_runs_finally() {
    let mut it = Interp::new();
    it.run(
        "async function* g(){ try{ yield 1; yield 2; } finally { console.log('cleanup'); } }\n\
         const it = g(); it.next().then(() => it.return()).then(r => console.log(JSON.stringify(r)));",
    )
    .ok()
    .unwrap();
    it.run_event_loop().ok().unwrap();
    assert_eq!(it.out, vec!["cleanup", "{\"done\":true}"]);
}

#[test]
fn for_await_break_runs_async_generator_finally() {
    let mut it = Interp::new();
    it.run(
        "async function* gen(){ try { yield 1; yield 2; yield 3; } finally { console.log('cleanup'); } }\n\
         (async function () { for await (const v of gen()) { console.log('x', v); if (v === 2) break; } console.log('after'); })();",
    )
    .ok()
    .unwrap();
    it.run_event_loop().ok().unwrap();
    assert_eq!(it.out, vec!["x 1", "x 2", "cleanup", "after"]);
}

#[test]
fn new_function_for_await_break_runs_finally() {
    let mut it = Interp::new();
    it.run(
        "const gen = new Function('return (async function* () { try { yield 1; yield 2; yield 3; } finally { console.log(\"cleanup\"); } })()')();\n\
         (async function main() { for await (const x of gen) { console.log('x', x); if (x === 2) break; } console.log('after break'); })();",
    )
    .ok()
    .unwrap();
    it.run_event_loop().ok().unwrap();
    assert_eq!(it.out, vec!["x 1", "x 2", "cleanup", "after break"]);
}

#[test]
fn hosted_new_function_async_iterator_return_runs_finally() {
    use crate::host::{
        thaw_tjs_get_global, thaw_tjs_iterator_step_exact_result, thaw_tjs_load_script,
        thaw_tjs_to_async_iterator_exact_result,
    };
    use std::ffi::CString;
    let src = CString::new(
        "globalThis.cleaned = false; globalThis.gen = new Function('return (async function* () { try { yield 1; yield 2; yield 3; } finally { globalThis.cleaned = true; } })()')();",
    )
    .unwrap();
    assert_eq!(thaw_tjs_load_script(src.as_ptr()), 1);
    let name = CString::new("gen").unwrap();
    let src_handle = thaw_tjs_get_global(name.as_ptr());
    assert!(src_handle > 0);
    let acquired = thaw_tjs_to_async_iterator_exact_result(src_handle as u64);
    assert!(acquired.error.is_null() && acquired.exception_handle == 0);
    let iter = acquired.value;
    let none = CString::new("").unwrap();
    let n1 = thaw_tjs_iterator_step_exact_result(iter, 6, none.as_ptr());
    assert!(n1.error.is_null() && n1.exception_handle == 0);
    let n2 = thaw_tjs_iterator_step_exact_result(iter, 6, none.as_ptr());
    assert!(n2.error.is_null() && n2.exception_handle == 0);
    let ret = thaw_tjs_iterator_step_exact_result(iter, 7, none.as_ptr());
    assert!(ret.error.is_null() && ret.exception_handle == 0);
    let probe = CString::new("if (!globalThis.cleaned) throw new Error('no cleanup');").unwrap();
    assert_eq!(thaw_tjs_load_script(probe.as_ptr()), 1);
}

#[test]
fn host_realm_streams_console_output_from_a_closed_new_function_generator() {
    use crate::host::{
        default_realm, thaw_tjs_get_global, thaw_tjs_iterator_step_exact_result,
        thaw_tjs_load_script, thaw_tjs_to_async_iterator_exact_result, with_package,
    };
    use std::ffi::CString;
    // the node-compat case: `finally` logs with console.log, the close is the for-await break
    let src = CString::new(
        "globalThis.gen = new Function('return (async function* () { try { yield 1; yield 2; yield 3; } finally { console.log(\"cleanup\"); } })()')();",
    )
    .unwrap();
    assert_eq!(thaw_tjs_load_script(src.as_ptr()), 1);
    let name = CString::new("gen").unwrap();
    let acquired =
        thaw_tjs_to_async_iterator_exact_result(thaw_tjs_get_global(name.as_ptr()) as u64);
    let iter = acquired.value;
    let none = CString::new("").unwrap();
    thaw_tjs_iterator_step_exact_result(iter, 6, none.as_ptr());
    thaw_tjs_iterator_step_exact_result(iter, 6, none.as_ptr());
    let ret = thaw_tjs_iterator_step_exact_result(iter, 7, none.as_ptr());
    assert!(ret.error.is_null() && ret.exception_handle == 0);
    // the compiled program never drains `Interp::out`, so the host realm must print as it goes
    let (streamed, buffered) = with_package(default_realm(), |p| {
        Ok((p.interp().rt.stream, p.interp().out.clone()))
    })
    .unwrap();
    assert!(streamed);
    assert!(buffered.is_empty(), "{buffered:?}");
}

#[test]
fn hosted_async_step_return_runs_generator_finally() {
    use crate::host::{
        thaw_tjs_get_global, thaw_tjs_iterator_step_exact_result, thaw_tjs_load_script,
    };
    use std::ffi::CString;
    let src = CString::new(
        "globalThis.cleaned = false; globalThis.gen = (async function* () { try { yield 1; yield 2; yield 3; } finally { globalThis.cleaned = true; } })();",
    )
    .unwrap();
    assert_eq!(thaw_tjs_load_script(src.as_ptr()), 1);
    let name = CString::new("gen").unwrap();
    let iter = thaw_tjs_get_global(name.as_ptr());
    assert!(iter > 0);
    let none = CString::new("").unwrap();
    // mode 6 = async next, mode 7 = async return (the compiled for-await break close)
    let next = thaw_tjs_iterator_step_exact_result(iter as u64, 6, none.as_ptr());
    assert!(next.error.is_null() && next.exception_handle == 0);
    let ret = thaw_tjs_iterator_step_exact_result(iter as u64, 7, none.as_ptr());
    assert!(ret.error.is_null() && ret.exception_handle == 0);
    // thaw_tjs_load_script returns 0 when the probe throws
    let probe = CString::new("if (!globalThis.cleaned) throw new Error('no cleanup');").unwrap();
    assert_eq!(thaw_tjs_load_script(probe.as_ptr()), 1);
}

#[test]
fn get_prototype_of_runs_the_proxy_trap() {
    assert_eq!(
        one("try { Object.getPrototypeOf(new Proxy({}, { getPrototypeOf() { throw new Error('trap'); } })); console.log('accepted'); } catch (e) { console.log(e.message); }"),
        "trap"
    );
    assert_eq!(
        one("console.log(Object.getPrototypeOf(new Proxy({}, {})) === Object.prototype, Reflect.getPrototypeOf(new Proxy({}, { getPrototypeOf() { return null; } })))"),
        "true null"
    );
}

#[test]
fn timer_handle_ref_methods_scan_clean_and_run() {
    let timers = MINI.replace(
        "exports.VERSION = '1.0';",
        "exports.VERSION = '1.0'; exports.t = function () { var t = setTimeout(function () {}, 1); t.unref(); return t.hasRef(); };",
    );
    let clean = crate::scan::scan_bundle(&timers).ok().unwrap();
    assert!(clean.is_clean(), "{:?}", clean.blockers);
    assert_eq!(
        one("const t = setTimeout(() => console.log('never'), 50); console.log(t.unref() === t, t.hasRef());"),
        "true false"
    );
}

// Recorded from node v22.23.1: a cleared/fired Timeout keeps its ref state (true unless unref'd),
// a cleared/fired Immediate reports false.
#[test]
fn cleared_timer_has_ref_matches_node22() {
    assert_eq!(
        one("const t = setTimeout(() => {}, 1000); clearTimeout(t); console.log(t.hasRef());"),
        "true"
    );
    assert_eq!(
        one("const t = setTimeout(() => {}, 1000); t.unref(); clearTimeout(t); console.log(t.hasRef());"),
        "false"
    );
    assert_eq!(
        one("const t = setTimeout(() => {}, 1000); clearTimeout(t); t.ref(); console.log(t.hasRef());"),
        "true"
    );
    assert_eq!(
        one("const i = setInterval(() => {}, 1000); clearInterval(i); console.log(i.hasRef());"),
        "true"
    );
    assert_eq!(
        one("const i = setImmediate(() => {}); clearImmediate(i); console.log(i.hasRef());"),
        "false"
    );
    assert_eq!(
        one("const i = setImmediate(() => {}); i.unref(); clearImmediate(i); console.log(i.hasRef());"),
        "false"
    );
}

#[test]
fn timeout_to_primitive_is_the_timer_id_and_immediate_has_none() {
    assert_eq!(
        one("const t = setTimeout(() => {}, 50); const i = setInterval(() => {}, 50); const m = setImmediate(() => {}); \
             console.log(typeof t[Symbol.toPrimitive], typeof i[Symbol.toPrimitive], typeof m[Symbol.toPrimitive]); \
             console.log(t[Symbol.toPrimitive]() === +t, typeof +t, String(t) === String(+t), `${i}` === String(+i)); \
             clearTimeout(t); clearInterval(i); clearImmediate(m);"),
        "function function undefined|true number true true"
    );
}

#[test]
fn timeout_to_primitive_id_clears_the_timer_and_dispose_clears_it() {
    assert_eq!(
        one("const t = setTimeout(() => console.log('never'), 20); clearTimeout(String(t)); \
             const u = setTimeout(() => console.log('never'), 20); clearTimeout(+u + ''); \
             const v = setTimeout(() => console.log('never'), 20); v[Symbol.dispose](); \
             const m = setImmediate(() => console.log('never')); m[Symbol.dispose](); \
             console.log(typeof Symbol.dispose, typeof v[Symbol.dispose], typeof m[Symbol.dispose]);"),
        "symbol function function"
    );
}

#[test]
fn webassembly_stub_is_typeof_safe_and_scan_blocks_real_use() {
    assert_eq!(
        one("console.log(typeof WebAssembly, typeof WebAssembly.compile)"),
        "object function"
    );
    let probe = MINI.replace(
        "exports.VERSION = '1.0';",
        "exports.VERSION = '1.0'; exports.has = function () { return typeof WebAssembly !== 'undefined'; };",
    );
    assert!(crate::scan::scan_bundle(&probe).ok().unwrap().is_clean());
    let use_it = MINI.replace(
        "exports.VERSION = '1.0';",
        "exports.VERSION = '1.0'; exports.run = function (x) { return WebAssembly.instantiate(x); };",
    );
    let blocked = crate::scan::scan_bundle(&use_it).ok().unwrap();
    assert!(
        blocked.blockers.contains_key("global:WebAssembly"),
        "{:?}",
        blocked.blockers
    );
}

#[test]
fn dns_host_resolves_localhost_and_scan_accepts_it() {
    let lookup = one("const r = JSON.parse(__thaw_dns_lookup('localhost')); console.log(r.ok, r.addresses.length > 0);");
    assert_eq!(lookup, "true true");
    let dns = MINI.replace(
        "exports.VERSION = '1.0';",
        "exports.VERSION = '1.0'; exports.lookup = function (h) { return __thaw_dns_lookup_start(h); }; exports.servers = function () { return __thaw_dns_default_servers(); }; exports.query = function (n) { return __thaw_dns_query_start(n, 'A', '[]', 100, 1); };",
    );
    assert!(crate::scan::scan_bundle(&dns).ok().unwrap().is_clean());
}

#[test]
fn message_channel_delivers_a_structured_clone_and_rejects_functions() {
    let mut it = Interp::new();
    it.run(
        "const ch = new MessageChannel(); ch.port1.onmessage = (e) => { console.log(JSON.stringify(e.data)); ch.port1.close(); }; \
         ch.port2.postMessage({ a: [1, 2] }); try { ch.port2.postMessage(() => 1); } catch (e) { console.log(e.name); }",
    )
    .ok()
    .unwrap();
    it.run_event_loop().ok().unwrap();
    assert_eq!(it.out, vec!["DataCloneError", "{\"a\":[1,2]}"]);
    let probe = MINI.replace(
        "exports.VERSION = '1.0';",
        "exports.VERSION = '1.0'; exports.port = function (v) { return globalThis.__thaw_is_message_port(v); };",
    );
    assert!(crate::scan::scan_bundle(&probe).ok().unwrap().is_clean());
}

#[test]
fn async_context_bridge_get_swap_roundtrip() {
    assert_eq!(
        one("const b = __thaw_async_context_bootstrap; console.log(b.get(), b.swap('A'), b.get(), b.swap(undefined), b.get())"),
        "undefined undefined A A undefined"
    );
}

#[test]
fn async_context_follows_promise_reactions_and_await() {
    let src = "const b = __thaw_async_context_bootstrap, log = [];
      b.swap('A');
      Promise.resolve().then(() => log.push('then:' + b.get()));
      queueMicrotask(() => log.push('micro:' + b.get()));
      (async () => { await null; log.push('await:' + b.get()); await new Promise((r) => r()); log.push('await2:' + b.get()); })();
      b.swap('B');
      Promise.resolve().then(() => log.push('thenB:' + b.get()));
      b.swap(undefined);
      Promise.resolve().then(() => log.push('thenU:' + b.get()));
      new Promise((r) => setTimeout(r, 1)).then(() => log.push('late:' + b.get()));
      setTimeout(() => console.log(log.sort().join(',')), 20);";
    assert_eq!(
        run_loop(src),
        "await2:A,await:A,late:undefined,micro:A,then:A,thenB:B,thenU:undefined"
    );
}

#[test]
fn async_context_follows_next_tick() {
    assert_eq!(
        run_loop("const b = __thaw_async_context_bootstrap; b.swap('T'); process.nextTick(() => console.log('t:' + b.get())); b.swap(undefined); process.nextTick(() => console.log('u:' + b.get()));"),
        "t:T|u:undefined"
    );
}

#[test]
fn async_context_follows_timers_and_immediates() {
    // Completion is counted, not timed: a wall-clock timeout can fire before the immediate on a slow build.
    let src = "const b = __thaw_async_context_bootstrap, l = [];
      b.swap('T'); setTimeout(() => { l.push('to:' + b.get()); done(); }, 1);
      let n = 0; const iv = setInterval(() => { l.push('iv:' + b.get()); if (++n === 2) clearInterval(iv); done(); }, 1);
      b.swap('I'); setImmediate(() => { l.push('im:' + b.get()); done(); });
      b.swap(undefined); const after = b.get();
      function done() { if (l.length === 4) console.log(l.sort().join(',') + ' after:' + after); }";
    assert_eq!(run_loop(src), "im:I,iv:T,iv:T,to:T after:undefined");
}

#[test]
fn async_hooks_exports_are_scan_clean_and_present() {
    assert_eq!(
        one("const h = __thaw_async_hooks_exports; console.log(typeof h.AsyncLocalStorage, typeof h.AsyncResource, typeof h.createHook, h.executionAsyncId())"),
        "function function function 1"
    );
    let probe = MINI.replace(
        "exports.VERSION = '1.0';",
        "exports.VERSION = '1.0'; exports.h = globalThis.__thaw_async_hooks_exports;",
    );
    assert!(crate::scan::scan_bundle(&probe).ok().unwrap().is_clean());
}

#[test]
fn als_propagates_through_every_async_boundary() {
    // Completion is counted, not timed: a wall-clock dump timer can fire before the immediate on a slow build.
    // Sorted: Node's timeout/immediate and tick/microtask order differ by CJS/ESM and timing.
    let src = "const { AsyncLocalStorage } = __thaw_async_hooks_exports; const als = new AsyncLocalStorage(); const log = [];
      function done() { if (log.length === 8) console.log(log.slice().sort().join(',')); }
      als.run('A', () => { log.push('sync:' + als.getStore());
        setTimeout(() => { log.push('timeout:' + als.getStore()); done(); }, 1);
        setImmediate(() => { log.push('immediate:' + als.getStore()); done(); });
        process.nextTick(() => { log.push('tick:' + als.getStore()); done(); });
        queueMicrotask(() => { log.push('micro:' + als.getStore()); done(); });
        Promise.resolve().then(() => { log.push('then:' + als.getStore()); done(); });
        (async () => { await new Promise((r) => setTimeout(r, 2)); log.push('await:' + als.getStore()); done(); })(); });
      log.push('outer:' + als.getStore()); done();";
    assert_eq!(
        run_loop(src),
        "await:A,immediate:A,micro:A,outer:undefined,sync:A,then:A,tick:A,timeout:A"
    );
}

#[test]
fn als_follows_awaits_enter_with_exit_and_nesting() {
    let src = "const { AsyncLocalStorage } = __thaw_async_hooks_exports; const als = new AsyncLocalStorage(); const out = [];
      async function work(n) { await null; out.push('w' + n + ':' + als.getStore()); await new Promise((r) => setTimeout(r, 1)); out.push('x' + n + ':' + als.getStore()); }
      (async () => {
        await Promise.all([als.run(1, work, 1), als.run(2, work, 2)]);
        als.enterWith(7); out.push('enter:' + als.getStore());
        out.push('exit:' + als.exit(() => String(als.getStore()))); out.push('after:' + als.getStore());
        setTimeout(() => { out.push('t:' + als.getStore()); console.log(out.join(',')); }, 1);
        als.run(9, () => { out.push('nested:' + als.getStore()); });
      })();";
    assert_eq!(
        run_loop(src),
        "w1:1,w2:2,x1:1,x2:2,enter:7,exit:undefined,after:7,nested:9,t:7"
    );
}

#[test]
fn als_resource_bind_and_disable() {
    let src = "const { AsyncLocalStorage, AsyncResource, createHook } = __thaw_async_hooks_exports; const als = new AsyncLocalStorage(); let r;
      als.run('R', () => { r = new AsyncResource('X'); }); const fn = als.run('B', () => AsyncResource.bind(() => als.getStore()));
      console.log(als.getStore(), r.runInAsyncScope(() => als.getStore()), fn());
      const h = createHook({ init() {} }).enable(); console.log(typeof h.disable); h.disable();
      als.run('A', () => setTimeout(() => console.log('later:' + als.getStore()), 1));
      als.disable(); console.log('now:' + als.getStore());";
    assert_eq!(
        run_loop(src),
        "undefined R B|function|now:undefined|later:undefined"
    );
}

static HOOK_INITS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Counts `init` hook calls from the async_hooks dispatch; the result is ignored.
unsafe extern "C" fn init_hook_adapter(
    _context: *const std::ffi::c_void,
    _args: *const std::ffi::c_char,
) -> *mut std::ffi::c_char {
    HOOK_INITS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    thaw_arena::owned_string("null")
}

#[test]
fn async_hooks_create_hook_runs_a_callback_marker_from_a_literal_object() {
    use crate::host::{thaw_tjs_register_callback, Native, Package};
    let init = thaw_tjs_register_callback(init_hook_adapter as *const _, std::ptr::null(), 4);
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"h/index.js": function(module, exports, require) {
  exports.hook = function (o) {
    var A = __thaw_async_hooks_exports;
    var h = A.createHook(o).enable();
    new A.AsyncResource('X');
    var kind = typeof h.disable;
    h.disable();
    return kind;
  };
},
};
var __thaw_bundle_require_maps = { "h/index.js": {}, };
var __thaw_bundle_entry_key = "h/index.js";
})(globalThis.require);"#;
    let mut p = Package::load(bundle).expect("loads");
    // the object literal's `init` arrives as a callback marker, the same wire shape the compiled side sends
    let options = Native::Object(vec![("init".into(), callback_marker(init))]);
    assert_eq!(
        p.call("hook", &[options]).ok(),
        Some(Native::Str("function".into()))
    );
    assert!(HOOK_INITS.load(std::sync::atomic::Ordering::SeqCst) >= 1);
}

#[test]
fn vm_run_ids_are_unique_across_realms() {
    let mut a = Interp::new();
    let mut b = Interp::new();
    let fa = a
        .run("(function(){ return function(){ return 'A'; }; })")
        .unwrap();
    let fb = b
        .run("(function(){ return function(){ return 'B'; }; })")
        .unwrap();
    let _ = b.call(&fb, &Value::Undefined, &[]).unwrap();
    let ia = b.call(&fa, &Value::Undefined, &[]).unwrap();
    let ra = b.call(&ia, &Value::Undefined, &[]).unwrap();
    assert_eq!(b.to_str(&ra).to_string(), "A");
}

#[test]
fn vm_create_and_free_contexts() {
    assert_eq!(
        one("const a = __thaw_vm_create(), b = __thaw_vm_create(); console.log(typeof a, a !== b); __thaw_vm_free(a); __thaw_vm_free(a);"),
        "number true"
    );
}

#[test]
fn vm_many_contexts_are_cheap_enough() {
    let t = std::time::Instant::now();
    one("for (let i = 0; i < 40; i++) { const id = __thaw_vm_create(); __thaw_vm_run(id, '1', 'f', 0, undefined, undefined); __thaw_vm_free(id); }");
    assert!(t.elapsed().as_secs() < 60, "{:?}", t.elapsed());
}

#[test]
fn vm_run_in_sub_realm_isolates_intrinsics_and_persists_declarations() {
    let src = "const id = __thaw_vm_create();
      __thaw_vm_run(id, 'var x = 10; let y = 20; const z = 30; function g() { return x + y + z; }', 'f', 0, undefined, undefined);
      const r = __thaw_vm_run(id, 'g() + typeof y + typeof process + typeof require', 'f', 0, undefined, undefined);
      __thaw_vm_run(id, 'Object.prototype.leak = 1', 'f', 0, undefined, undefined);
      const arr = __thaw_vm_run(id, '[1, 2]', 'f', 0, undefined, undefined);
      let err; try { __thaw_vm_run(id, 'nope + 1', 'f', 0, undefined, undefined); } catch (e) { err = e; }
      console.log(r, typeof ({}).leak, __thaw_vm_run(id, 'typeof Object.prototype.leak', 'f', 0), arr instanceof Array, Array.isArray(arr), err.name, err instanceof ReferenceError, __thaw_vm_run(-1, '1 + 1', 'f', 0));";
    assert_eq!(
        one(src),
        "60numberundefinedundefined undefined number false true ReferenceError false 2"
    );
}

#[test]
fn vm_realm_has_only_ecma_globals() {
    assert_eq!(one("const id = __thaw_vm_create(); console.log(__thaw_vm_run(id, 'JSON.stringify([Math.max(1,3), typeof Promise, typeof console, typeof WebAssembly, typeof setTimeout, typeof process, this === globalThis])', 'f', 0, undefined, undefined));"),
        "[3,\"function\",\"object\",\"object\",\"undefined\",\"undefined\",true]");
}

#[test]
fn vm_sandbox_mirroring_matches_node() {
    let src = include_str!("../../thaw-registry/src/registry/builtins/system/vm_context.js").to_string() + "
      const ctx = createContext({ a: 1, out: [] });
      runInContext('var x = 10; let y = 20; const z = 30; function g() { return x + y + z; } out.push(g())', ctx);
      console.log(runInContext('g() + a + typeof y', ctx), ctx.x, typeof ctx.y, typeof ctx.g, JSON.stringify(ctx.out));
      console.log(runInNewContext('value * 2', { value: 21 }), runInNewContext('typeof process + typeof require'));
      runInNewContext('Object.prototype.leak = 1'); console.log(typeof ({}).leak, runInNewContext('typeof Object.prototype.leak'));
      const c2 = createContext({}); runInContext('n = (typeof n === \"number\" ? n : 0) + 1', c2); runInContext('n = (typeof n === \"number\" ? n : 0) + 1', c2); console.log(c2.n);
      const c3 = createContext({ a: 1 }); runInContext('a = 5; var v = 1; let l = 2; w = 3', c3); console.log(c3.a, c3.v, c3.l, c3.w, runInContext('l', c3));
      const c4 = createContext({ k: 1 }); runInContext('var k = 7; function f() { return k }', c4); c4.k = 9; console.log(c4.k, runInContext('f()', c4));
      const c6 = createContext({}); runInContext('var cnt = 0; function inc() { return ++cnt }', c6); runInContext('inc(); inc()', c6); console.log(c6.cnt, runInContext('cnt', c6)); c6.cnt = 10; console.log(runInContext('inc()', c6), c6.cnt);
      let e; try { runInContext('var ev = 1; throw new Error(\"boom\")', c6); } catch (x) { e = x } console.log(e.message, c6.ev, e instanceof Error);";
    assert_eq!(
        one(&src),
        "61number 10 undefined function [60]|42 undefinedundefined|undefined undefined|2|5 1 undefined 3 2|9 9|2 2|11 11|boom 1 false"
    );
}

#[test]
fn vm_global_this_reads_and_writes_sandbox() {
    let src = include_str!("../../thaw-registry/src/registry/builtins/system/vm_context.js").to_string() + "
      const sb = { value: 1 }; runInNewContext('globalThis.value = 5; var seen = value; this.zz = value + 1', sb); console.log(sb.value, sb.seen, sb.zz);
      console.log(runInNewContext('globalThis.value + this.value', { value: 3 }));
      const s2 = { arr: [1, 2], fn: (n) => n + 1 };
      const r = runInNewContext('globalThis.q = fn(arr.length); [typeof console, typeof setTimeout, typeof JSON, typeof Promise, [] instanceof Array, q]', s2);
      console.log(JSON.stringify(r), s2.q, r instanceof Array, Array.isArray(r), runInNewContext('this === globalThis'));";
    assert_eq!(
        one(&src),
        "5 5 6|6|[\"object\",\"undefined\",\"object\",\"function\",true,3] 3 false true true"
    );
}

#[test]
fn vm_timeout_interrupts_runaway_scripts() {
    let t = std::time::Instant::now();
    let src = include_str!("../../thaw-registry/src/registry/builtins/system/vm_context.js").to_string() + "
      const c = createContext({});
      try { runInContext('while (true) {}', c, { timeout: 30 }); } catch (e) { console.log(e.code, e.message); }
      console.log(runInContext('1 + 1', c));
      try { runInNewContext('for (;;) {}', {}, { timeout: 20 }); } catch (e) { console.log(e.code, e instanceof Error); }
      try { runInNewContext('nope + 1'); } catch (e) { console.log(e.name, e.message, e instanceof ReferenceError); }
      try { runInContext('do {} while (true)', c, { timeout: 15 }); } catch (e) { console.log(e.code, e.message); }
      const sb = createContext({});
      try { runInContext('try { while(true){} } catch (e) { caught = true }', sb, { timeout: 25 }); } catch (e) { console.log(e.code, sb.caught); }
      try { runInThisContext('while(true){}', { timeout: 30 }); } catch (e) { console.log(e.code, e.message); }
      console.log(runInContext('let n = 0; for (let i = 0; i < 1000; i++) n += i; n', sb, { timeout: 1000 }), runInThisContext('1+2'));";
    assert_eq!(
        one(&src),
        "ERR_SCRIPT_EXECUTION_TIMEOUT Script execution timed out after 30ms|2|ERR_SCRIPT_EXECUTION_TIMEOUT false|ReferenceError nope is not defined false|ERR_SCRIPT_EXECUTION_TIMEOUT Script execution timed out after 15ms|ERR_SCRIPT_EXECUTION_TIMEOUT undefined|ERR_SCRIPT_EXECUTION_TIMEOUT Script execution timed out after 30ms|499500 3"
    );
    assert!(t.elapsed().as_secs() < 5, "{:?}", t.elapsed());
}

#[test]
fn vm_eval_syntax_errors_and_script_emulation() {
    let src = include_str!("../../thaw-registry/src/registry/builtins/system/vm_context.js").to_string() + "
      const t = (f) => { try { return f(); } catch (e) { return 'ERR:' + e.name; } };
      function Script(code) { const s = String(code); __thaw_vm_eval(s, 'f', true); this.s = s; }
      Script.prototype.runInContext = function (c) { return runInContext(this.s, c); };
      Script.prototype.runInThisContext = function () { return runInThisContext(this.s); };
      console.log(t(() => { new Script('return 1'); return 'compiled'; }), t(() => new Script('var = ;')));
      const c2 = createContext({}); const s = new Script('n = (typeof n === \"number\" ? n : 0) + 1'); s.runInContext(c2); s.runInContext(c2);
      console.log(c2.n, isContext(c2), isContext({}), new Script('1 + 1').runInThisContext());
      const ctx = createContext({ zz: 5 });
      console.log(runInContext('(function (a) {\\nreturn zz + a\\n})', ctx)(1), Function.apply(null, ['a', 'b'].concat('return a + b'))(2, 3), Function('__thaw_extensions', 'with (__thaw_extensions[0]) { return function () {\\nreturn q\\n}; }')([{ q: 7 }])());
      console.log(t(() => runInContext('let a = ;', createContext({}))), t(() => runInThisContext('let a = ;')), t(() => runInNewContext('return 1')), typeof __thaw_vm_eval('1', 'f', true), __thaw_vm_eval('1+1', 'f', false));";
    assert_eq!(
        one(&src),
        "ERR:SyntaxError ERR:SyntaxError|2 true false 2|6 5 7|ERR:SyntaxError ERR:SyntaxError ERR:SyntaxError undefined 2"
    );
}

#[test]
fn node_module_bundle_gets_create_require_and_builtin_names() {
    use crate::host::{Native, Package};
    let bundle = r#"module.exports = (function(require) {
var __thaw_bundle_builtin_names = Object.freeze(["fs","module","path"]);
var __thaw_bundle_factories = {
"node:module": function(module, exports, require) {
  var set = new Set(__thaw_bundle_builtin_names.slice());
  function isBuiltin(n) { return set.has(String(n).replace(/^node:/, '')); }
  function apply1(fn, a) { return fn(a); }
  function base() { var r = __thaw_bundle_create_require('/tmp/example/main.js'); return apply1(r('node:path').basename, '/a/b.txt'); }
  function res() { var r = __thaw_bundle_create_require('/tmp/example/main.js'); return [r.resolve('node:path'), r.resolve('path'), r('path') === r('node:path')].join(); }
  function bad() { try { __thaw_bundle_create_require('/x')('nope-xyz'); } catch (e) { return e.code; } return 'none'; }
  module.exports = { isBuiltin: isBuiltin, base: base, res: res, bad: bad };
},
"node:path": function(module, exports) { function basename(p) { return p.split('/').pop(); } module.exports = { basename: basename }; },
"p/index.js": function(module, exports) { module.exports = { probe: function () { return typeof __thaw_bundle_create_require; } }; },
};
var __thaw_bundle_require_maps = { "node:module": {}, "node:path": {}, "p/index.js": {}, };
var __thaw_bundle_entry_key = "node:module";
})(globalThis.require);"#;
    assert!(crate::scan::scan_bundle(bundle).unwrap().is_clean());
    let mut p = Package::load(bundle).expect("loads");
    assert_eq!(p.call("base", &[]).unwrap(), Native::Str("b.txt".into()));
    assert_eq!(
        p.call("res", &[]).unwrap(),
        Native::Str("node:path,path,true".into())
    );
    assert_eq!(
        p.call("bad", &[]).unwrap(),
        Native::Str("MODULE_NOT_FOUND".into())
    );
    assert_eq!(
        p.call("isBuiltin", &[Native::Str("node:fs".into())])
            .unwrap(),
        Native::Bool(true)
    );
    assert_eq!(
        p.call("isBuiltin", &[Native::Str("fs".into())]).unwrap(),
        Native::Bool(true)
    );
    assert_eq!(
        p.call("isBuiltin", &[Native::Str("not-a-module".into())])
            .unwrap(),
        Native::Bool(false)
    );
    // other entries get no loader globals
    let other = bundle.replace(
        r#"var __thaw_bundle_entry_key = "node:module";"#,
        r#"var __thaw_bundle_entry_key = "p/index.js";"#,
    );
    let mut q = Package::load(&other).expect("loads");
    assert_eq!(
        q.call("probe", &[]).unwrap(),
        Native::Str("undefined".into())
    );
}

/// A primitive rejection packet: only its value tag and payload words are set (no object, native or
/// aggregate identity). The owned result text is the `[native exception value]` placeholder.
fn primitive_exception_packet(
    tag: i64,
    number: f64,
    bigint: i64,
    boolean: bool,
    object: usize,
) -> thaw_arena::exception::ExceptionPacket {
    thaw_arena::exception::ExceptionPacket {
        id: 0,
        pending: 0,
        native_text: 0,
        native_error_identity: 0,
        object,
        native: 0,
        aggregate: 0,
        tag,
        number: number.to_bits(),
        bigint: bigint as u64,
        boolean: u64::from(boolean),
        _roots: Vec::new(),
        _owned_bytes: Vec::new(),
        _leases: Vec::new(),
    }
}

/// Runs a callback result that carries `packet` through the host's result unwrapping.
fn take_exception_packet(packet: thaw_arena::exception::ExceptionPacket) -> Ctl {
    let pointer = thaw_arena::owned_string(b"\x02[native exception value]");
    let (_, packet) = thaw_arena::exception::register_packet(packet).unwrap();
    assert!(thaw_arena::bind_owned_exception(pointer, packet));
    let mut it = Interp::new();
    match crate::host::take_callback_result(&mut it, pointer) {
        Err(ctl) => ctl,
        Ok(text) => panic!("expected a throw, got {text:?}"),
    }
}

#[test]
fn exception_packet_primitive_number_zero_survives() {
    // `throw 0`: the falsy reason must reach the JS side as the number 0, not an Error.
    let ctl = take_exception_packet(primitive_exception_packet(1, 0.0, 0, false, 0));
    assert!(matches!(ctl, Ctl::Throw(Value::Number(n)) if n == 0.0));
}

#[test]
fn exception_packet_primitive_tags_keep_their_values() {
    let bool_false = take_exception_packet(primitive_exception_packet(3, 0.0, 0, false, 0));
    assert!(matches!(bool_false, Ctl::Throw(Value::Bool(false))));
    let undef = take_exception_packet(primitive_exception_packet(5, 0.0, 0, false, 0));
    assert!(matches!(undef, Ctl::Throw(Value::Undefined)));
    let null = take_exception_packet(primitive_exception_packet(6, 0.0, 0, false, 0));
    assert!(matches!(null, Ctl::Throw(Value::Null)));
    let big = take_exception_packet(primitive_exception_packet(2, 0.0, -7, false, 0));
    assert!(matches!(
        big,
        Ctl::Throw(Value::BigInt(b)) if *b == num_bigint::BigInt::from(-7)
    ));
}

#[test]
fn exception_packet_with_object_identity_stays_an_error() {
    // An object-identity packet is not a primitive: it keeps the Error path (never the tag value).
    let ctl = take_exception_packet(primitive_exception_packet(1, 0.0, 0, false, 8));
    assert!(matches!(ctl, Ctl::Throw(Value::Object(_))));
}

#[test]
fn exception_packet_handle_value_returns_the_stored_object() {
    // A live handle id maps back to the very object it was stored for (same Rc, same properties).
    let mut it = Interp::new();
    let Ok(Value::Object(original)) =
        it.run("({ code: 'ERR_BAD_REQUEST', response: { status: 404 } })")
    else {
        panic!("object literal did not evaluate to an object");
    };
    let id = crate::host::store_handle(0, Value::Object(original.clone())) as u64;
    let Some(Value::Object(found)) = crate::host::handle_value(id) else {
        panic!("stored handle {id} did not resolve to an object");
    };
    assert!(Rc::ptr_eq(&original, &found));
    assert!(crate::host::handle_value(0).is_none());
    assert!(crate::host::handle_value(id + 1000).is_none());
}

#[test]
fn exception_packet_tag7_unknown_object_keeps_error_fallback() {
    // No tracked handle for the pointer (the lookup symbol is absent in unit tests): the old Error path.
    let mut packet = primitive_exception_packet(7, 0.0, 0, false, 0x1234);
    packet.pending = 0x1234;
    let ctl = take_exception_packet(packet);
    assert!(matches!(ctl, Ctl::Throw(Value::Object(_))));
}

#[test]
fn exception_packet_tag7_live_handle_rethrows_the_original_object() {
    // The guard plus lookup yields the stored object itself, not a fresh Error.
    let mut it = Interp::new();
    let Ok(Value::Object(original)) = it.run("({ code: 'ERR_BAD_REQUEST' })") else {
        panic!("object literal did not evaluate to an object");
    };
    let id = crate::host::store_handle(0, Value::Object(original.clone())) as u64;
    let mut packet = primitive_exception_packet(7, 0.0, 0, false, 0x1234);
    packet.pending = 0x1234;
    let Some(Value::Object(found)) = crate::host::original_js_exception_with(&packet, |_| id)
    else {
        panic!("tag-7 packet with a live handle did not yield the original object");
    };
    assert!(Rc::ptr_eq(&original, &found));
    // Non-zero pending/native_text (async-callback display text) still yields the same object.
    packet.pending = 0x9999;
    packet.native_text = 0x8888;
    let Some(Value::Object(found)) = crate::host::original_js_exception_with(&packet, |_| id)
    else {
        panic!("tag-7 packet with display text in pending/native_text did not yield the object");
    };
    assert!(Rc::ptr_eq(&original, &found));
    // The remaining identity fields still reject the packet.
    packet.native = 0x7777;
    assert!(crate::host::original_js_exception_with(&packet, |_| id).is_none());
    packet.native = 0;
    packet.aggregate = 0x6666;
    assert!(crate::host::original_js_exception_with(&packet, |_| id).is_none());
    packet.aggregate = 0;
    packet.tag = 6;
    assert!(crate::host::original_js_exception_with(&packet, |_| id).is_none());
    packet.tag = 7;
    packet.object = 0;
    assert!(crate::host::original_js_exception_with(&packet, |_| id).is_none());
}

#[test]
fn exception_packet_tag7_recorded_handle_id_resolves_when_lookup_misses() {
    // A throw on another thread: the tracked-root lookup misses (0), but the id recorded in the
    // packet's i64 slot still resolves to the original object.
    let mut it = Interp::new();
    let Ok(Value::Object(original)) = it.run("({ code: 'ERR_BAD_REQUEST' })") else {
        panic!("object literal did not evaluate to an object");
    };
    let id = crate::host::store_handle(0, Value::Object(original.clone())) as u64;
    let packet = primitive_exception_packet(7, 0.0, id as i64, false, 0x1234);
    let Some(Value::Object(found)) = crate::host::original_js_exception_with(&packet, |_| 0) else {
        panic!("tag-7 packet with a recorded handle id did not yield the original object");
    };
    assert!(Rc::ptr_eq(&original, &found));
    // No recorded id and no lookup hit stays on the framed-Error path.
    let packet = primitive_exception_packet(7, 0.0, 0, false, 0x1234);
    assert!(crate::host::original_js_exception_with(&packet, |_| 0).is_none());
}

/// A busy worker keeps the thaw-js loop alive; a native continuation that a timer made ready must still run inside
/// that loop (the compiled terminal loop's `turn_hook`), or the awaited `terminate()` never runs.
#[test]
fn busy_worker_does_not_starve_native_continuation() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RESUMED: AtomicBool = AtomicBool::new(false);
    // stands in for thaw_runtime_run_until_idle: the "native continuation" runs once the timer has fired
    fn hook(it: &mut Interp) -> bool {
        if RESUMED.load(Ordering::SeqCst) {
            return false;
        }
        let fired = it
            .run("globalThis.__fired === true")
            .map(|v| v.truthy())
            .unwrap_or(false);
        if fired {
            RESUMED.store(true, Ordering::SeqCst);
            let _ = it.run("globalThis.__w.terminate()");
            return true;
        }
        false
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // Interp is not Send: build it inside the thread
        let mut it = Interp::new();
        it.turn_hook = Some(hook);
        let _ = it.run_main(
            "const { Worker } = globalThis.__thaw_worker_threads_exports; \
             globalThis.__w = new Worker('while(true){}', { eval: true }); \
             setTimeout(() => { globalThis.__fired = true; }, 100);",
        );
        let _ = tx.send(RESUMED.load(Ordering::SeqCst));
    });
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(10)),
        Ok(true)
    );
}

thread_local! {
    static FAKE_READY: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}
static FAKE_PARKED: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

extern "C" fn fake_park_ready() {
    FAKE_READY.with(|ready| {
        FAKE_PARKED
            .lock()
            .unwrap()
            .extend(ready.borrow_mut().drain(..))
    });
}

extern "C" fn fake_adopt_ready() {
    let parked: Vec<u32> = FAKE_PARKED.lock().unwrap().drain(..).collect();
    FAKE_READY.with(|ready| ready.borrow_mut().extend(parked));
}

#[test]
fn ready_continuation_settled_on_a_coroutine_thread_reaches_the_host() {
    *crate::eventloop::TEST_READY_HOOKS.lock().unwrap() = Some((fake_park_ready, fake_adopt_ready));
    let mut it = crate::Interp::new();
    let emit = it.native(|_, _, _| {
        FAKE_READY.with(|ready| ready.borrow_mut().push(1));
        Ok(Value::Undefined)
    });
    // Shaped like the existing coroutine test: the callback is an argument, and the async body
    // (a coroutine thread) drives an async generator with for-await.
    let f = it
        .eval_function(
            "(async function (emit) { async function* gen() { yield 'done'; } \
             for await (const v of gen()) { emit(v); } })",
        )
        .expect("compiles");
    let started = it.call(&f, &Value::Undefined, &[emit]);
    if let Err(Ctl::Throw(v)) = &started {
        panic!("call threw: {}", it.to_str(v));
    }
    let looped = it.run_event_loop();
    if let Err(Ctl::Throw(v)) = &looped {
        panic!("loop threw: {}", it.to_str(v));
    }
    looped.expect("loop runs");
    // `emit` ran on the async body's coroutine thread; the hand-off must have carried its entry back here.
    assert_eq!(FAKE_READY.with(|ready| ready.borrow().clone()), vec![1]);
    *crate::eventloop::TEST_READY_HOOKS.lock().unwrap() = None;
}

const BYTES: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"b/index.js": function(module, exports, require) {
  exports.make = function () { return Buffer.from([1, 2, 3]); };
  exports.make8 = function () { return new Uint8Array([9]); };
  exports.echo = function (b) { return b; };
  exports.shape = function (b) { return [b.length, Buffer.isBuffer(b), b[2]].join(','); };
  exports.fill = function (b) { for (var i = 0; i < b.length; i++) b[i] = 7; return b; };
},
};
var __thaw_bundle_require_maps = { "b/index.js": {}, };
var __thaw_bundle_entry_key = "b/index.js";
})(globalThis.require);"#;

/// `{"__thaw_tjs_bytes__": ...}` argument node for `data`, as the compiled side sends it.
fn tj35_bytes_arg(data: &[u8]) -> String {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
    format!(r#"{{"__thaw_tjs_bytes__":{{"k":"Buffer","b64":"{b64}"}}}}"#)
}

#[test]
fn tj35_bytes_cross_the_typed_abi_as_tagged_buffers_both_ways() {
    use std::ffi::CString;
    let bundle = CString::new(BYTES).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"bt".as_ptr(), bundle.as_ptr()),
        1
    );
    // a Buffer and a Uint8Array both leave as the Buffer shape the compiled side decodes as Bytes
    assert_eq!(
        typed_call("bt::make", "[]").unwrap(),
        r#"{"type":"Buffer","data":[1,2,3]}"#
    );
    assert_eq!(
        typed_call("bt::make8", "[]").unwrap(),
        r#"{"type":"Buffer","data":[9]}"#
    );
    // the tagged node is rebuilt as a real Buffer
    // (a string result is JSON text, so it is quoted)
    assert_eq!(
        typed_call("bt::shape", &format!("[{}]", tj35_bytes_arg(&[1, 2, 3]))).unwrap(),
        "\"3,true,3\""
    );
    assert_eq!(
        typed_call("bt::shape", &format!("[{}]", tj35_bytes_arg(&[]))).unwrap(),
        "\"0,true,\""
    );
    // every byte value survives both directions
    let all: Vec<u8> = (0..=255).collect();
    let echoed = format!(
        r#"{{"type":"Buffer","data":[{}]}}"#,
        all.iter().map(u8::to_string).collect::<Vec<_>>().join(",")
    );
    assert_eq!(
        typed_call("bt::echo", &format!("[{}]", tj35_bytes_arg(&all))).unwrap(),
        echoed
    );
    // a mutated argument comes back with the new bytes (the compiled side copies them back)
    assert_eq!(
        typed_call("bt::fill", &format!("[{}]", tj35_bytes_arg(&[1, 2, 3]))).unwrap(),
        r#"{"type":"Buffer","data":[7,7,7]}"#
    );
    // a user object with the Buffer shape is not revived: only the tagged node is
    assert_eq!(
        typed_call("bt::shape", r#"[{"type":"Buffer","data":[1]}]"#).unwrap(),
        "\",false,\""
    );
}

const UNDEF: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"ud/index.js": function(module, exports, require) {
  exports.nothing = function () {};
  exports.nul = function () { return null; };
},
};
var __thaw_bundle_require_maps = { "ud/index.js": {}, };
var __thaw_bundle_entry_key = "ud/index.js";
})(globalThis.require);"#;

#[test]
fn tj46_undefined_result_keeps_the_marker() {
    use std::ffi::CString;
    let bundle = CString::new(UNDEF).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"ud".as_ptr(), bundle.as_ptr()),
        1
    );
    // a top-level undefined keeps the marker so the compiled side can tell it from null
    assert_eq!(
        typed_call("ud::nothing", "[]").unwrap(),
        r#"{"$__thaw_napi_undefined$":true}"#
    );
    assert_eq!(typed_call("ud::nul", "[]").unwrap(), "null");
}

const BYTES_KIND: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"k/index.js": function(module, exports, require) {
  exports.proto = function (b) { return [b.length, Buffer.isBuffer(b), b instanceof Uint8Array].join(','); };
},
};
var __thaw_bundle_require_maps = { "k/index.js": {}, };
var __thaw_bundle_entry_key = "k/index.js";
})(globalThis.require);"#;

#[test]
fn tj35_bytes_kind_selects_buffer_or_plain_uint8array_on_the_tagged_node() {
    use base64::Engine as _;
    use std::ffi::CString;
    let bundle = CString::new(BYTES_KIND).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"bk".as_ptr(), bundle.as_ptr()),
        1
    );
    let b64 = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]);
    // `k: "Uint8Array"` revives a plain Uint8Array: not a Buffer, no Buffer prototype
    assert_eq!(
        typed_call(
            "bk::proto",
            &format!(r#"[{{"__thaw_tjs_bytes__":{{"k":"Uint8Array","b64":"{b64}"}}}}]"#)
        )
        .unwrap(),
        "\"3,false,true\""
    );
    // `k: "Buffer"` keeps the Buffer prototype
    assert_eq!(
        typed_call(
            "bk::proto",
            &format!(r#"[{{"__thaw_tjs_bytes__":{{"k":"Buffer","b64":"{b64}"}}}}]"#)
        )
        .unwrap(),
        "\"3,true,true\""
    );
}

#[test]
fn tj35_branded_buffer_wrapper_node_revives_from_its_data_array() {
    use std::ffi::CString;
    let bundle = CString::new(BYTES_KIND).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"bkd".as_ptr(), bundle.as_ptr()),
        1
    );
    // the Json-routed form carries `data` (a number array) instead of `b64`
    assert_eq!(
        typed_call(
            "bkd::proto",
            r#"[{"__thaw_tjs_bytes__":{"k":"Buffer","data":[1,2]}}]"#
        )
        .unwrap(),
        "\"2,true,true\""
    );
}

#[test]
fn defer_thenable_passes_non_thenables_through() {
    assert_eq!(
        one("var o = {}; console.log(__thaw_defer_thenable(5), __thaw_defer_thenable(null) === null, __thaw_defer_thenable(o) === o, typeof globalThis.__thaw_defer_thenable)"),
        "5 true true function"
    );
}

#[test]
fn defer_thenable_adopts_a_thenable_and_reads_then_once() {
    let (out, _) = main_run(
        "var reads = 0; var t = { get then() { reads++; return function (r) { r(7); }; } };
         (async () => { console.log(await __thaw_defer_thenable(t), reads); })();",
    );
    assert_eq!(out.join("|"), "7 1");
}

#[test]
fn defer_thenable_turns_a_throwing_then_getter_into_a_rejection() {
    let (out, _) = main_run(
        "var bad = { get then() { throw new Error('boom'); } };
         var p = __thaw_defer_thenable(bad);
         console.log(p instanceof Promise);
         (async () => { try { await p; console.log('no'); } catch (e) { console.log('caught', e.message); } })();",
    );
    assert_eq!(out.join("|"), "true|caught boom");
}

#[test]
fn array_values_iterates_a_proxy_through_its_traps() {
    assert_eq!(
        one("var t = []; var p = new Proxy(t, { get(_, k, r) { if (k === 'length') return 2; if (k === '0') return 'a'; if (k === '1') return 'b'; return Reflect.get(t, k, r); } }); console.log([...p].join())"),
        "a,b"
    );
    assert_eq!(
        one("var h = new Proxy([], { get(_, k) { if (k === 'length') return 2; if (k === '1') return 'x'; return Reflect.get(Array.prototype, k); } }); var s = [...h]; console.log(s.length, 0 in s, s[1])"),
        "2 true x"
    );
}

#[test]
fn reflect_get_runs_an_accessor_with_the_receiver() {
    assert_eq!(
        one("var t = { get x() { return this.v; } }; var r = { v: 7 }; console.log(Reflect.get(t, 'x', r), Reflect.get(t, 'x'), Reflect.get(Object.create(t), 'x', r));"),
        "7 undefined 7"
    );
}

#[test]
fn reflect_get_without_receiver_and_data_props_unchanged() {
    assert_eq!(
        one("var a = [1, 2]; console.log(Reflect.get(a, 'length', {}), Reflect.get({ y: 3 }, 'y', {}), Reflect.get(a, 0));"),
        "2 3 1"
    );
}

#[test]
fn reflect_get_inherited_getter_sees_proxy_receiver() {
    assert_eq!(
        one("var seen; var t = Object.create({ get g() { seen = this; return 1; } }); var p = new Proxy(t, { get(tg, k, r) { return Reflect.get(tg, k, r); } }); p.g; console.log(seen === p);"),
        "true"
    );
}

#[test]
fn reflect_set_writes_through_the_receiver() {
    assert_eq!(
        one("var t = { set x(v) { this.y = v; } }; var r = {}; console.log(Reflect.set(t, 'x', 1, r), r.y, t.y);"),
        "true 1 undefined"
    );
}

#[test]
fn reflect_set_returns_false_instead_of_writing_when_refused() {
    assert_eq!(
        one("console.log(Reflect.set(Object.freeze({ a: 1 }), 'a', 2), Reflect.set({ get g() { return 1; } }, 'g', 2), Reflect.set({}, 'x', 1));"),
        "false false true"
    );
    assert_eq!(
        one("var r = Object.preventExtensions({}); console.log(Reflect.set({}, 'z', 1, r), r.z);"),
        "false undefined"
    );
    assert_eq!(
        one("try { Reflect.set(1, 'a', 2); console.log('no'); } catch (e) { console.log(e.constructor.name); }"),
        "TypeError"
    );
}

#[test]
fn reflect_define_property_returns_the_trap_boolean() {
    assert_eq!(
        one("console.log(Reflect.defineProperty(Object.freeze({}), 'a', { value: 1 }), Reflect.defineProperty({}, 'a', { value: 1 }), Reflect.defineProperty(new Proxy({}, { defineProperty() { return false; } }), 'a', { value: 1 }));"),
        "false true false"
    );
}

#[test]
fn reflect_delete_property_reports_the_result() {
    assert_eq!(
        one("console.log(Reflect.deleteProperty(Object.seal({ s: 1 }), 's'), Reflect.deleteProperty({ a: 1 }, 'a'), Reflect.deleteProperty({}, 'missing'));"),
        "false true true"
    );
}

#[test]
fn reflect_set_prototype_of_returns_false_instead_of_throwing() {
    assert_eq!(
        one("console.log(Reflect.setPrototypeOf(Object.preventExtensions({}), {}), Reflect.setPrototypeOf({}, null));"),
        "false true"
    );
    assert_eq!(
        one("var a = {}; var b = Object.create(a); console.log(Reflect.setPrototypeOf(a, b));"),
        "false"
    );
}

#[test]
fn reflect_is_extensible_and_prevent_extensions_exist() {
    assert_eq!(
        one("var q = {}; console.log(Reflect.isExtensible(q), Reflect.preventExtensions(q), Reflect.isExtensible(q));"),
        "true true false"
    );
}

#[test]
fn nonextensible_closure_redefines_its_synthesized_name_and_length() {
    assert_eq!(
        one("var f = () => {}; Object.preventExtensions(f); console.log(Reflect.defineProperty(f, 'length', { value: 0, writable: false, enumerable: false, configurable: true }), Reflect.defineProperty(f, 'name', { value: 'q', configurable: true }), f.name, Reflect.defineProperty(f, 'name', { value: 'r', configurable: true }), Reflect.defineProperty(f, 'extra', { value: 1 }));"),
        "true true q true false"
    );
}

#[test]
fn reflect_construct_takes_the_prototype_from_new_target() {
    assert_eq!(
        one("function F() { this.x = 1; } function A() {} A.prototype.hi = 1; var inst = Reflect.construct(F, [], A); console.log(Object.getPrototypeOf(inst) === A.prototype, inst.x, inst instanceof A);"),
        "true 1 true"
    );
}

#[test]
fn reflect_non_object_arguments_throw_type_error() {
    assert_eq!(
        one("function F() {} try { Reflect.construct(F, 1); console.log('no'); } catch (e) { console.log(e.constructor.name); } try { Reflect.getPrototypeOf(1); console.log('no'); } catch (e) { console.log(e.constructor.name); } try { Reflect.apply(Math.max, null, 1); console.log('no'); } catch (e) { console.log(e.constructor.name); }"),
        "TypeError|TypeError|TypeError"
    );
}

#[test]
fn reflect_set_through_a_proxy_receiver_reaches_the_target() {
    assert_eq!(
        one("var log = []; var p = new Proxy({}, { set(t, k, v, r) { log.push(k); return Reflect.set(t, k, v, r); } }); p.a = 5; console.log(log.join(), p.a);"),
        "a 5"
    );
}

#[test]
fn reflect_set_array_index_and_length() {
    assert_eq!(
        one("var arr = [1, 2]; console.log(Reflect.set(arr, 1, 9), arr[1], Reflect.set(arr, 'length', 1), arr.length);"),
        "true 9 true 1"
    );
}

#[test]
fn process_emit_warning_emits_warning_event_on_next_tick() {
    let out = run_loop(
        "process.on('warning', function (w) { console.log(w.name, w.code, w.count, w.type); });
process.emitWarning('dep', 'DeprecationWarning', 'DEP0XXX');
process.emitWarning('plain');
var e = new Error('m'); e.name = 'MaxListenersExceededWarning'; e.count = 2; e.type = 'x'; process.emitWarning(e);
process.emitWarning('o', { type: 'T', code: 'C' });
console.log('sync');",
    );
    assert_eq!(
        out,
        "sync|DeprecationWarning DEP0XXX undefined undefined|Warning undefined undefined undefined|MaxListenersExceededWarning undefined 2 x|T C undefined undefined"
    );
}

#[test]
fn process_emit_warning_without_listener_writes_stderr_line() {
    let mut it = Interp::new();
    it.run("process.emitWarning('plain');")
        .expect("script runs");
    it.run_event_loop().expect("loop runs");
    let lines: Vec<&String> = it
        .err
        .iter()
        .filter(|l| l.contains("Warning: plain"))
        .collect();
    assert_eq!(lines.len(), 1, "{:?}", it.err);
}

#[test]
fn process_chdir_changes_cwd_and_throws_enoent_on_missing_dir() {
    let original = std::env::current_dir().unwrap();
    let dir = std::env::temp_dir().join(format!("thaw-chdir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap().display().to_string();
    let src = format!(
        "process.chdir({dir:?}); console.log(process.cwd());
         try {{ process.chdir('/nope/thaw-missing'); }} catch (e) {{ console.log(e.code, e.message); }}"
    );
    let out = run(&src);
    std::env::set_current_dir(&original).unwrap();
    assert_eq!(
        out,
        [
            dir.clone(),
            format!(
                "ENOENT ENOENT: no such file or directory, chdir '{dir}' -> '/nope/thaw-missing'"
            ),
        ]
    );
}

#[test]
fn property_is_enumerable_forwards_to_proxy_descriptor() {
    assert_eq!(
        one("var p = new Proxy({a:1}, {}); console.log(p.propertyIsEnumerable('a'), Object.prototype.propertyIsEnumerable.call(p,'b'))"),
        "true false"
    );
}

#[test]
fn property_is_enumerable_uses_proxy_trap_enumerable_flag() {
    assert_eq!(
        one("var p = new Proxy({a:1}, { getOwnPropertyDescriptor(t,k){ return { value: 1, enumerable: false, configurable: true }; } }); console.log(p.propertyIsEnumerable('a'))"),
        "false"
    );
}

#[test]
fn property_is_enumerable_sees_accessor_on_proxy_target() {
    assert_eq!(
        one("var t = {}; Object.defineProperty(t,'a',{get(){return 1},enumerable:true,configurable:true}); var p = new Proxy(t,{}); console.log(Object.keys(p).join(), p.propertyIsEnumerable('a'))"),
        "a true"
    );
}

#[test]
fn performance_now_is_sub_millisecond_and_monotonic() {
    assert_eq!(
        one("let p = performance.now(), mono = true, frac = false;
             for (let i = 0; i < 100000 && !frac; i++) { const n = performance.now(); if (n < p) mono = false; if (n % 1 !== 0) frac = true; p = n; }
             console.log(typeof p, mono, frac);"),
        "number true true"
    );
}

#[test]
fn loop_stats_are_zero_before_the_loop_and_split_idle_after() {
    let (out, _) = main_run(
        "const a = __thaw_loop_stats(); console.log(a[0] === 0 && a[1] === 0);
         // armed from a loop callback
         setTimeout(() => setTimeout(() => { const s = __thaw_loop_stats(); console.log(s[1] > 0, s[0] >= 0); }, 15), 0);",
    );
    assert_eq!(out.join("|"), "true|true true");
}

#[test]
fn push_on_proxy_forwards_to_target_array() {
    assert_eq!(
        one("var t = [1]; var p = new Proxy(t, { get(tg, k) { return tg[k]; }, set(tg, k, v) { tg[k] = v; return true; } }); var n = p.push(2, 3); console.log(n, t.length, t.join())"),
        "3 3 1,2,3"
    );
}

#[test]
fn array_slice_call_on_typed_array_copies_elements() {
    assert_eq!(
        one("const u = new Uint8Array([1, 2, 3]); console.log(JSON.stringify(Array.prototype.slice.call(u)))"),
        "[1,2,3]"
    );
    assert_eq!(
        one("const u = new Uint8Array([1, 2, 3]); console.log(JSON.stringify([Array.prototype.slice.call(u, -2), Array.prototype.slice.call(u, 1, 2), Array.prototype.slice.call(u, 1, -1)]))"),
        "[[2,3],[2],[2]]"
    );
}

#[test]
fn array_slice_call_on_array_like_objects_and_arguments() {
    assert_eq!(
        one("console.log(JSON.stringify(Array.prototype.slice.call({ 0: 'a', 1: 'b', length: 2 })))"),
        r#"["a","b"]"#
    );
    assert_eq!(
        one("function f() { return Array.prototype.slice.call(arguments, 1); } console.log(JSON.stringify(f(1, 2, 3)))"),
        "[2,3]"
    );
}

#[test]
fn array_slice_call_on_buffer_and_string() {
    assert_eq!(
        one("console.log(JSON.stringify(Array.prototype.slice.call(Buffer.from([5, 6, 7]), 1)))"),
        "[6,7]"
    );
    assert_eq!(
        one("console.log(JSON.stringify(Array.prototype.slice.call('abc', 1)))"),
        r#"["b","c"]"#
    );
}

#[test]
fn array_slice_call_result_is_a_fresh_array_not_a_view() {
    assert_eq!(
        one("const u = new Uint8Array([1, 2]); const r = Array.prototype.slice.call(u); r[0] = 9; console.log(u[0], Array.isArray(r), r.length)"),
        "1 true 2"
    );
}

#[test]
fn push_on_plain_array_is_unchanged() {
    assert_eq!(
        one("var a = [1]; var n = a.push(2, 3); console.log(n, a.join())"),
        "3 1,2,3"
    );
}

#[test]
fn push_on_non_array_non_proxy_still_throws() {
    assert_eq!(
        one("try { Array.prototype.push.call({}, 1); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError push on non-array"
    );
}

#[test]
fn pop_on_proxy_forwards_to_target_array() {
    assert_eq!(
        one("var t = [1, 2]; var p = new Proxy(t, { get(tg, k) { return tg[k]; }, set(tg, k, v) { tg[k] = v; return true; }, deleteProperty(tg, k) { delete tg[k]; return true; } }); var v = p.pop(); console.log(v, t.length, t.join())"),
        "2 1 1"
    );
}

#[test]
fn process_argv_starts_with_the_host_binary() {
    let host = std::env::args().next().expect("test binary has argv[0]");
    assert_eq!(
        one(&format!(
            "console.log(process.argv.length >= 1, process.argv[0] === {host:?}, typeof process.argv[0])"
        )),
        "true true string"
    );
}

/// A fresh fixture directory for the disk CommonJS loader tests (removed and recreated per run).
fn main_file_fixture(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("thaw-main-file-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn main_file_loader_resolves_relative_json_and_index() {
    let dir = main_file_fixture("resolve");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(
        dir.join("main.js"),
        "const h = require('./h');\nconst d = require('./d.json');\nconst i = require('./sub');\n\
         console.log([h, d.k, i.name, __filename, __dirname, require.main === module, this === module.exports].join('|'));\n",
    )
    .unwrap();
    std::fs::write(dir.join("h.js"), "module.exports = 'helper';\n").unwrap();
    std::fs::write(dir.join("d.json"), "{\"k\": 1}").unwrap();
    std::fs::write(dir.join("sub").join("index.js"), "exports.name = 'sub';\n").unwrap();
    let main = dir.join("main.js").display().to_string();
    let mut it = Interp::new();
    assert_eq!(it.run_main_file(&main).ok(), Some(0));
    let d = dir.display();
    assert_eq!(
        it.out.join("\n"),
        format!("helper|1|sub|{d}/main.js|{d}|true|true")
    );
}

#[test]
fn main_file_loader_caches_cycles_and_reports_missing() {
    let dir = main_file_fixture("cycle");
    std::fs::write(
        dir.join("main.js"),
        "const a = require('./a');\n\
         try { require('./nope'); } catch (e) { console.log(e.code, e.message); }\n\
         console.log([a.before, a.fromB, a.done, require('./a') === a].join(','));\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("a.js"),
        "exports.before = true;\nconst b = require('./b');\nexports.fromB = b.sawA;\nexports.done = true;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.js"),
        "const a = require('./a');\nexports.sawA = a.before;\n",
    )
    .unwrap();
    let mut it = Interp::new();
    assert_eq!(
        it.run_main_file(&dir.join("main.js").display().to_string())
            .ok(),
        Some(0)
    );
    assert_eq!(
        it.out,
        vec![
            "MODULE_NOT_FOUND Cannot find module './nope'".to_string(),
            "true,true,true,true".to_string(),
        ]
    );
}

#[test]
fn main_file_loader_strips_shebang_and_binds_this_to_exports() {
    let dir = main_file_fixture("shebang");
    std::fs::write(
        dir.join("main.js"),
        "console.log(require('./s').kind, require('./s').self);\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("s.js"),
        "#!/usr/bin/env node\nexports.kind = typeof module.exports;\nexports.self = this === module.exports;\n",
    )
    .unwrap();
    let mut it = Interp::new();
    assert_eq!(
        it.run_main_file(&dir.join("main.js").display().to_string())
            .ok(),
        Some(0)
    );
    assert_eq!(it.out, vec!["object true".to_string()]);
}

const PROBE_BUNDLE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"tpa/index.js": function(module, exports, require) {
  exports.u8 = new Uint8Array([1, 2]);
  exports.buf = Buffer.from([1]);
  exports.obj = { a: 1 };
},
};
var __thaw_bundle_require_maps = { "tpa/index.js": {}, };
var __thaw_bundle_entry_key = "tpa/index.js";
})(globalThis.require);"#;

#[test]
fn typed_array_handle_probe_reports_name_and_values() {
    use crate::host::{thaw_tjs_get_global, thaw_tjs_handle_typed_array_probe, thaw_tjs_register};
    use std::ffi::{CStr, CString};
    let source = CString::new(PROBE_BUNDLE).unwrap();
    assert_eq!(thaw_tjs_register(c"tpa".as_ptr(), source.as_ptr()), 1);
    let probe_of = |export: &str| {
        let handle = thaw_tjs_get_global(CString::new(format!("tpa::{export}")).unwrap().as_ptr());
        assert!(handle > 0, "{export}");
        let text = thaw_tjs_handle_typed_array_probe(handle as u64);
        if text.is_null() {
            return None;
        }
        let owned = unsafe { CStr::from_ptr(text) }
            .to_str()
            .unwrap()
            .to_string();
        unsafe { thaw_arena::destroy_string(text) };
        Some(owned)
    };
    assert_eq!(
        probe_of("u8").as_deref(),
        Some(r#"{"n":"Uint8Array","v":["1","2"],"e":[]}"#)
    );
    // a Buffer prints itself, and a plain object is not a typed array
    assert_eq!(probe_of("buf"), None);
    assert_eq!(probe_of("obj"), None);
}

const LENGTH_BUNDLE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"lenb/index.js": function(module, exports, require) {
  exports.len = function (f) { return f.length; };
},
};
var __thaw_bundle_require_maps = { "lenb/index.js": {}, };
var __thaw_bundle_entry_key = "lenb/index.js";
})(globalThis.require);"#;

#[test]
fn compiled_callback_function_length_is_its_declared_arity() {
    use crate::host::{thaw_tjs_register_callback, Native, Package};
    let four = thaw_tjs_register_callback(sum_adapter as *const _, std::ptr::null(), 4);
    let mut p = Package::load(LENGTH_BUNDLE).expect("loads");
    assert_eq!(
        p.call("len", &[callback_marker(four)]).ok(),
        Some(Native::Number(4.0))
    );
}

#[test]
fn compiled_rest_callback_length_leaves_out_the_rest_parameter() {
    use crate::host::*;
    let source = std::ffi::CString::new(LENGTH_BUNDLE).unwrap();
    assert_eq!(thaw_tjs_register(c"lenb".as_ptr(), source.as_ptr()), 1);
    // three declared parameters, the last one a rest parameter: `length` is 2
    let made = thaw_tjs_register_native_callback_graph(
        graph_adapter as *const _,
        std::ptr::null(),
        0,
        3,
        0,
        std::ptr::null(),
        1,
    );
    assert!(made.error.is_null() && made.value > 0);
    let callback = made.value;
    let function = thaw_tjs_get_global(c"lenb::len".as_ptr());
    assert!(function > 0);
    let wire = std::ffi::CString::new(format!(
        "{{\"root\":{{\"r\":0}},\"nodes\":[{{\"a\":[{{\"r\":1}}]}},{{\"hdl\":{callback}}}]}}"
    ))
    .unwrap();
    thaw_tjs_retain_handle(callback as i64);
    let result =
        thaw_tjs_call_handle_handle_graph_args_result(function as u64, wire.as_ptr(), false);
    assert!(result.error.is_null());
    assert_eq!(
        unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(result.value)) }
            .to_str()
            .unwrap(),
        "2"
    );
}

const NUMBER_BUNDLE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"nfn/index.js": function(module, exports, require) {
  exports.num = function (x) { return x; };
},
};
var __thaw_bundle_require_maps = { "nfn/index.js": {}, };
var __thaw_bundle_entry_key = "nfn/index.js";
})(globalThis.require);"#;

#[test]
fn typed_call_result_marks_non_finite_numbers() {
    use std::ffi::CString;
    let bundle = CString::new(NUMBER_BUNDLE).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"nfn".as_ptr(), bundle.as_ptr()),
        1
    );
    let sentinel = |name: &str| format!("{{\"$__thaw_non_finite$\":\"{name}\"}}");
    assert_eq!(
        typed_call("nfn::num", r#"[{"__thaw_tjs_num__":"NaN"}]"#).unwrap(),
        sentinel("NaN")
    );
    assert_eq!(
        typed_call("nfn::num", r#"[{"__thaw_tjs_num__":"Infinity"}]"#).unwrap(),
        sentinel("Infinity")
    );
    assert_eq!(
        typed_call("nfn::num", r#"[{"__thaw_tjs_num__":"-Infinity"}]"#).unwrap(),
        sentinel("-Infinity")
    );
    // finite numbers are unchanged
    assert_eq!(typed_call("nfn::num", "[1.5]").unwrap(), "1.5");
}

#[test]
fn typed_result_nested_non_finite_numbers_use_the_std_sentinel() {
    register_exit_package(
        "tjsnf2",
        "  exports.mk = function () { return { a: NaN, b: Infinity, c: -Infinity, ok: 1, l: [NaN, Infinity], n: { z: -Infinity } }; };",
    );
    let s = |name: &str| format!("{{\"$__thaw_non_finite$\":\"{name}\"}}");
    assert_eq!(
        typed_call("tjsnf2::mk", "[]").unwrap(),
        format!(
            r#"{{"a":{},"b":{},"c":{},"ok":1,"l":[{},{}],"n":{{"z":{}}}}}"#,
            s("NaN"),
            s("Infinity"),
            s("-Infinity"),
            s("NaN"),
            s("Infinity"),
            s("-Infinity"),
        )
    );
}

#[test]
fn typed_result_keeps_an_undefined_own_property_as_the_marker() {
    register_exit_package(
        "tjskeep",
        "  exports.mk = function () { return { a: 1, gone: undefined, n: { z: undefined }, l: [undefined, { y: undefined }] }; };",
    );
    let marker = r#"{"$__thaw_napi_undefined$":true}"#;
    assert_eq!(
        typed_call("tjskeep::mk", "[]").unwrap(),
        format!(r#"{{"a":1,"gone":{marker},"n":{{"z":{marker}}},"l":[null,{{"y":{marker}}}]}}"#)
    );
}

#[test]
fn typed_result_top_level_and_array_undefined_are_unchanged() {
    register_exit_package(
        "tjsku",
        "  exports.u = function () { return undefined; };\n  exports.a = function () { return [undefined, 1]; };",
    );
    assert_eq!(
        typed_call("tjsku::u", "[]").unwrap(),
        r#"{"$__thaw_napi_undefined$":true}"#
    );
    assert_eq!(typed_call("tjsku::a", "[]").unwrap(), "[null,1]");
}

#[test]
fn typed_arg_undefined_marker_round_trips_as_an_own_property() {
    register_exit_package(
        "tjsarg",
        "  exports.k = function (o) { return Object.keys(o).join() + ':' + (o.a === undefined); };",
    );
    assert_eq!(
        typed_call("tjsarg::k", r#"[{"a":{"$__thaw_napi_undefined$":true}}]"#).unwrap(),
        "\"a:true\""
    );
}

#[test]
fn async_result_keeps_an_undefined_own_property() {
    use crate::host::thaw_tjs_watch_promise;
    register_exit_package(
        "tjskpend",
        "  exports.later = function () { return new Promise(function (r) { globalThis.fireObj = r; }); };\n  exports.fire = function () { globalThis.fireObj({ gone: undefined, a: 1 }); };",
    );
    let marker = typed_call("\u{4}tjskpend::later", "[]").expect("pending marker");
    let id: i64 = marker
        .strip_prefix(r#"{"__thaw_tjs_pending__":"#)
        .and_then(|rest| rest.strip_suffix('}'))
        .expect("marker shape")
        .parse()
        .expect("handle id");
    SETTLED.with(|s| s.borrow_mut().clear());
    thaw_tjs_watch_promise(id, std::ptr::null_mut(), record_settle);
    typed_call("tjskpend::fire", "[]").expect("fire");
    let marker = r#"{"$__thaw_napi_undefined$":true}"#;
    assert_eq!(
        SETTLED.with(|s| s.borrow().clone()),
        vec![(Some(format!(r#"{{"gone":{marker},"a":1}}"#)), None)]
    );
}

// ----- process.exit / exitCode across packages (host.rs terminate_process) -----

thread_local! {
    /// The status the exit hook received (the hook replaces `_exit` in these tests).
    static HOOK_EXIT: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
}

fn record_hook_exit(code: i32) {
    HOOK_EXIT.with(|c| c.set(Some(code)));
}

fn exit_hook_on() {
    HOOK_EXIT.with(|c| c.set(None));
    crate::host::set_exit_hook(Some(record_hook_exit));
}

/// Registers a package `name` whose entry module runs `body` (`exports` is in scope).
fn register_exit_package(name: &str, body: &str) {
    use crate::host::thaw_tjs_register;
    use std::ffi::CString;
    let bundle = format!(
        r#"module.exports = (function(require){{
var __thaw_bundle_factories = {{ "{name}/index.js": function(module, exports, require) {{
{body}
}}, }};
var __thaw_bundle_require_maps = {{ "{name}/index.js": {{}}, }};
var __thaw_bundle_entry_key = "{name}/index.js";
}})(globalThis.require);"#
    );
    let (name, source) = (CString::new(name).unwrap(), CString::new(bundle).unwrap());
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
}

#[test]
fn exit_from_another_package_uses_the_shared_exit_code() {
    register_exit_package(
        "xa",
        "exports.setCode = function () { process.exitCode = 9; };",
    );
    register_exit_package("xb", "exports.quit = function () { process.exit(); };");
    exit_hook_on();
    typed_call("xa::setCode", "[]").expect("setCode");
    assert!(typed_call("xb::quit", "[]").is_err());
    assert_eq!(HOOK_EXIT.with(std::cell::Cell::get), Some(9));
}

#[test]
fn exit_listener_in_another_realm_runs_with_the_exit_code() {
    register_exit_package(
        "ya",
        "process.on('exit', function (c) { globalThis.seen = c; });
         exports.seen = function () { return globalThis.seen; };",
    );
    register_exit_package("yb", "exports.quit = function () { process.exit(4); };");
    exit_hook_on();
    assert!(typed_call("yb::quit", "[]").is_err());
    assert_eq!(HOOK_EXIT.with(std::cell::Cell::get), Some(4));
    assert_eq!(typed_call("ya::seen", "[]"), Ok("4".to_string()));
}

#[test]
fn exit_inside_a_timer_terminates_the_event_loop() {
    use crate::host::thaw_tjs_run_event_loop;
    register_exit_package(
        "tm",
        "exports.arm = function () { setTimeout(function () { process.exit(7); }, 1); };",
    );
    exit_hook_on();
    typed_call("tm::arm", "[]").expect("arm");
    assert_eq!(thaw_tjs_run_event_loop(), 7);
    assert_eq!(HOOK_EXIT.with(std::cell::Cell::get), Some(7));
}

#[test]
fn exit_is_not_catchable_by_js_try_catch() {
    register_exit_package(
        "ct",
        "exports.guard = function () { try { process.exit(2); } catch (e) { globalThis.caught = 1; } };
         exports.caught = function () { return globalThis.caught === 1; };",
    );
    exit_hook_on();
    assert!(typed_call("ct::guard", "[]").is_err());
    assert_eq!(HOOK_EXIT.with(std::cell::Cell::get), Some(2));
    assert_eq!(typed_call("ct::caught", "[]"), Ok("false".to_string()));
}

#[test]
fn natural_end_returns_the_shared_exit_code() {
    use crate::host::thaw_tjs_run_event_loop;
    register_exit_package("nt", "exports.set = function () { process.exitCode = 5; };");
    typed_call("nt::set", "[]").expect("set");
    assert_eq!(thaw_tjs_run_event_loop(), 5);
}

#[test]
fn object_literal_anonymous_function_takes_its_key_as_name() {
    assert_eq!(
        one("var o = { describe: function(){}, go: () => 1, named: function inner(){}, paren: (function(){}), k: class {} }; console.log([o.describe.name, o.go.name, o.named.name, o.paren.name, o.k.name].join())"),
        "describe,go,inner,paren,k"
    );
    assert_eq!(
        one("var o = { describe: function(){} }; console.log(o.describe)"),
        "[Function: describe]"
    );
}

#[test]
fn stdout_write_takes_typed_array_and_arraybuffer_bytes() {
    let mut it = Interp::new();
    let src = r#"
        process.stdout.write(Buffer.from('héllo\n'));
        process.stdout.write(new Uint8Array([104, 105, 10]));
        process.stdout.write(new Uint8Array([0, 104, 105, 10, 9]).subarray(1, 4));
        process.stdout.write(new Uint8Array([104, 105, 10]).buffer);
        process.stdout.write('text\n');
    "#;
    it.run_main(src)
        .unwrap_or_else(|_| panic!("script must run"));
    assert_eq!(it.out, vec!["héllo", "hi", "hi", "hi", "text"]);
}

#[test]
fn stdout_byte_write_calls_its_callback_after_the_sync_code() {
    let mut it = Interp::new();
    let src = r#"
        let order = [];
        process.stdout.write(Buffer.from('ok\n'), () => order.push('cb'));
        order.push('sync');
        process.on('exit', () => console.log('order=' + order.join(',')));
    "#;
    it.run_main(src)
        .unwrap_or_else(|_| panic!("script must run"));
    assert_eq!(it.out, vec!["ok", "order=sync,cb"]);
}

// ----- multi-package fairness (host.rs terminate loop time slices) -----

#[test]
fn package_with_endless_interval_does_not_starve_a_timer_in_another_package() {
    use crate::host::thaw_tjs_run_event_loop;
    // "rra" is registered first: before the fix it held the loop for its whole 2 s self-timeout, so "rrb" fired late
    register_exit_package(
        "rra",
        "exports.arm = function () { var t0 = Date.now();
         var t = setInterval(function () {
           if (process.exitCode === 3) clearInterval(t);
           else if (Date.now() - t0 > 2000) { clearInterval(t); process.exitCode = 99; } }, 1); };",
    );
    register_exit_package(
        "rrb",
        "exports.arm = function () { var t0 = Date.now();
         setTimeout(function () { process.exitCode = Date.now() - t0 < 1000 ? 3 : 98; }, 10); };",
    );
    typed_call("rra::arm", "[]").expect("arm a");
    typed_call("rrb::arm", "[]").expect("arm b");
    assert_eq!(thaw_tjs_run_event_loop(), 3);
}

#[test]
fn listening_server_shape_pending_work_does_not_starve_a_later_package() {
    use crate::host::thaw_tjs_run_event_loop;
    // a re-arming timer chain (the net pump shape) instead of an interval
    register_exit_package(
        "rsa",
        "exports.arm = function () { var t0 = Date.now();
         function pump() {
           if (process.exitCode === 3) return;
           if (Date.now() - t0 > 2000) { process.exitCode = 99; return; }
           setTimeout(pump, 1);
         }
         setTimeout(pump, 1); };",
    );
    register_exit_package(
        "rsb",
        "exports.arm = function () { var t0 = Date.now();
         setTimeout(function () { process.exitCode = Date.now() - t0 < 1000 ? 3 : 98; }, 10); };",
    );
    typed_call("rsa::arm", "[]").expect("arm a");
    typed_call("rsb::arm", "[]").expect("arm b");
    assert_eq!(thaw_tjs_run_event_loop(), 3);
}

#[test]
fn process_exit_in_a_later_package_ends_the_run_while_another_loops_forever() {
    use crate::host::thaw_tjs_run_event_loop;
    register_exit_package(
        "rxa",
        "exports.arm = function () { var t0 = Date.now();
         var t = setInterval(function () { if (Date.now() - t0 > 2000) clearInterval(t); }, 1);
         process.on('exit', function (c) { globalThis.seen = c; }); };
         exports.seen = function () { return globalThis.seen; };",
    );
    register_exit_package(
        "rxb",
        "exports.arm = function () { setTimeout(function () { process.exit(7); }, 10); };",
    );
    exit_hook_on();
    typed_call("rxa::arm", "[]").expect("arm a");
    typed_call("rxb::arm", "[]").expect("arm b");
    assert_eq!(thaw_tjs_run_event_loop(), 7);
    assert_eq!(HOOK_EXIT.with(std::cell::Cell::get), Some(7));
    assert_eq!(typed_call("rxa::seen", "[]"), Ok("7".to_string()));
}

#[test]
fn timers_within_a_package_keep_their_order_while_another_package_keeps_the_loop_busy() {
    use crate::host::thaw_tjs_run_event_loop;
    // "qtb" is registered first so it is sliced in front of "ptb" on every round
    register_exit_package(
        "qtb",
        "exports.arm = function () { var t0 = Date.now();
         var t = setInterval(function () { if (Date.now() - t0 > 60) clearInterval(t); }, 1); };",
    );
    register_exit_package(
        "ptb",
        "globalThis.log = [];
         exports.log = function () { return globalThis.log.join(); };
         exports.arm = function () {
           process.nextTick(function () { globalThis.log.push('tick'); });
           setTimeout(function () { globalThis.log.push('t30'); }, 30);
           setTimeout(function () { globalThis.log.push('t10a'); }, 10);
           setTimeout(function () { globalThis.log.push('t10b'); }, 10);
           setTimeout(function () { globalThis.log.push('t20'); }, 20);
         };",
    );
    typed_call("qtb::arm", "[]").expect("arm q");
    typed_call("ptb::arm", "[]").expect("arm p");
    assert_eq!(thaw_tjs_run_event_loop(), 0);
    assert_eq!(
        typed_call("ptb::log", "[]"),
        Ok("\"tick,t10a,t10b,t20,t30\"".to_string())
    );
}

#[test]
fn promise_jobs_drain_between_timers_while_another_package_keeps_the_loop_busy() {
    use crate::host::thaw_tjs_run_event_loop;
    register_exit_package(
        "qpj",
        "exports.arm = function () { var t0 = Date.now();
         var t = setInterval(function () { if (Date.now() - t0 > 60) clearInterval(t); }, 1); };",
    );
    register_exit_package(
        "ppj",
        "globalThis.log = [];
         exports.log = function () { return globalThis.log.join(); };
         exports.arm = function () {
           setTimeout(function () { globalThis.log.push('a'); Promise.resolve().then(function () { globalThis.log.push('a micro'); }); }, 5);
           setTimeout(function () { globalThis.log.push('b'); Promise.resolve().then(function () { globalThis.log.push('b micro'); }); }, 15);
           setTimeout(function () { globalThis.log.push('c'); }, 25);
         };",
    );
    typed_call("qpj::arm", "[]").expect("arm q");
    typed_call("ppj::arm", "[]").expect("arm p");
    assert_eq!(thaw_tjs_run_event_loop(), 0);
    assert_eq!(
        typed_call("ppj::log", "[]"),
        Ok("\"a,a micro,b,b micro,c\"".to_string())
    );
}

#[test]
fn a_yielding_package_gets_no_early_before_exit_or_exit() {
    use crate::host::thaw_tjs_run_event_loop;
    // "qlv" keeps the loop busy so "elv" yields several times before its own 12 ms timer fires
    register_exit_package(
        "qlv",
        "exports.arm = function () { var t0 = Date.now();
         var t = setInterval(function () { if (Date.now() - t0 > 60) clearInterval(t); }, 1); };",
    );
    register_exit_package(
        "elv",
        "globalThis.b = 0; globalThis.e = 0;
         process.on('beforeExit', function () { globalThis.b++; });
         process.on('exit', function () { globalThis.e++; });
         exports.counts = function () { return globalThis.b + ',' + globalThis.e; };
         exports.arm = function () { setTimeout(function () {}, 12); };",
    );
    typed_call("qlv::arm", "[]").expect("arm q");
    typed_call("elv::arm", "[]").expect("arm e");
    assert_eq!(thaw_tjs_run_event_loop(), 0);
    assert_eq!(typed_call("elv::counts", "[]"), Ok("\"1,1\"".to_string()));
}

#[test]
fn a_realm_with_no_other_package_never_yields_its_slice() {
    // white-box: a slice end with no other loaded package must not yield, so the run is unchanged
    let mut it = Interp::new();
    it.run("setTimeout(function () { console.log('done'); }, 20);")
        .expect("schedules");
    it.slice_end = Some(std::time::Instant::now());
    assert_eq!(it.run_event_loop().expect("loop"), 0);
    assert!(!it.yielded);
    assert_eq!(it.out.join("|"), "done");
}

#[test]
fn not_callable_error_names_the_value_kind() {
    let mut it = Interp::new();
    let cases = [
        (Value::Undefined, "TypeError: undefined is not a function"),
        (
            Value::Null,
            "TypeError: object is not a function (value null)",
        ),
        (
            Value::str("abc"),
            "TypeError: string is not a function (value 'abc')",
        ),
    ];
    for (v, want) in cases {
        match it.call(&v, &Value::Undefined, &[]) {
            Err(Ctl::Throw(e)) => assert_eq!(it.to_str(&e).as_ref(), want),
            _ => panic!("expected a TypeError throw for {want}"),
        }
    }
}

#[test]
fn not_callable_error_names_the_constructor_of_an_object() {
    let mut it = Interp::new();
    let obj = it.run("({a: 1})").unwrap();
    match it.call(&obj, &Value::Undefined, &[]) {
        Err(Ctl::Throw(e)) => {
            let msg = it.to_str(&e).to_string();
            assert!(
                msg.starts_with("TypeError: object is not a function (constructor Object, value "),
                "{msg}"
            );
        }
        _ => panic!("expected a TypeError throw"),
    }
}

/// Runs the package setup and the event loop on a fresh thread (its own package table) and fails instead of
/// hanging when the loop never ends.
fn run_loop_or_fail(setup: impl FnOnce() + Send + 'static) -> i32 {
    use std::sync::mpsc::RecvTimeoutError;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(move || {
            setup();
            let _ = sender.send(crate::host::thaw_tjs_run_event_loop());
        })
        .expect("spawn");
    match receiver.recv_timeout(std::time::Duration::from_secs(20)) {
        Ok(code) => code,
        Err(RecvTimeoutError::Timeout) => {
            panic!("event loop did not end: a platform job or timer leaked")
        }
        Err(RecvTimeoutError::Disconnected) => panic!("package setup panicked"),
    }
}

/// Serializes every test that starts a zlib job or reads the process-wide zlib tables (`zlib::active()`): those
/// tables are shared by all threads, so a parallel test's pending job would otherwise leak into another's assertion.
/// Held for the whole test body; a poisoned lock (an earlier panic) is recovered so later tests still run.
pub(crate) static ZLIB_GLOBAL_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn zlib_global_guard() -> std::sync::MutexGuard<'static, ()> {
    ZLIB_GLOBAL_TESTS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Owner package: starts one gzip job and sets exitCode 3 once its own poll collects the result.
const ZLIB_OWNER_BODY: &str = r#"exports.go = function () {
  var job = __thaw_zlib_async_start('compress', 'gzip', new Uint8Array(1 << 18), '');
  (function wait() {
    var done = JSON.parse(__thaw_zlib_async_poll());
    if (!done.some(function (d) { return d.handle === job; })) return setTimeout(wait, 1);
    var bytes = new Uint8Array(__thaw_zlib_async_take(job));
    process.exitCode = bytes.length > 0 && bytes[0] === 31 ? 3 : 4;
  })();
};"#;

/// Foreign package: polls the shared zlib table every turn (as a realm whose shim polls would) until the owner
/// sets exitCode 3, or gives up after 10 s with 99.
const ZLIB_FOREIGN_POLLER_BODY: &str = r#"exports.arm = function () { var t0 = Date.now(); var t = setInterval(function () {
  __thaw_zlib_async_poll();
  if (process.exitCode === 3) clearInterval(t);
  else if (Date.now() - t0 > 10000) { clearInterval(t); process.exitCode = 99; } }, 1); };"#;

#[test]
fn zlib_job_completes_when_another_package_polls_the_table_first() {
    let _zlib_guard = zlib_global_guard();
    let code = run_loop_or_fail(|| {
        register_exit_package("zfa", ZLIB_FOREIGN_POLLER_BODY);
        register_exit_package("zfo", ZLIB_OWNER_BODY);
        typed_call("zfa::arm", "[]").expect("arm");
        typed_call("zfo::go", "[]").expect("go");
    });
    assert_eq!(code, 3);
    assert!(!crate::zlib::active());
}

#[test]
fn zlib_jobs_are_delivered_once_each_to_their_owner_despite_a_foreign_poller() {
    let _zlib_guard = zlib_global_guard();
    // The owner starts two jobs; a foreign package polls the table; each owner callback must run exactly once.
    let owner = r#"exports.go = function () {
  var jobs = [__thaw_zlib_async_start('compress', 'gzip', new Uint8Array(1 << 18), ''),
              __thaw_zlib_async_start('compress', 'deflate', new Uint8Array(1 << 18), '')];
  var hits = 0;
  (function wait() {
    JSON.parse(__thaw_zlib_async_poll()).forEach(function (d) {
      if (jobs.indexOf(d.handle) >= 0) { hits++; __thaw_zlib_async_take(d.handle); }
    });
    if (hits < 2) return setTimeout(wait, 1);
    process.exitCode = hits === 2 ? 3 : 4;
  })();
};"#;
    let code = run_loop_or_fail(move || {
        register_exit_package("zgp", ZLIB_FOREIGN_POLLER_BODY);
        register_exit_package("zgo", owner);
        typed_call("zgp::arm", "[]").expect("arm");
        typed_call("zgo::go", "[]").expect("go");
    });
    assert_eq!(code, 3);
    assert!(!crate::zlib::active());
}

#[test]
fn zlib_chain_with_a_pending_net_connect_in_another_package_ends() {
    let _zlib_guard = zlib_global_guard();
    // "tnb" (registered first) polls the zlib table and lives 100 ms; "tna" connects a loopback socket and runs a
    // gzip round trip whose second job starts inside the first job's callback, then tears everything down.
    let owner = r#"exports.go = function () {
  var l = __thaw_net_listen('127.0.0.1', 0).split(':');
  var op = __thaw_net_connect_start('127.0.0.1', Number(l[2]));
  var conn = null, acc = null, input = new Uint8Array(1 << 16);
  function job(operation, format, bytes, next) {
    var h = __thaw_zlib_async_start(operation, format, bytes, '');
    (function wait() {
      var done = JSON.parse(__thaw_zlib_async_poll()).filter(function (d) { return d.handle === h; });
      if (!done.length) return setTimeout(wait, 1);
      if (!done[0].ok) { process.exitCode = 6; return; }
      next(new Uint8Array(__thaw_zlib_async_take(h)));
    })();
  }
  (function connect() {
    if (!conn) { var r = __thaw_async_poll(op); if (r === 'pending') return setTimeout(connect, 1); conn = r.split(':'); }
    if (!acc) { var a = __thaw_net_poll_accept(Number(l[1])); if (a === 'err:pending') return setTimeout(connect, 1); acc = a.split(':'); }
    __thaw_net_write(Number(conn[1]), Buffer.from('ping').toString('hex'));
    job('compress', 'gzip', input, function (packed) {
      job('decompress', 'gzip', packed, function (plain) {
        process.exitCode = plain.length === input.length ? 0 : 7;
        __thaw_net_destroy(Number(conn[1]));
        __thaw_net_destroy(Number(acc[1]));
        __thaw_net_close_listener(Number(l[1]));
      });
    });
  })();
};"#;
    let code = run_loop_or_fail(move || {
        register_exit_package(
            "tnb",
            "exports.arm = function () { var t0 = Date.now(); var t = setInterval(function () {
               __thaw_zlib_async_poll();
               if (Date.now() - t0 > 100) clearInterval(t); }, 1); };",
        );
        register_exit_package("tna", owner);
        typed_call("tnb::arm", "[]").expect("arm");
        typed_call("tna::go", "[]").expect("go");
    });
    assert_eq!(code, 0);
    assert!(!crate::zlib::active());
}

#[test]
fn zlib_job_is_collected_only_by_the_realm_that_started_it() {
    let _zlib_guard = zlib_global_guard();
    let mut owner = Interp::new();
    let mut other = Interp::new();
    owner
        .run("globalThis.h = __thaw_zlib_async_start('compress', 'deflate', new Uint8Array([1, 2, 3]), '');")
        .unwrap_or_else(|_| panic!("start"));
    other
        .run("console.log(__thaw_zlib_async_poll());")
        .unwrap_or_else(|_| panic!("foreign poll"));
    assert_eq!(other.out.last().map(String::as_str), Some("[]"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        owner
            .run("console.log(__thaw_zlib_async_poll());")
            .unwrap_or_else(|_| panic!("owner poll"));
        let line = owner.out.last().cloned().unwrap_or_default();
        if line.contains("\"ok\":true") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "owner never saw its job finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    owner
        .run("console.log(__thaw_zlib_async_take(h).byteLength);")
        .unwrap_or_else(|_| panic!("take"));
    let size: usize = owner.out.last().and_then(|l| l.parse().ok()).unwrap_or(0);
    assert!(size > 0, "deflate result must be non-empty");
    assert!(!crate::zlib::active());
}

/// The registry shim's zlib hook shape: one job table per realm, and a collected result with no waiting callback is
/// taken so it cannot stay in DONE (keeps `zlib::active()` true for every package).
const ZLIB_SHIM_HOOK: &str = r#"
var jobs = globalThis.__thaw_zlib_jobs || (globalThis.__thaw_zlib_jobs = new Map());
var prev = globalThis.__thaw_poll_platform_events;
globalThis.__thaw_poll_platform_events = function () {
  if (prev) prev();
  JSON.parse(__thaw_zlib_async_poll()).forEach(function (r) {
    var j = jobs.get(r.handle);
    if (!j) { __thaw_zlib_async_take(r.handle); return; }
    jobs.delete(r.handle);
    j(r.ok ? new Uint8Array(__thaw_zlib_async_take(r.handle)) : null);
  });
};
function zjob(op, bytes, cb) { jobs.set(__thaw_zlib_async_start(op, 'gzip', bytes, ''), cb); }
"#;

/// Loopback listen + connect + accept, polled with setTimeout; calls `then(l, conn, acc)` once accepted.
const NET_OPEN_BODY: &str = r#"
function netOpen(then) {
  var l = __thaw_net_listen('127.0.0.1', 0).split(':');
  var op = __thaw_net_connect_start('127.0.0.1', Number(l[2]));
  var conn = null, acc = null;
  (function connect() {
    if (!conn) { var r = __thaw_async_poll(op); if (r === 'pending') return setTimeout(connect, 1); conn = r.split(':'); }
    if (!acc) { var a = __thaw_net_poll_accept(Number(l[1])); if (a === 'err:pending') return setTimeout(connect, 1); acc = a.split(':'); }
    then(l, conn, acc);
  })();
}
"#;

#[test]
fn zlib_hook_gzip_callback_with_net_write_destroy_close_ends() {
    let _zlib_guard = zlib_global_guard();
    let owner = format!(
        "{ZLIB_SHIM_HOOK}{NET_OPEN_BODY}\n{}",
        r#"exports.go = function () {
  netOpen(function (l, conn, acc) {
    var t = setInterval(function () {}, 2);
    zjob('compress', new Uint8Array(4 << 20).fill(97), function (packed) {
      zjob('decompress', packed, function (plain) {
        __thaw_net_write(Number(conn[1]), Buffer.from('x').toString('hex'));
        queueMicrotask(function () {
          process.exitCode = plain && plain.length === 4 << 20 ? 0 : 5;
          clearInterval(t);
          __thaw_net_destroy(Number(conn[1]));
          __thaw_net_destroy(Number(acc[1]));
          __thaw_net_close_listener(Number(l[1]));
        });
      });
    });
  });
};"#
    );
    let code = run_loop_or_fail(move || {
        register_exit_package(
            "t8b",
            "exports.arm = function () { var t0 = Date.now(); var t = setInterval(function () {
               if (Date.now() - t0 > 100) clearInterval(t); }, 1); };",
        );
        register_exit_package("t8a", &owner);
        typed_call("t8b::arm", "[]").expect("arm");
        typed_call("t8a::go", "[]").expect("go");
    });
    assert_eq!(code, 0);
    assert!(!crate::zlib::active());
}

#[test]
fn zlib_owner_and_net_in_separate_packages_end() {
    let _zlib_guard = zlib_global_guard();
    let net = format!(
        "{NET_OPEN_BODY}\n{}",
        r#"exports.arm = function () { netOpen(function (l, conn, acc) {
  setTimeout(function () {
    __thaw_net_destroy(Number(conn[1]));
    __thaw_net_destroy(Number(acc[1]));
    __thaw_net_close_listener(Number(l[1]));
  }, 300);
}); };"#
    );
    let zlib = format!(
        "{ZLIB_SHIM_HOOK}\n{}",
        r#"exports.go = function () {
  zjob('compress', new Uint8Array(1 << 20).fill(98), function (packed) {
    zjob('decompress', packed, function (plain) { process.exitCode = plain ? 0 : 5; });
  });
};"#
    );
    let code = run_loop_or_fail(move || {
        register_exit_package("t9n", &net);
        register_exit_package("t9z", &zlib);
        typed_call("t9n::arm", "[]").expect("arm");
        typed_call("t9z::go", "[]").expect("go");
    });
    assert_eq!(code, 0);
    assert!(!crate::zlib::active());
}

#[test]
fn zlib_hook_without_registered_job_does_not_leak_done() {
    let _zlib_guard = zlib_global_guard();
    // A result whose handle is in no callback table (a second shim instance) must still be taken, or DONE keeps
    // `zlib::active()` true and the loop never ends.
    let body = format!(
        "{ZLIB_SHIM_HOOK}\n{}",
        "exports.go = function () { globalThis.h = __thaw_zlib_async_start('compress', 'gzip', new Uint8Array(1 << 16), ''); };"
    );
    let code = run_loop_or_fail(move || {
        register_exit_package("t8m", &body);
        typed_call("t8m::go", "[]").expect("go");
    });
    assert_eq!(code, 0);
    assert!(!crate::zlib::active());
}

#[test]
fn zlib_active_for_ignores_foreign_realm_jobs() {
    let _zlib_guard = zlib_global_guard();
    let mut owner = Interp::new();
    let other = Interp::new();
    owner
        .run("globalThis.h = __thaw_zlib_async_start('compress', 'gzip', new Uint8Array([1, 2, 3]), '');")
        .unwrap_or_else(|_| panic!("start"));
    assert!(crate::zlib::active_for(owner.realm_id));
    assert!(!crate::zlib::active_for(other.realm_id));
    assert!(crate::zlib::active());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        owner
            .run("console.log(__thaw_zlib_async_poll());")
            .unwrap_or_else(|_| panic!("poll"));
        if owner.out.last().is_some_and(|l| l.contains("\"ok\":true")) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "job never finished");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    // finished but not yet taken: still the owner's pending work
    assert!(crate::zlib::active_for(owner.realm_id));
    owner
        .run("__thaw_zlib_async_take(h);")
        .unwrap_or_else(|_| panic!("take"));
    assert!(!crate::zlib::active_for(owner.realm_id));
}

// TJ84: a Timeout/Immediate created in one package realm must be clearable from another.

/// A one-module bundle named `prefix` (the factory key is `{prefix}/index.js`), as `MINI` does.
fn tj84_bundle(prefix: &str, body: &str) -> std::ffi::CString {
    std::ffi::CString::new(format!(
        "module.exports = (function(require){{\nvar __thaw_bundle_factories = {{\n\"{prefix}/index.js\": function(module, exports, require, requireAsync, __filename, __dirname, __thaw_require) {{\n{body}\n}},\n}};\nvar __thaw_bundle_require_maps = {{ \"{prefix}/index.js\": {{}}, }};\nvar __thaw_bundle_entry_key = \"{prefix}/index.js\";\n}})(globalThis.require);"
    ))
    .unwrap()
}

/// Run `f` on a fresh thread (its own package table); a hang or a panic in `f` fails instead of blocking.
fn tj84_guarded(f: impl FnOnce() + Send + 'static) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        f();
        let _ = tx.send(());
    });
    assert!(
        matches!(rx.recv_timeout(std::time::Duration::from_secs(10)), Ok(())),
        "hang (the timer was not cleared) or panic in the guarded body"
    );
}

fn tj84_register(name: &str, body: &str) {
    use crate::host::thaw_tjs_register;
    let bundle = tj84_bundle(name, body);
    let name = std::ffi::CString::new(name).unwrap();
    assert_eq!(thaw_tjs_register(name.as_ptr(), bundle.as_ptr()), 1);
}

fn tj84_export(path: &str) -> u64 {
    use crate::host::thaw_tjs_get_global;
    let f = thaw_tjs_get_global(std::ffi::CString::new(path).unwrap().as_ptr());
    assert!(f > 0, "{path}");
    f as u64
}

/// Call the handle `f` with `args` (a wire JSON string) and return the result handle.
fn tj84_call(f: u64, args: &str) -> u64 {
    use crate::host::thaw_tjs_call_handle_handle_graph_args_result;
    let args = std::ffi::CString::new(args).unwrap();
    let r = thaw_tjs_call_handle_handle_graph_args_result(f, args.as_ptr(), false);
    assert!(r.error.is_null(), "call failed");
    r.value
}

/// Call `f` with the one argument `handle` (kept alive for the call).
fn tj84_call_with_handle(f: u64, handle: u64) -> u64 {
    use crate::host::thaw_tjs_retain_handle;
    thaw_tjs_retain_handle(handle as i64);
    tj84_call(
        f,
        &format!(r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}}]}},{{"hdl":{handle}}}]}}"#),
    )
}

const TJ84_EMPTY_ARGS: &str = r#"{"root":{"r":0},"nodes":[{"a":[]}]}"#;

const TJ84_A: &str = "exports.start = function () { globalThis.__n = 0; return setInterval(function () { globalThis.__n++; }, 1); };\n\
exports.startTimeout = function () { return setTimeout(function () { globalThis.__fired = true; }, 50); };\n\
exports.startImmediate = function () { return setImmediate(function () { globalThis.__fired = true; }); };\n\
exports.fired = function () { return globalThis.__fired === true; };";

const TJ84_B: &str = "exports.stop = function (t) { clearInterval(t); };\n\
exports.stopTimeout = function (t) { clearTimeout(t); };\n\
exports.stopImmediate = function (t) { clearImmediate(t); };\n\
exports.noise = function () { setTimeout(function () { globalThis.__noise = true; }, 1); };\n\
exports.noisy = function () { return globalThis.__noise === true; };";

#[test]
fn tj84_clear_interval_from_another_package_ends_the_loop() {
    use crate::host::thaw_tjs_run_event_loop;
    tj84_guarded(|| {
        tj84_register("a", TJ84_A);
        tj84_register("b", TJ84_B);
        let handle = tj84_call(tj84_export("a::start"), TJ84_EMPTY_ARGS);
        tj84_call_with_handle(tj84_export("b::stop"), handle);
        assert_eq!(thaw_tjs_run_event_loop(), 0);
    });
}

#[test]
fn tj84_id_collision_does_not_clear_a_foreign_timer() {
    use crate::host::{thaw_tjs_handle_to_string, thaw_tjs_run_event_loop};
    tj84_guarded(|| {
        tj84_register("a", TJ84_A);
        tj84_register("b", TJ84_B);
        // a live timer in b, then the interval in a: both would have taken the same per-realm id
        tj84_call(tj84_export("b::noise"), TJ84_EMPTY_ARGS);
        let handle = tj84_call(tj84_export("a::start"), TJ84_EMPTY_ARGS);
        tj84_call_with_handle(tj84_export("b::stop"), handle);
        assert_eq!(thaw_tjs_run_event_loop(), 0);
        let noisy = tj84_call(tj84_export("b::noisy"), TJ84_EMPTY_ARGS);
        let text = unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(noisy)) };
        assert_eq!(text.to_str().unwrap(), "true", "b's timer was removed");
    });
}

#[test]
fn tj84_clear_timeout_and_immediate_across_packages() {
    use crate::host::{thaw_tjs_handle_to_string, thaw_tjs_run_event_loop};
    tj84_guarded(|| {
        tj84_register("a", TJ84_A);
        tj84_register("b", TJ84_B);
        let timeout = tj84_call(tj84_export("a::startTimeout"), TJ84_EMPTY_ARGS);
        let immediate = tj84_call(tj84_export("a::startImmediate"), TJ84_EMPTY_ARGS);
        tj84_call_with_handle(tj84_export("b::stopTimeout"), timeout);
        tj84_call_with_handle(tj84_export("b::stopImmediate"), immediate);
        assert_eq!(thaw_tjs_run_event_loop(), 0);
        let fired = tj84_call(tj84_export("a::fired"), TJ84_EMPTY_ARGS);
        let text = unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(fired)) };
        assert_eq!(
            text.to_str().unwrap(),
            "false",
            "a cleared callback still ran"
        );
    });
}

// TJ85: ref/unref/hasRef/refresh on a Timeout/Immediate created in another package realm.

const TJ85_B: &str = "exports.unref = function (t) { t.unref(); };\n\
exports.ref = function (t) { t.ref(); };\n\
exports.hasRef = function (t) { return t.hasRef(); };\n\
exports.refresh = function (t) { return t.refresh() === t; };";

/// The string form of a result handle.
fn tj85_text(handle: u64) -> String {
    use crate::host::thaw_tjs_handle_to_string;
    let text = unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(handle)) };
    text.to_str().unwrap().to_string()
}

#[test]
fn tj85_foreign_unref_interval_ends_the_loop() {
    use crate::host::thaw_tjs_run_event_loop;
    tj84_guarded(|| {
        tj84_register("a", TJ84_A);
        tj84_register("b", TJ85_B);
        let handle = tj84_call(tj84_export("a::start"), TJ84_EMPTY_ARGS);
        tj84_call_with_handle(tj84_export("b::unref"), handle);
        assert_eq!(thaw_tjs_run_event_loop(), 0);
    });
}

#[test]
fn tj85_has_ref_tracks_a_foreign_unref_and_ref() {
    use crate::host::thaw_tjs_run_event_loop;
    tj84_guarded(|| {
        tj84_register("a", TJ84_A);
        tj84_register("b", TJ85_B);
        let handle = tj84_call(tj84_export("a::start"), TJ84_EMPTY_ARGS);
        let has_ref = tj84_export("b::hasRef");
        assert_eq!(tj85_text(tj84_call_with_handle(has_ref, handle)), "true");
        tj84_call_with_handle(tj84_export("b::unref"), handle);
        assert_eq!(tj85_text(tj84_call_with_handle(has_ref, handle)), "false");
        tj84_call_with_handle(tj84_export("b::ref"), handle);
        assert_eq!(tj85_text(tj84_call_with_handle(has_ref, handle)), "true");
        tj84_call_with_handle(tj84_export("b::unref"), handle);
        assert_eq!(thaw_tjs_run_event_loop(), 0);
    });
}

#[test]
fn tj85_foreign_ref_after_unref_keeps_the_timer_alive() {
    use crate::host::{thaw_tjs_handle_to_string, thaw_tjs_run_event_loop};
    tj84_guarded(|| {
        tj84_register("a", TJ84_A);
        tj84_register("b", TJ85_B);
        let handle = tj84_call(tj84_export("a::startTimeout"), TJ84_EMPTY_ARGS);
        tj84_call_with_handle(tj84_export("b::unref"), handle);
        tj84_call_with_handle(tj84_export("b::ref"), handle);
        assert_eq!(thaw_tjs_run_event_loop(), 0);
        let fired = tj84_call(tj84_export("a::fired"), TJ84_EMPTY_ARGS);
        let text = unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(fired)) };
        assert_eq!(
            text.to_str().unwrap(),
            "true",
            "re-ref'd timeout did not run"
        );
    });
}

#[test]
fn tj85_foreign_unref_timeout_does_not_block_the_loop() {
    use crate::host::{thaw_tjs_handle_to_string, thaw_tjs_run_event_loop};
    tj84_guarded(|| {
        tj84_register("a", TJ84_A);
        tj84_register("b", TJ85_B);
        let handle = tj84_call(tj84_export("a::startTimeout"), TJ84_EMPTY_ARGS);
        tj84_call_with_handle(tj84_export("b::unref"), handle);
        assert_eq!(thaw_tjs_run_event_loop(), 0);
        let fired = tj84_call(tj84_export("a::fired"), TJ84_EMPTY_ARGS);
        let text = unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(fired)) };
        assert_eq!(text.to_str().unwrap(), "false", "unref'd timeout ran");
    });
}

#[test]
fn tj85_immediate_variants() {
    use crate::host::{thaw_tjs_handle_to_string, thaw_tjs_run_event_loop};
    tj84_guarded(|| {
        tj84_register("a", TJ84_A);
        tj84_register("b", TJ85_B);
        let handle = tj84_call(tj84_export("a::startImmediate"), TJ84_EMPTY_ARGS);
        let has_ref = tj84_export("b::hasRef");
        assert_eq!(tj85_text(tj84_call_with_handle(has_ref, handle)), "true");
        tj84_call_with_handle(tj84_export("b::unref"), handle);
        assert_eq!(tj85_text(tj84_call_with_handle(has_ref, handle)), "false");
        tj84_call_with_handle(tj84_export("b::ref"), handle);
        assert_eq!(tj85_text(tj84_call_with_handle(has_ref, handle)), "true");
        assert_eq!(thaw_tjs_run_event_loop(), 0);
        let fired = tj84_call(tj84_export("a::fired"), TJ84_EMPTY_ARGS);
        let text = unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(fired)) };
        assert_eq!(
            text.to_str().unwrap(),
            "true",
            "re-ref'd immediate did not run"
        );
    });
}

#[test]
fn tj85_foreign_refresh_returns_the_timeout_and_keeps_it_alive() {
    use crate::host::{thaw_tjs_handle_to_string, thaw_tjs_run_event_loop};
    tj84_guarded(|| {
        tj84_register("a", TJ84_A);
        tj84_register("b", TJ85_B);
        let handle = tj84_call(tj84_export("a::startTimeout"), TJ84_EMPTY_ARGS);
        let refreshed = tj84_call_with_handle(tj84_export("b::refresh"), handle);
        assert_eq!(
            tj85_text(refreshed),
            "true",
            "refresh did not return the timeout"
        );
        assert_eq!(thaw_tjs_run_event_loop(), 0);
        let fired = tj84_call(tj84_export("a::fired"), TJ84_EMPTY_ARGS);
        let text = unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(fired)) };
        assert_eq!(
            text.to_str().unwrap(),
            "true",
            "refreshed timeout did not run"
        );
    });
}

#[test]
fn tj85_foreign_unref_does_not_touch_the_local_timer_in_the_caller() {
    use crate::host::{thaw_tjs_handle_to_string, thaw_tjs_run_event_loop};
    tj84_guarded(|| {
        let b_body = format!("{TJ84_B}\n{TJ85_B}");
        tj84_register("a", TJ84_A);
        tj84_register("b", &b_body);
        // a live ref'd timer owned by b, then a's interval: b's unref targets a's id
        tj84_call(tj84_export("b::noise"), TJ84_EMPTY_ARGS);
        let handle = tj84_call(tj84_export("a::start"), TJ84_EMPTY_ARGS);
        tj84_call_with_handle(tj84_export("b::unref"), handle);
        assert_eq!(thaw_tjs_run_event_loop(), 0);
        let noisy = tj84_call(tj84_export("b::noisy"), TJ84_EMPTY_ARGS);
        let text = unsafe { std::ffi::CStr::from_ptr(thaw_tjs_handle_to_string(noisy)) };
        assert_eq!(text.to_str().unwrap(), "true", "b's timer was affected");
    });
}

#[test]
fn slowbuffer_is_a_global_function_returning_a_sized_buffer() {
    assert_eq!(
        one("console.log(typeof SlowBuffer, SlowBuffer(4).length, SlowBuffer(4) instanceof Buffer, Buffer.isBuffer(SlowBuffer(2)))"),
        "function 4 true true"
    );
}

#[test]
fn slowbuffer_prototype_is_buffer_prototype_and_validates_size() {
    assert_eq!(
        one("console.log(SlowBuffer.prototype === Buffer.prototype, typeof SlowBuffer.prototype.toString);
             try { SlowBuffer('4'); } catch (e) { console.log(e.name, e.code); }"),
        "true function|TypeError ERR_INVALID_ARG_TYPE"
    );
}

#[test]
fn slowbuffer_is_exposed_through_the_buffer_module_shape() {
    // Same object shape as the registry `buffer` builtin (basic.rs): {Buffer, SlowBuffer, ...} from globals.
    assert_eq!(
        one("var require = function (n) { if (n === 'buffer') return { Buffer: globalThis.Buffer, SlowBuffer: globalThis.SlowBuffer }; throw new Error('Cannot find module ' + n); };
             console.log(require('buffer').SlowBuffer === SlowBuffer, require('buffer').Buffer === Buffer);"),
        "true true"
    );
}

#[test]
fn buffer_equal_constant_time_load_shape_does_not_throw() {
    // buffer-equal-constant-time reads `require('buffer').SlowBuffer.prototype.equal` at module load, then
    // installs a comparator; the comparator here mirrors its byte-wise equality check.
    assert_eq!(
        one("var require = function (n) { if (n === 'buffer') return { Buffer: globalThis.Buffer, SlowBuffer: globalThis.SlowBuffer }; throw new Error('Cannot find module ' + n); };
             var B = require('buffer').SlowBuffer;
             var origSlowBufEqual = B.prototype.equal;
             function bufferEq(a, b) {
               if (!Buffer.isBuffer(a) || !Buffer.isBuffer(b) || a.length !== b.length) return false;
               var c = 0;
               for (var i = 0; i < a.length; i++) c |= a[i] ^ b[i];
               return c === 0;
             }
             console.log(typeof origSlowBufEqual, bufferEq(Buffer.from('secret'), Buffer.from('secret')), bufferEq(Buffer.from('secret'), Buffer.from('secrex')));"),
        "undefined true false"
    );
}

/// Bundle whose `shape`/`view`/`buf` report how a tagged `Bytes` argument was revived.
const BYTES_REVIVE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"r/index.js": function(module, exports, require) {
  exports.shape = function (v) { return Object.prototype.toString.call(v) + "," + (v instanceof ArrayBuffer) + "," + (v instanceof Uint8Array) + "," + v.byteLength; };
  exports.view = function (v) { return Object.prototype.toString.call(v) + "," + (v instanceof DataView) + "," + v.byteLength + "," + v.getUint8(0); };
  exports.buf = function (v) { return Buffer.isBuffer(v) + "," + v.length; };
},
};
var __thaw_bundle_require_maps = { "r/index.js": {}, };
var __thaw_bundle_entry_key = "r/index.js";
})(globalThis.require);"#;

/// `{"__thaw_tjs_bytes__": {"k": kind, "b64": ...}}` for `data`; `kind: None` omits `k`.
fn tj93_bytes_arg_kind(kind: Option<&str>, data: &[u8]) -> String {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
    match kind {
        Some(k) => format!(r#"{{"__thaw_tjs_bytes__":{{"k":"{k}","b64":"{b64}"}}}}"#),
        None => format!(r#"{{"__thaw_tjs_bytes__":{{"b64":"{b64}"}}}}"#),
    }
}

#[test]
fn tj93_array_buffer_kind_node_revives_as_array_buffer() {
    use std::ffi::CString;
    let bundle = CString::new(BYTES_REVIVE).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"tj93ab".as_ptr(), bundle.as_ptr()),
        1
    );
    assert_eq!(
        typed_call(
            "tj93ab::shape",
            &format!("[{}]", tj93_bytes_arg_kind(Some("ArrayBuffer"), &[1, 2, 3]))
        )
        .unwrap(),
        "\"[object ArrayBuffer],true,false,3\""
    );
}

#[test]
fn tj93_data_view_kind_node_revives_as_data_view() {
    use std::ffi::CString;
    let bundle = CString::new(BYTES_REVIVE).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"tj93dv".as_ptr(), bundle.as_ptr()),
        1
    );
    assert_eq!(
        typed_call(
            "tj93dv::view",
            &format!("[{}]", tj93_bytes_arg_kind(Some("DataView"), &[1, 2, 3]))
        )
        .unwrap(),
        "\"[object DataView],true,3,1\""
    );
}

#[test]
fn tj93_unknown_or_missing_kind_still_revives_as_buffer() {
    use std::ffi::CString;
    let bundle = CString::new(BYTES_REVIVE).unwrap();
    assert_eq!(
        crate::host::thaw_tjs_register(c"tj93bf".as_ptr(), bundle.as_ptr()),
        1
    );
    assert_eq!(
        typed_call(
            "tj93bf::buf",
            &format!("[{}]", tj93_bytes_arg_kind(None, &[1, 2, 3]))
        )
        .unwrap(),
        "\"true,3\""
    );
    assert_eq!(
        typed_call(
            "tj93bf::buf",
            &format!("[{}]", tj93_bytes_arg_kind(Some("Weird"), &[1, 2, 3]))
        )
        .unwrap(),
        "\"true,3\""
    );
}

// P4 guards: `this` resolution must match the JS spec before and after the per-scope `this` field.

#[test]
fn this_method_call_and_sloppy_nested_default_to_global() {
    assert_eq!(
        one("var o = { v: 7, m() { return this.v; }, n() { return function () { return this === globalThis; }(); } }; console.log(o.m(), o.n());"),
        "7 true"
    );
}

#[test]
fn this_getter_callback_thisarg_and_super_method() {
    assert_eq!(
        one("var o = { _v: 2, get v() { return this._v * 2; } }; console.log(o.v, [1].map(function () { return this; }, o)[0] === o);"),
        "4 true"
    );
    assert_eq!(
        one("class P { who() { return this.n; } } class Q extends P { n = 'q'; who() { return super.who() + '!'; } } console.log(new Q().who());"),
        "q!"
    );
}

#[test]
fn arrow_this_is_lexical_through_nested_arrows() {
    assert_eq!(
        one("var o = { v: 1, m() { var a = () => () => this.v; return a()(); } }; var p = { v: 2 }; console.log(o.m.call(p), (() => this)() === this);"),
        "2 true"
    );
    assert_eq!(
        one("var o = { n: 3, m() { var r = []; for (let i = 0; i < this.n; i++) r.push(() => this.n + i); return r.map(f => f()).join(); } }; console.log(o.m());"),
        "3,4,5"
    );
}

#[test]
fn class_field_initializers_and_static_this() {
    assert_eq!(
        one("class C { x = 5; y = this.x + 1; f = () => this.y; static s = this.name; } var c = new C(); console.log(c.x, c.y, c.f(), C.s);"),
        "5 6 6 C"
    );
}

#[test]
fn class_strict_method_extraction_this_is_undefined() {
    assert_eq!(
        one("class D { m() { return this; } } var m = new D().m; console.log(m() === undefined);"),
        "true"
    );
}

#[test]
fn class_derived_constructor_this_after_super() {
    assert_eq!(
        one("class A { constructor() { this.a = 1; } } class B extends A { constructor() { super(); this.b = this.a + 1; } } var b = new B(); console.log(b.a, b.b, b instanceof B);"),
        "1 2 true"
    );
}

#[test]
fn this_with_statement_does_not_resolve_this_as_a_property() {
    // `this` is not an identifier reference: a `with` object's `this` property must not shadow it.
    // `with` needs sloppy code, so the body runs inside `new Function`.
    assert_eq!(
        one(
            r#"console.log(new Function("var o = { m: 1 }; var t = { id: 3, get() { with (o) { return this.id; } } }; return t.get();")());"#
        ),
        "3"
    );
    assert_eq!(
        one(
            r#"console.log(new Function("var w = { this: 99 }; var u = { r() { with (w) { return this === u; } } }; return u.r();")());"#
        ),
        "true"
    );
}

#[test]
fn this_bound_call_apply() {
    assert_eq!(
        one("function f(a) { return [this.k, a].join(); } var o = { k: 'K' }; console.log(f.call(o, 1), f.apply(o, [2]), f.bind(o, 3)());"),
        "K,1 K,2 K,3"
    );
    assert_eq!(
        one("function f(a) { return [this.k, a].join(); } var g = f.bind({ k: 'K' }); console.log(g.call({ k: 'X' }, 4));"),
        "K,4"
    );
}

#[test]
fn this_global_sloppy_vs_strict_and_null_receiver() {
    assert_eq!(
        one("function sl() { return this; } function st() { 'use strict'; return this; } console.log(sl() === globalThis, st() === undefined, sl.call(null) === globalThis);"),
        "true true true"
    );
}

#[test]
fn this_eval_and_new_function_global() {
    assert_eq!(
        one("console.log(new Function('return this')() === globalThis);"),
        "true"
    );
}

#[test]
fn generator_this_in_body_and_yield() {
    assert_eq!(
        one("var o = { v: 4, *g() { yield this.v; yield this === o; } }; console.log([...o.g()].join(','));"),
        "4,true"
    );
}

#[test]
fn async_method_this_survives_await() {
    let (out, _) = main_run(
        "var o = { v: 5, async m() { await null; return this.v; } };
         o.m().then(v => console.log('async', v));",
    );
    assert_eq!(out.join("|"), "async 5");
}

/// Like `run`, but parsed as a sloppy-mode script (for `with`).
fn run_sloppy(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    match it.run_sloppy(src) {
        Ok(_) => it.out,
        Err(Ctl::Throw(v)) => {
            let mut out = it.out.clone();
            out.push(format!("Uncaught {}", it.to_str(&v)));
            out
        }
        Err(_) => vec!["stray completion".into()],
    }
}

// Guard tests for read-modify-write on identifiers (`i++`, `x += n`, `x = x op y`). They pin the
// observable semantics the fast path must keep; expectations were checked against Node.

#[test]
fn rmw_scope_with_global_and_undeclared_targets() {
    assert_eq!(
        run("var i=0; i++; ++i; i--; console.log(i, i++ + ++i);"),
        vec!["1 4"]
    );
    assert_eq!(
        run_sloppy("var o={x:1}; with(o){ x++; x+=5; } console.log(o.x);"),
        vec!["7"]
    );
    assert_eq!(
        run("globalThis.g1=1; g1++; g1 += 1; console.log(g1, globalThis.g1);"),
        vec!["3 3"]
    );
    assert_eq!(run("u = 5; u++; console.log(u);"), vec!["6"]);
    assert_eq!(
        run("try { zz += 1 } catch(e) { console.log(e.name, e.message) }"),
        vec!["ReferenceError zz is not defined"]
    );
}

#[test]
fn rmw_closure_sees_updates_and_read_happens_before_rhs() {
    assert_eq!(
        run("function f(){ var n=1; var g=()=>n; n++; n+=2; return g(); } console.log(f());"),
        vec!["4"]
    );
    assert_eq!(
        run("var k=0; var inc=()=>k++; inc(); inc(); console.log(k);"),
        vec!["2"]
    );
    // the left operand is read before the right-hand side runs, so the write back wins
    assert_eq!(
        run("var x=1; function s(){ x=10; return 1;} x += s(); console.log(x);"),
        vec!["2"]
    );
    assert_eq!(
        run("var x2=1; function s2(){ x2=10; return 1;} x2 = x2 + s2(); console.log(x2);"),
        vec!["2"]
    );
    assert_eq!(run("var y=1; y += (y = 5); console.log(y);"), vec!["6"]);
}

#[test]
fn rmw_const_assignments_throw_and_keep_value() {
    assert_eq!(
        run(
            "const c=1; try { c++ } catch(e){ console.log(e.message) } try { c += 1 } catch(e){ console.log(e.message) } try { c = c + 1 } catch(e){ console.log(e.message) } console.log(c);"
        ),
        vec![
            "Assignment to constant variable.",
            "Assignment to constant variable.",
            "Assignment to constant variable.",
            "1"
        ]
    );
}

#[test]
fn rmw_bigint_and_mixing_errors() {
    assert_eq!(
        run("let b=5n; b++; b += 2n; b = b * 2n; console.log(String(b), typeof b);"),
        vec!["16 bigint"]
    );
    assert_eq!(
        run("let m=1; try { m += 2n } catch(e) { console.log(e.name, e.message) }"),
        vec!["TypeError Cannot mix BigInt and other types, use explicit conversions"]
    );
}

#[test]
fn rmw_operator_coercions_and_postfix_results() {
    assert_eq!(
        run("let s3='a'; s3 += 'b'; s3 += 1; console.log(s3); let n3=1; n3 = n3 + '2'; console.log(n3);"),
        vec!["ab1", "12"]
    );
    assert_eq!(
        run("var oo={valueOf(){ return 7 }}; var q=1; q += oo; console.log(q);"),
        vec!["8"]
    );
    assert_eq!(
        run("let o2={valueOf(){return 7}}; let p=o2++; console.log(p, o2); let pp=++o2; console.log(pp);"),
        vec!["7 8", "9"]
    );
    assert_eq!(
        run("let st='5'; st++; console.log(st, typeof st); let st2='5'; let r2=st2--; console.log(r2, st2);"),
        vec!["6 number", "5 4"]
    );
    assert_eq!(
        run("let un; un++; console.log(un); let nul=null; let rn=nul++; console.log(rn, nul);"),
        vec!["NaN", "0 1"]
    );
    assert_eq!(run("let t=true; t+=1; console.log(t);"), vec!["2"]);
    assert_eq!(
        run("let d=7; d /= 2; d %= 2; d *= 3; d -= 0.5; console.log(d);"),
        vec!["4"]
    );
    assert_eq!(
        run("let z=1; z/=0; console.log(z); z=-1; z%=0; console.log(z); let nz=0; nz *= -1; console.log(Object.is(nz,-0));"),
        vec!["Infinity", "NaN", "true"]
    );
    assert_eq!(
        run("let a1=1; let b1='3'; a1 = a1 * b1; console.log(a1); let nan=NaN; nan = nan < 1; console.log(nan);"),
        vec!["3", "false"]
    );
}

#[test]
fn construct_new_target_is_the_constructor_for_functions_and_classes() {
    assert_eq!(
        one("function F(){ this.nt = new.target; } class C { constructor(){ this.nt = new.target; } } class D extends C {} console.log(new F().nt === F, new C().nt === C, new D().nt === D, F.call({}) === undefined)"),
        "true true true true"
    );
}

#[test]
fn reflect_construct_new_target_picks_the_instance_prototype() {
    assert_eq!(
        one("function F(){ this.nt = new.target; } function Alt(){} Alt.prototype = { tag: 'alt' }; var r = Reflect.construct(F, [], Alt); console.log(Object.getPrototypeOf(r) === Alt.prototype, r.nt === Alt, r.tag)"),
        "true true alt"
    );
    assert_eq!(
        one("function Alt(){} Alt.prototype = Object.create(Map.prototype); var m = Reflect.construct(Map, [], Alt); console.log(m instanceof Alt, m.set(1, 2) === m, m.get(1))"),
        "true true 2"
    );
}

#[test]
fn proxy_construct_trap_receives_new_target() {
    assert_eq!(
        one("var F = function(){ this.k = 1; }; var seen; var P = new Proxy(F, { construct(t, a, nt) { seen = nt; return Reflect.construct(t, a, nt); } }); var o = new P(); console.log(seen === P, o.k, o instanceof F)"),
        "true 1 true"
    );
    assert_eq!(
        one("class F2 { constructor(){ this.nt = new.target; } } var P2 = new Proxy(F2, {}); console.log(new P2().nt === P2)"),
        "true"
    );
}

#[test]
fn derived_class_over_proxy_construct_trap_result_uses_its_get_prototype_of_trap() {
    assert_eq!(
        one("var P = new Proxy(function(){}, { construct(t, a, nt) { return new Proxy({}, { getPrototypeOf() { return nt.prototype; } }); } }); class D extends P {} console.log(new D() instanceof D)"),
        "true"
    );
}

#[test]
fn derived_class_over_proxy_construct_trap_gets_derived_new_target() {
    assert_eq!(
        one("var F = function(){ this.k = 1; }; var seen; var P = new Proxy(F, { construct(t, a, nt) { seen = nt; return Reflect.construct(t, a, nt); } }); class D extends P {} var d = new D(); console.log(seen === D, d.k, Object.getPrototypeOf(d) === D.prototype, d instanceof D)"),
        "true 1 true true"
    );
    assert_eq!(
        one("var F = function(){ this.k = 1; }; var seen; var P = new Proxy(F, { construct(t, a, nt) { seen = nt; return Reflect.construct(t, a, nt); } }); class D extends P { constructor() { super(); this.z = 2; } } var d = new D(); console.log(seen === D, d.k, d.z)"),
        "true 1 2"
    );
}

#[test]
fn bound_function_constructs_its_target_with_bound_arguments() {
    assert_eq!(
        one("function F(a, b){ this.s = a + b; this.isF = new.target === F; } var B = F.bind(null, 2); var o = new B(3); console.log(o.s, o.isF, o instanceof F)"),
        "5 true true"
    );
}

#[test]
fn new_target_is_undefined_for_plain_calls_and_subclassed_builtins_keep_prototype() {
    assert_eq!(
        one("function F(){ return new.target === undefined } console.log(F(), new F() instanceof F)"),
        "true true"
    );
    assert_eq!(
        one("class A extends Array {} class M extends Map {} var a = new A(); a.push(1); console.log(a instanceof A, a.length, new M() instanceof M, new M().size)"),
        "true 1 true 0"
    );
}

#[test]
fn derived_class_over_proxy_trap_result_object_is_adopted() {
    assert_eq!(
        one("var F = function(){ this.k = 1; }; var P = new Proxy(F, { construct(t, a, nt) { return { q: 9 }; } }); class D extends P {} var d = new D(); console.log(d.q, d.k, d instanceof D)"),
        "9 undefined false"
    );
}

#[test]
fn call_guard_closure_capture_and_param_shadowing() {
    assert_eq!(
        one("function mk(a){ return function(){ a++; return a; }; } var c = mk(5); c(); console.log(c(), mk(1)())"),
        "7 2"
    );
    assert_eq!(
        one("function s(a){ var a; return a } function t(a){ function a(){} return typeof a } function d(a, a){ return a } console.log(s(5), t(5), d(1, 2))"),
        "5 function 2"
    );
}

#[test]
fn call_guard_arguments_length_and_values() {
    assert_eq!(
        one("function f(){ return arguments.length + ':' + [].slice.call(arguments).join('-') } console.log(f(), f(1, 2, 3))"),
        "0: 3:1-2-3"
    );
    assert_eq!(
        one("function g(a){ return arguments.length } console.log(g(), g(1, 2, 3, 4, 5, 6))"),
        "0 6"
    );
}

#[test]
fn call_guard_default_and_rest_params_with_arguments() {
    assert_eq!(
        one("function f(a, b = a + 1, ...r){ return [a, b, r.length, arguments.length].join() } console.log(f(1), f(1, 5, 6, 7))"),
        "1,2,0,1 1,5,2,4"
    );
}

#[test]
fn call_guard_recursion_depth_range_error_then_recovers() {
    let src = "function r(n){ return r(n + 1); } try { r(0) } catch (e) { console.log(e instanceof RangeError, e.message) }
      const g = (n) => g(n + 1); try { g(0) } catch (e) { console.log('arrow', e instanceof RangeError) }
      class K { m(n){ return this.m(n + 1) } } try { new K().m(0) } catch (e) { console.log('method', e.message) }
      function fib(n){ return n < 2 ? n : fib(n - 1) + fib(n - 2) } console.log(fib(10));";
    let out = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || one(src))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(
        out,
        "true Maximum call stack size exceeded|arrow true|method Maximum call stack size exceeded|55"
    );
}

#[test]
fn call_guard_construct_bound_and_this() {
    assert_eq!(
        one("function P(a, b){ this.s = a + b } var Q = P.bind(null, 1); console.log(new P(3, 4).s, Q.call(null, 5) === undefined)"),
        "7 true"
    );
    assert_eq!(
        one("var o = { v: 3, get d(){ return this.v * 2 }, m(k){ return this.v + k } }; console.log(o.d, o.m(1), o.m.call({ v: 10 }, 1))"),
        "6 4 11"
    );
}

#[test]
fn call_guard_class_constructor_and_methods() {
    assert_eq!(
        one("class A { constructor(x){ this.x = x } get y(){ return this.x + 1 } static mk(...a){ return new A(a.length) } } console.log(A.mk(1, 2, 3).x, new A(4).y)"),
        "3 5"
    );
}

#[test]
fn call_guard_generator_and_async_params() {
    assert_eq!(
        one("function* g(a, ...r){ yield a; yield r.length; } console.log([...g(7, 8, 9)].join())"),
        "7,2"
    );
    let (out, _) = main_run(
        "async function h(a = 2){ return a * 3 }
         (async () => {
           console.log(await h(), await h(5), await h(undefined, 1));
         })();",
    );
    assert_eq!(out.join("|"), "6 15 6");
}

#[test]
fn call_guard_deadline_interrupts_recursive_calls() {
    let src = include_str!("../../thaw-registry/src/registry/builtins/system/vm_context.js").to_string() + "
      function f(n){ return n < 0 ? 0 : f(n - 1) }
      try { runInThisContext('while (true) { f(3) }', { timeout: 30 }); } catch (e) { console.log(e.code, e.message); }
      console.log(runInThisContext('f(5)'));";
    assert_eq!(
        one(&src),
        "ERR_SCRIPT_EXECUTION_TIMEOUT Script execution timed out after 30ms|0"
    );
}

// `s += x` appends in place only for a uniquely owned string; these pin the JS value semantics the
// in-place path must keep (aliasing, re-entrancy, const/TDZ/with, surrogates, flag, key uses).
#[test]
fn str_append_keeps_aliases_and_closures_unchanged() {
    assert_eq!(
        one(r"var a='x'.repeat(300); var b=a; a+='y'; console.log(a.length, b.length)"),
        "301 300"
    );
    assert_eq!(
        one(
            r"var a2='x'.repeat(300); var arr=[a2]; a2+='y'; console.log(arr[0].length, a2.length)"
        ),
        "300 301"
    );
    assert_eq!(
        one(
            r"function mk(){ var s='x'.repeat(300); var f=function(){return s.length}; s+='y'; return f; } console.log(mk()())"
        ),
        "301"
    );
    assert_eq!(
        one(r"var s='ab'.repeat(150); s+=s; console.log(s.length, s.slice(-3))"),
        "600 bab"
    );
    assert_eq!(
        one(r"var s='x'.repeat(300); var r = (s += 'a'); console.log(r.length, r === s)"),
        "301 true"
    );
}

#[test]
fn str_append_rhs_reentrancy_and_object_rhs() {
    assert_eq!(
        one(r"var s='x'.repeat(300); s += (s = 'q', 'z'); console.log(s.length, s.slice(-1))"),
        "301 z"
    );
    assert_eq!(
        one(
            r"var s='x'.repeat(300); function f(){ s += 'k'; return 'z'; } s += f(); console.log(s.length, s.slice(-2))"
        ),
        "301 xz"
    );
    assert_eq!(
        one(
            r"var s='x'.repeat(300); s += {toString(){ s='q'; return 'T'}}; console.log(s.length, s.slice(-1))"
        ),
        "301 T"
    );
}

#[test]
fn str_append_const_tdz_and_with_targets() {
    assert_eq!(
        one(
            r"const c='x'.repeat(300); try { c += 'y' } catch(e) { console.log(e.name, c.length) }"
        ),
        "TypeError 300"
    );
    assert_eq!(
        one(r"try { q += 'a'; let q = 'x'.repeat(300); } catch (e) { console.log(e.name) }"),
        "ReferenceError"
    );
    assert_eq!(
        one(r"var g = 'x'.repeat(300); g += 'y'; console.log(g.length, g.slice(-2))"),
        "301 xy"
    );
}

#[test]
fn str_append_with_target_in_sloppy_script() {
    let mut it = Interp::new();
    assert!(it
        .run_sloppy(r"var w = {s: 'x'.repeat(300)}; var s = 'o'; with (w) { s += 'y'; } console.log(w.s.length, s)")
        .is_ok());
    assert_eq!(it.out.join("|"), "301 o");
}

#[test]
fn str_append_keeps_surrogate_seams_paired() {
    assert_eq!(
        one(
            r"var x0 = 'x'.repeat(300) + '\uD83D'; x0 += '\uDE00'; console.log(x0.length, x0.codePointAt(300).toString(16), x0.charCodeAt(301))"
        ),
        "302 1f600 56832"
    );
    assert_eq!(
        one(
            r"var t='x'.repeat(300)+'\uD83D'; t+='a'; console.log(t.length, t.charCodeAt(300), t.charCodeAt(301))"
        ),
        "302 55357 97"
    );
    assert_eq!(
        one(r"var u='\uD83D'; u+='\uDE00'; console.log(u.length, u.codePointAt(0).toString(16))"),
        "2 1f600"
    );
}

#[test]
fn str_append_ascii_flag_follows_the_contents() {
    assert_eq!(
        one(
            r"var s5='a'.repeat(300); console.log(s5.indexOf('a'), s5.length); s5 += 'é'; s5 += 'z'; console.log(s5.length, s5.indexOf('é'), s5.indexOf('z'), s5.charAt(300), s5.slice(299, 302), s5.charCodeAt(300), s5.lastIndexOf('a'))"
        ),
        "0 300|302 300 301 é aéz 233 299"
    );
    assert_eq!(
        one(
            r"var t5='b'.repeat(300); t5+='c'; t5+='d'; console.log(t5.indexOf('d'), t5.charAt(301), t5.length, /d$/.test(t5))"
        ),
        "301 d 302 true"
    );
}

#[test]
fn str_append_result_works_as_map_json_regexp_and_key() {
    assert_eq!(
        one(
            r"var k=''; for (var i=0;i<100;i++) k+='ab'; k += 'c'; var m=new Map(); m.set(k,1); var lit='ab'.repeat(100)+'c'; console.log(m.get(lit), m.has(k), ({[lit]: 5})[k], JSON.parse(JSON.stringify({v:k})).v === k, /^(ab)+c$/.test(k), k.length)"
        ),
        "1 true 5 true true 201"
    );
    assert_eq!(
        one(
            r"var o={}; var kk='ab'.repeat(100); kk += 'c'; o[kk]=7; console.log(Object.keys(o)[0].length, o['ab'.repeat(100)+'c'], new Set([kk, 'ab'.repeat(100)+'c']).size, kk.split('c').length, kk === 'ab'.repeat(100)+'c')"
        ),
        "201 7 1 2 true"
    );
}

#[test]
fn str_build_prepend_and_binary_add_forms() {
    assert_eq!(
        one(r"var p=''; for(var i2=0;i2<300;i2++){ p = 'x' + p; } console.log(p.length)"),
        "300"
    );
    assert_eq!(
        one(r"var q2='x'.repeat(300); q2 = q2 + 'y'; console.log(q2.length, q2.slice(-1))"),
        "301 y"
    );
}

#[test]
fn str_append_in_a_loop_copies_only_when_promoting() {
    let mut it = Interp::new();
    let base = crate::jsstr::copies_made();
    assert!(it
        .run(r"var s=''; for (var i=0;i<100000;i++) s+='ab'; console.log(s.length)")
        .is_ok());
    assert_eq!(it.out.join("|"), "200000");
    // 128 appends build the sub-256-byte Plain string (one copy each); after promotion to a Buf the
    // 1e5-append loop copies nothing. A copy per append would make this ~1e5.
    let copies = crate::jsstr::copies_made() - base;
    assert!(copies <= 130, "copies: {copies}");
}

// ----- a non-iterable object in destructuring, for...of, spread is a catchable TypeError (not unsupported) -----

#[test]
fn non_iterable_object_in_destructuring_for_of_and_spread_throws_type_error() {
    assert_eq!(
        one("function t(g) { try { g(); return 'no'; } catch (e) { return e.name + ':' + (e instanceof TypeError) + ':' + e.message; } }
             function f() {}
             console.log(
               t(function () { var [a] = {}; }),
               t(function () { for (var x of {}) {} }),
               t(function () { f(...{}); }),
               t(function () { [...{}]; })
             );"),
        "TypeError:true:value is not iterable TypeError:true:value is not iterable TypeError:true:Spread syntax requires ...iterable[Symbol.iterator] to be a function TypeError:true:value is not iterable"
    );
}

#[test]
fn int32_bitwise_table_matches_js() {
    // expected values generated with node 22
    assert_eq!(
        one("console.log(-1.5|0, 4294967295.5|0, 2147483648|0, 4294967296|0, 4294967297|0, -2147483649|0, 1e20|0, 1e300|0, NaN|0, Infinity|0, -Infinity|0, -0|0, 3000000000>>>0, -1>>>0, 1<<31, 1<<32, 1<<-1, 5>>>32, -8>>1, 0xd1310ba6^0x243f6a88, ~5, ~~3.7, ~NaN)"),
        "-1 -1 -2147483648 0 1 2147483647 1661992960 0 0 0 0 0 3000000000 4294967295 -2147483648 1 -2147483648 5 -4 -183606994 -6 3 -1"
    );
    assert_eq!(
        one("console.log(-0.9|0, 2.5^0, -2.5>>>0, 4294967295>>>0, 9007199254740993|0, 1e19|0, -1e19|0, 1e-300|0, 2**53|0, (2**53+2)>>>0, 0.5<<1, 1.99>>>0)"),
        "0 2 4294967294 4294967295 0 -1981284352 1981284352 0 0 2 0 1"
    );
    assert_eq!(
        one("console.log(7 >> 33, -1 << 31, 1 >>> 0, 0xffffffff|0, -1 >>> 31, 255 ^ -1, 1.5 & 3.9, -7 >> 1, -7 >>> 1)"),
        "3 -2147483648 1 -1 1 -256 1 -4 2147483644"
    );
}

#[test]
fn bitwise_coercion_and_slow_path_still_apply() {
    assert_eq!(
        one("console.log(({valueOf(){return 6}})&3, '7'|0, true|2, null^5, (1n & 1n) === 1n, (6n << 2n) === 24n)"),
        "2 7 3 5 true true"
    );
    assert_eq!(
        one("try { 2n | 1 } catch (e) { console.log(e instanceof TypeError) }"),
        "true"
    );
}

#[test]
fn compound_number_ops_match_js() {
    assert_eq!(
        one("let n=1; n += 2; n ^= 0xff; n <<= 3; n >>>= 1; console.log(n)"),
        "1008"
    );
    assert_eq!(
        one("let x=1; x += (x = 10, 5); let y=1; y ^= (y=10, 5); console.log(x, y)"),
        "6 4"
    );
    assert_eq!(
        one("var a=[1,2,3]; var i=1; a[i] ^= 7; a[i] <<= 2; console.log(a[1])"),
        "20"
    );
    assert_eq!(
        one("var t=new Int32Array(2); t[0]=2147483647; t[0] += 1; t[1] |= 5; t[1] >>= 1; console.log(t[0], t[1])"),
        "-2147483648 2"
    );
    assert_eq!(
        one("let g=5; g %= 3; g -= 0.5; let h = 2**31; h |= 0; let k = 3; k >>>= -1; let e=2; e **= 3; console.log(g, h, k, e)"),
        "1.5 -2147483648 0 8"
    );
}

#[test]
fn compound_non_number_operands_take_generic_path() {
    assert_eq!(
        one("var s='a'; s += 1; let o={valueOf(){return 2}}; let o2=1; o2 += o; let b=1n; b += 2n; console.log(s, o2, b === 3n)"),
        "a1 3 true"
    );
    assert_eq!(
        one("let z = '5'; z ^= 1; let w = '6'; w *= 2; let q = true; q |= 2; console.log(z, w, q)"),
        "4 12 3"
    );
    assert_eq!(
        one("try { let u=3; u <<= 1n } catch (e) { console.log(e instanceof TypeError) }"),
        "true"
    );
}

#[test]
fn compound_bitwise_on_const_still_throws() {
    assert_eq!(
        one("const c=1; try { c |= 1 } catch (e) { console.log(e instanceof TypeError, e.message) }"),
        "true Assignment to constant variable."
    );
}

// ----- hinted reads (hints.rs). Unit tests run with the oracle on: every hinted read is checked
// against the by-name lookup. Expected values match node unless a comment says Thaw differs.

/// Like `one`, for sloppy-mode scripts (`with`, `delete x`, `var arguments` need them).
fn one_sloppy(src: &str) -> String {
    run_sloppy_lines(src).join("|")
}

fn run_sloppy_lines(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    let _ = it.run_sloppy(src);
    it.out
}

#[test]
fn ident_hint_records_and_hits_in_loops() {
    let mut it = Interp::new();
    let _ = it.run("let s = 0; function f(a) { return a + 1 } for (let i = 0; i < 50; i++) s += f(i); console.log(s)");
    assert_eq!(it.out, vec!["1275".to_string()]);
    assert!(
        it.ident_hints.iter().any(|c| c.get() != 0),
        "no hint was recorded"
    );
}

#[test]
fn ident_hint_shadow_declared_after_record() {
    // Thaw has no TDZ: the first call runs before `let x` and sees the global. node throws here.
    assert_eq!(
        one("let x = 'g'; const out = []; for (let i = 0; i < 3; i++) { const c = () => x; out.push(c()); let x = 'L' + i; out.push(c()); } console.log(out.join(','))"),
        "g,L0,g,L1,g,L2"
    );
}

#[test]
fn ident_hint_closure_per_iteration_bindings() {
    assert_eq!(
        one("const fs = []; for (let i = 0; i < 3; i++) fs.push(() => i); const vs = []; for (var j = 0; j < 3; j++) vs.push(() => j); console.log(fs.map(f => f()).join(), vs.map(f => f()).join())"),
        "0,1,2 3,3,3"
    );
}

#[test]
fn ident_hint_same_function_called_repeatedly() {
    assert_eq!(
        one("let k = 1; function f(a) { const t = a * 2; return t + k } let s = 0; for (let i = 0; i < 5; i++) s += f(i); k = 100; for (let i = 0; i < 5; i++) s += f(i); console.log(s)"),
        "545"
    );
}

#[test]
fn ident_hint_var_hoisting_and_redeclare() {
    assert_eq!(
        one("function h(c) { const out = []; out.push(typeof w); if (c) { var w = 'x'; } var w = 'y'; out.push(w); return out.join(); } const r = []; for (let i = 0; i < 3; i++) r.push(h(i % 2)); console.log(r.join('|'))"),
        "undefined,y|undefined,y|undefined,y"
    );
}

#[test]
fn ident_hint_function_hoisting_and_block_functions() {
    assert_eq!(
        run("console.log(early()); function early() { return 'H' } const r = []; for (let i = 0; i < 3; i++) { { function g() { return 'blk' + i } } r.push(g()); } console.log(r.join())"),
        vec!["H".to_string(), "blk0,blk1,blk2".to_string()]
    );
}

#[test]
fn ident_hint_named_function_expression_self_binding() {
    assert_eq!(
        one("const fact = function f(n) { return n <= 1 ? 1 : n * f(n - 1) }; const self = function g(g) { return g }; const s = []; for (let i = 1; i <= 5; i++) s.push(fact(i)); s.push(self(7), self(8)); console.log(s.join())"),
        "1,2,6,24,120,7,8"
    );
}

#[test]
fn ident_hint_arguments_as_local() {
    assert_eq!(
        run_sloppy_lines("function a1(p) { const r = [arguments[0]]; var arguments = 'loc'; r.push(arguments); return r.join(); } function a2() { return arguments.length } const out = []; for (let i = 0; i < 2; i++) { out.push(a1(i + 10)); out.push(a2(1, 2, 3)); } console.log(out.join('|'))"),
        vec!["10,loc|3|11,loc|3".to_string()]
    );
}

#[test]
fn ident_hint_catch_param_shadowing() {
    assert_eq!(
        one("let e = 'outer'; const out = []; function peek() { return e } for (let i = 0; i < 2; i++) { try { throw i } catch (e) { out.push(e, peek()) } out.push(e) } console.log(out.join())"),
        "0,outer,outer,1,outer,outer"
    );
}

#[test]
fn ident_hint_class_field_and_static_reads() {
    assert_eq!(
        one("let k = 'K'; class A { static s = k; f = k; static m() { return k } } const out = []; for (let i = 0; i < 2; i++) { out.push(new A().f, A.s, A.m()); k = 'J'; } console.log(out.join())"),
        "K,K,K,J,K,J"
    );
}

#[test]
fn ident_hint_read_before_let_documents_no_tdz() {
    // Thaw keeps its no-TDZ behaviour: an early read resolves to the outer binding. node throws a
    // ReferenceError here; this test pins Thaw's behaviour and must not change with the hints.
    assert_eq!(
        one("let x = 'o'; const out = []; for (let i = 0; i < 2; i++) { out.push(x); let x = 'i' + i; out.push(x); } console.log(out.join())"),
        "o,i0,o,i1"
    );
}

#[test]
fn ident_hint_with_gains_and_loses_name() {
    assert_eq!(
        one_sloppy("var x = 'g'; const obj = {}; const out = []; function getx() { return x } const o2 = { x: 'W' }; out.push(getx()); with (o2) { out.push(getx(), x) } with (obj) { for (let i = 0; i < 4; i++) { if (i === 1) obj.x = 'w'; if (i === 2) delete obj.x; out.push(x); } } out.push(getx()); console.log(out.join())"),
        "g,g,W,g,w,g,g,g"
    );
}

#[test]
fn ident_hint_delete_implicit_global_between_reads() {
    assert_eq!(
        one_sloppy("function rd() { try { return zz } catch (e) { return 'RE' } } function rd2() { return zz2 } const out = []; zz = 1; out.push(rd()); out.push(rd()); delete zz; out.push(rd()); zz = 2; out.push(rd()); a1 = 1; a2 = 2; a3 = 3; zz2 = 'z'; out.push(rd2()); delete a2; out.push(rd2()); console.log(out.join())"),
        "1,1,RE,2,z,z"
    );
}

#[test]
fn ident_hint_over_16_vars_uses_hash_index() {
    let decls: Vec<String> = (0..20).map(|i| format!("var v{i} = {i};")).collect();
    let src = format!(
        "function big() {{ {} let s = 0; for (let i = 0; i < 3; i++) {{ s += v0 + v5 + v17 + v19 + i; }} return s; }} console.log(big());",
        decls.join(" ")
    );
    assert_eq!(one(&src), "126");
}

#[test]
fn ident_hint_generator_reads_across_yields() {
    assert_eq!(
        one("const k = 10; function* gen() { let t = 0; for (let i = 0; i < 3; i++) { t += k + i; yield t; } } const out = []; for (const v of gen()) out.push(v); let kk = 1; function* g2() { for (let i = 0; i < 3; i++) { yield kk; kk++; } } for (const v of g2()) out.push(v); console.log(out.join())"),
        "10,21,33,1,2,3"
    );
}

#[test]
fn ident_hint_async_reads_across_awaits() {
    let (out, code) = main_run(
        "let base = 5; async function step(i) { await null; return base + i } async function main2() { const out = []; for (let i = 0; i < 3; i++) { out.push(await step(i)); base++; } console.log(out.join()); } main2();",
    );
    assert_eq!(out, vec!["5,7,9".to_string()]);
    assert_eq!(code, 0);
}

#[test]
fn ident_hint_recursion_reads_per_frame() {
    assert_eq!(
        one("function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2) } function depth(n) { const t = n; return n ? depth(n - 1) + t : 0 } console.log(fib(15), depth(20))"),
        "610 210"
    );
}

#[test]
fn ident_hint_same_function_node_under_different_parents() {
    assert_eq!(
        one("function outer(v) { let x = v; return function () { return x } } const f1 = outer(1), f2 = outer(2), f3 = outer(3); const out = []; for (let i = 0; i < 2; i++) out.push(f1(), f2(), f3()); console.log(out.join())"),
        "1,2,3,1,2,3"
    );
}

#[test]
fn ident_hint_typeof_and_numeric_rmw_paths() {
    assert_eq!(
        one("let n = 2; let s = 0; const out = []; for (let i = 0; i < 4; i++) { s += n * 1.5; s = s + n; out.push(typeof missing9); } console.log(s, out[3], typeof n)"),
        "20 undefined number"
    );
}

#[test]
fn ident_hint_deep_scope_chain_over_hop_limit() {
    // 300 nested lexical blocks: the read of `x` is 300 hops away, beyond what a hint can store.
    let open: String = (0..300).map(|i| format!("{{ let a{i} = {i};")).collect();
    let close = "}".repeat(300);
    let src = format!(
        "let x = 'deep'; const out = []; for (let r = 0; r < 2; r++) {{ {open} out.push(x); {close} }} console.log(out.join())"
    );
    assert_eq!(one(&src), "deep,deep");
}

#[test]
fn ident_hint_new_function_bodies_grow_table_boundedly() {
    let mut it = Interp::new();
    let _ = it.run("let s = 0; for (let i = 0; i < 100000; i++) { s += new Function('a', 'return a + 1')(i); } console.log(s)");
    assert_eq!(it.out, vec!["5000050000".to_string()]);
    assert!(
        it.ident_hints.len() < 1_000_000,
        "hint table grew to {}",
        it.ident_hints.len()
    );
}

#[test]
fn ident_hint_mask_invariant_after_many_declarations() {
    let mut it = Interp::new();
    let _ = it.run_sloppy("let a = 1; const b = 2; var c = 3; function d() {} class E {} for (const f of [1]) {} try { throw 1 } catch (g) {} switch (1) { case 1: let h = 1; } x9 = 4; delete x9; with ({ y: 1 }) { y } (function () { var q = 1; let r = 2; function s() {} return q + r; })(); console.log(a + b + c);");
    assert_eq!(it.out, vec!["6".to_string()]);
    it.global.check_mask_invariant();
}

#[test]
fn ident_hint_strict_and_sloppy_functions() {
    assert_eq!(
        one("var x = 'g'; function s1() { 'use strict'; return x } function s2() { return x } const out = []; for (let i = 0; i < 3; i++) out.push(s1(), s2()); console.log(out.join())"),
        "g,g,g,g,g,g"
    );
}

#[test]
fn ident_hint_vm_context_sandbox_mutation() {
    let src = format!(
        "{}\nconst sb = {{ x: 'a', out: [] }}; const ctx = createContext(sb); runInContext(\"function rd() {{ return x }} out.push(rd()); x = 'b'; out.push(rd());\", ctx); sb.x = 'c'; runInContext(\"out.push(rd()); out.push(typeof rd);\", ctx); console.log(sb.out.join());",
        include_str!("../../thaw-registry/src/registry/builtins/system/vm_context.js")
    );
    assert_eq!(one(&src), "a,b,c,function");
}

#[test]
fn hint_stats_bcrypt_shape() {
    // A 40-name wrapper (its name filter saturates) around a bcrypt-like kernel called 1000 times.
    let vars: String = (0..40).map(|i| format!("var w{i} = {i};")).collect();
    let src = format!(
        "function wrap() {{ {vars} function enc(lr, off, P, S) {{ var l = lr[off], r = lr[off + 1], i = 0, n = 0; for (i = 0; i < 16; i++) {{ n = S[l >>> 24]; n += S[0x100 | ((l >> 16) & 0xff)]; r ^= n ^ P[i]; n = r; l ^= n; }} lr[off] = l; lr[off + 1] = r; }} return enc; }}
        const enc = wrap(); const P = []; for (let k = 0; k < 18; k++) P.push(k * 7919); const S = []; for (let k = 0; k < 1024; k++) S.push((k * 2654435761) >>> 0);
        const lr = [1, 2]; for (let c = 0; c < 1000; c++) enc(lr, 0, P, S); console.log(lr.length);"
    );
    let mut it = Interp::new();
    it.hint_stats.on = true;
    let _ = it.run(&src);
    assert_eq!(it.out, vec!["2".to_string()]);
    let st = &it.hint_stats;
    eprintln!("{}", st.summary());
    assert!(st.hit.get() > 0, "no hinted read hit: {}", st.summary());
    it.hint_stats.on = false;
}

#[test]
fn ident_hint_compound_assignment_targets_are_annotated() {
    // `n += v`, `r ^= v`, `l ^= v` read their target through `eval_ident`; the target ident must
    // carry a hint id like any other identifier read, so nothing in this kernel is unannotated.
    let src = "function enc(P) { var l = 1, r = 2, n = 0, i = 0; for (i = 0; i < 16; i++) { n += P[i]; r ^= n; l ^= r; } return l ^ r ^ n; }
        const P = []; for (let k = 0; k < 16; k++) P.push(k * 7919); let t = 0; for (let c = 0; c < 200; c++) t = enc(P); console.log(t);";
    let mut it = Interp::new();
    it.hint_stats.on = true;
    let _ = it.run(src);
    assert_eq!(it.out.len(), 1);
    let st = &it.hint_stats;
    eprintln!("{}", st.summary());
    assert_eq!(
        st.unannotated.get(),
        0,
        "unannotated reads remain: {}",
        st.summary()
    );
    assert!(st.hit.get() > 0, "no hinted read hit: {}", st.summary());
    it.hint_stats.on = false;
}

#[test]
fn hint_stats_package_factories_annotated() {
    // A bcrypt-shaped kernel inside a registry bundle factory: package code must be annotated at load.
    const BUNDLE: &str = r#"module.exports = (function(require){ var __thaw_bundle_entry_key = "m";
var __thaw_bundle_factories = { "m": function(module, exports, require) {
  var P = [], S = [];
  for (var k = 0; k < 18; k++) P.push(k * 7919);
  for (var k = 0; k < 1024; k++) S.push((k * 2654435761) >>> 0);
  function enc(lr) { var l = lr[0], r = lr[1], i = 0, n = 0; for (i = 0; i < 16; i++) { n = S[l >>> 24]; n = n + S[0x100 | ((l >> 16) & 0xff)]; r = r ^ (n ^ P[i]); n = r; l = l ^ n; } lr[0] = l; lr[1] = r; }
  exports.run = function () { var lr = [1, 2]; for (var c = 0; c < 1000; c++) enc(lr); return lr[0]; };
} };
var __thaw_bundle_require_maps = { "m": {} };
})(globalThis.require);"#;
    use crate::host::Package;
    let mut p = Package::load(BUNDLE).expect("loads");
    p.interp.hint_stats.on = true;
    assert!(p.call("run", &[]).is_ok());
    let st = &p.interp.hint_stats;
    eprintln!("{}", st.summary());
    assert!(st.hit.get() > 0, "no hinted read hit: {}", st.summary());
    assert!(
        st.unannotated.get() * 10 < st.reads.get(),
        "package reads left unannotated: {}",
        st.summary()
    );
    p.interp.hint_stats.on = false;
}

#[test]
fn ident_hint_conditional_let_layout_drift() {
    // `eval('var pre')` runs on some calls only, so `let t` sits at a different index of the
    // frame's list on each call. The read of `t` must follow the layout it actually sees.
    let mut it = Interp::new();
    it.hint_stats.on = true;
    let _ = it.run_sloppy("function f(c) { if (c) eval('var pre = 1'); let t = c ? 'T' : 'F'; return t } const out = []; for (let i = 0; i < 6; i++) out.push(f(i % 2 === 0)); console.log(out.join())");
    assert_eq!(it.out, vec!["T,F,T,F,T,F".to_string()]);
    eprintln!("{}", it.hint_stats.summary());
    it.hint_stats.on = false;
}

#[test]
fn ident_hint_mask_collision_resolves_both_names() {
    // Two names with the same filter bit, one bound in the outer scope and one in `f`. The read
    // of the outer name walks through `f`'s scope, whose bit is set, so it must take the by-name
    // path and still find the outer binding.
    let mut pair = None;
    'search: for i in 0..200 {
        for j in i + 1..200 {
            let (a, b) = (format!("q{i}"), format!("q{j}"));
            if name_bit(&a) == name_bit(&b) {
                pair = Some((a, b));
                break 'search;
            }
        }
    }
    let (a, b) = pair.expect("two names share a filter bit");
    let src = format!(
        "let {a} = 'outer'; function f() {{ let {b} = 'inner'; const out = []; for (let k = 0; k < 3; k++) out.push({a}, {b}); return out.join(','); }} console.log(f(), f());"
    );
    let mut it = Interp::new();
    it.hint_stats.on = true;
    let _ = it.run(&src);
    assert_eq!(
        it.out,
        vec!["outer,inner,outer,inner,outer,inner outer,inner,outer,inner,outer,inner".to_string()]
    );
    assert!(
        it.hint_stats.miss_mid_mask.get() > 0,
        "the colliding scope was never walked: {}",
        it.hint_stats.summary()
    );
    eprintln!("{}", it.hint_stats.summary());
    it.hint_stats.on = false;
}

// ----- no `prototype` object for methods, accessors and async functions (R5a) -----

#[test]
fn no_prototype_for_methods_accessors_and_async_functions() {
    assert_eq!(
        one(
            "var o = { m() {}, get g() { return 1; }, set s(v) {}, async am() {} };\n\
             class C { k() {} static s() {} get x() { return 1; } }\n\
             async function af() {}\n\
             var ae = async function () {};\n\
             var ar = async () => 1;\n\
             console.log(o.m.hasOwnProperty('prototype'), 'prototype' in o.m, \
             Object.getOwnPropertyDescriptor(o, 'g').get.hasOwnProperty('prototype'), \
             Object.getOwnPropertyDescriptor(o, 's').set.hasOwnProperty('prototype'), \
             o.am.hasOwnProperty('prototype'), C.prototype.k.hasOwnProperty('prototype'), \
             C.s.hasOwnProperty('prototype'), af.hasOwnProperty('prototype'), \
             ae.hasOwnProperty('prototype'), ar.hasOwnProperty('prototype'), \
             Object.getOwnPropertyNames(af).indexOf('prototype'))"
        ),
        "false false false false false false false false false false -1"
    );
}

#[test]
fn no_prototype_keeps_prototype_for_plain_functions_classes_and_generators() {
    assert_eq!(
        one("function f() {}\n\
             class C {}\n\
             function* g() {}\n\
             var o = { *gm() {} };\n\
             console.log(f.hasOwnProperty('prototype'), f.prototype.constructor === f, \
             C.prototype.constructor === C, g.hasOwnProperty('prototype'), \
             o.gm.hasOwnProperty('prototype'), typeof g.prototype)"),
        "true true true true true object"
    );
}

#[test]
fn no_prototype_new_works_on_plain_functions_and_classes() {
    assert_eq!(
        one("function P(a) { this.a = a; }\n\
             class K { constructor(v) { this.v = v; } }\n\
             var p = new P(3), k = new K(4);\n\
             console.log(p.a, p instanceof P, k.v, k instanceof K, P.prototype.isPrototypeOf(p))"),
        "3 true 4 true true"
    );
}

#[test]
fn no_prototype_async_and_method_closures_free_their_scope() {
    let mut it = Interp::new();
    assert!(it
        .run("async function zzNoProtoAsync() {} var zzo = { async zzNoProtoMethod() {} };")
        .is_ok());
    let find = |it: &Interp, n: &str| {
        it.fn_cache
            .values()
            .find(|d| d.name.as_deref() == Some(n))
            .cloned()
            .expect("cached definition")
    };
    let defs = [find(&it, "zzNoProtoAsync"), find(&it, "zzNoProtoMethod")];
    let scope = Scope::new(Some(it.global.clone()));
    let weak = Rc::downgrade(&scope);
    for i in 0..10_000 {
        drop(it.closure_from_def(defs[i % 2].clone(), &scope));
    }
    drop(scope);
    assert!(
        weak.upgrade().is_none(),
        "10k async functions / methods kept their scope alive"
    );
}

// ----- lazy `prototype` for ordinary functions (R5b): every observable result is unchanged -----

#[test]
fn lazy_prototype_key_order_is_unchanged_by_materialisation() {
    assert_eq!(
        one("function f(){}; f.a=1; console.log(JSON.stringify(Object.getOwnPropertyNames(f)), JSON.stringify(Reflect.ownKeys(f)), JSON.stringify(Object.keys(f)));"),
        r#"["length","name","prototype","a"] ["length","name","prototype","a"] ["a"]"#
    );
    assert_eq!(
        one("function f(){}; f.a=1; f.prototype; console.log(JSON.stringify(Object.getOwnPropertyNames(f)), JSON.stringify(Reflect.ownKeys(f)), JSON.stringify(Object.keys(f)));"),
        r#"["length","name","prototype","a"] ["length","name","prototype","a"] ["a"]"#
    );
    assert_eq!(
        one("var g = function(){}; console.log(JSON.stringify(Object.getOwnPropertyNames(g)));"),
        r#"["length","name","prototype"]"#
    );
    assert_eq!(
        one("var g = function(){}; g.prototype; g.x = 2; console.log(JSON.stringify(Object.getOwnPropertyNames(g)), JSON.stringify(Object.keys(g)));"),
        r#"["length","name","prototype","x"] ["x"]"#
    );
    // more than INDEX_MIN properties: the hash index must still find the placeholder
    assert_eq!(
        one("function m(){}; for (var i=0;i<12;i++) m['k'+i]=i; console.log(JSON.stringify(Object.getOwnPropertyNames(m)).length, JSON.stringify(Object.getOwnPropertyNames(m).slice(0,4)), Object.getOwnPropertyNames(m).indexOf('prototype')); m.prototype; console.log(JSON.stringify(Object.getOwnPropertyNames(m).slice(0,4)), Object.getOwnPropertyNames(m).indexOf('prototype'), Object.getOwnPropertyNames(m).length);"),
        r#"91 ["length","name","prototype","k0"] 2|["length","name","prototype","k0"] 2 15"#
    );
    // a later non-enumerable key stays after the prototype, as before
    assert_eq!(
        one("function f(){}; f.a=1; Object.defineProperty(f,'z',{value:1,enumerable:true}); console.log(JSON.stringify(Object.getOwnPropertyNames(f)), JSON.stringify(Object.keys(f)));"),
        r#"["length","name","prototype","a","z"] ["a","z"]"#
    );
    assert_eq!(
        one("function f(){}; f.z=3; console.log(Object.keys(Object.getOwnPropertyDescriptors(f)).join(','));"),
        "length,name,prototype,z"
    );
    assert_eq!(
        one("function f(){}; console.log(Object.getOwnPropertyNames(f).join(','), Object.getOwnPropertyDescriptor(f,'name').value, Object.getOwnPropertyDescriptor(f,'length').value);"),
        "length,name,prototype f 0"
    );
    assert_eq!(
        one("function f(){}; console.log(f.hasOwnProperty('prototype'), 'prototype' in f, Object.prototype.hasOwnProperty.call(f,'prototype'), Object.getOwnPropertyNames(f).indexOf('prototype'));"),
        "true true true 2"
    );
}

#[test]
fn lazy_prototype_descriptors_and_constructor_flags() {
    assert_eq!(
        one("function f(){}; var d = Object.getOwnPropertyDescriptor(f,'prototype'); console.log(d.writable, d.enumerable, d.configurable, typeof d.value, d.value.constructor === f);"),
        "true false false object true"
    );
    assert_eq!(
        one("function f(){}; f.prototype; var d = Object.getOwnPropertyDescriptor(f,'prototype'); console.log(d.writable, d.enumerable, d.configurable, typeof d.value, d.value.constructor === f);"),
        "true false false object true"
    );
    assert_eq!(
        one("function f(){}; var d = Object.getOwnPropertyDescriptor(f.prototype,'constructor'); console.log(d.writable, d.enumerable, d.configurable, d.value === f, Object.getOwnPropertyNames(f.prototype).join(','));"),
        "true false true true constructor"
    );
    assert_eq!(
        one("function f(){}; var all = Object.getOwnPropertyDescriptors(f); console.log(all.prototype.writable, all.prototype.enumerable, all.prototype.configurable, all.prototype.value.constructor === f);"),
        "true false false true"
    );
    assert_eq!(
        one("function f(){}; Object.defineProperty(f,'prototype',{value:{k:2}}); console.log(new f().k, JSON.stringify(Object.getOwnPropertyDescriptor(f,'prototype').writable));"),
        "2 true"
    );
    assert_eq!(
        one("function f(){}; console.log(Reflect.deleteProperty(f,'prototype'), f.hasOwnProperty('prototype'), f.propertyIsEnumerable('prototype'));"),
        "false true false"
    );
}

#[test]
fn lazy_prototype_new_instanceof_and_constructor_identity() {
    assert_eq!(
        one("function f(){ this.v = 1; }; console.log(f.prototype.constructor === f, new f().v, new f() instanceof f, new f().constructor === f, Object.getPrototypeOf(new f()) === f.prototype);"),
        "true 1 true true true"
    );
    assert_eq!(
        one("function f(){ this.v = 1; }; var o = new f(); console.log(o.v, o instanceof f, Object.getPrototypeOf(o) === f.prototype, f.prototype.constructor === f);"),
        "1 true true true"
    );
    assert_eq!(
        one("function f(){}; var p0 = f.prototype; f.prototype = {x:1}; console.log(new f().x, p0 === f.prototype, p0.constructor === f);"),
        "1 false true"
    );
    assert_eq!(
        one("function f(){}; f.prototype = {x:1}; console.log(new f().x, f.hasOwnProperty('prototype'));"),
        "1 true"
    );
    assert_eq!(
        one("function Base(){ this.b = 1; }; class D extends Base { constructor(){ super(); this.d = 2; } }; var x = new D(); console.log(x.b, x.d, Object.getPrototypeOf(D) === Base, x instanceof Base, x instanceof D);"),
        "1 2 true true true"
    );
    assert_eq!(
        one("function Base(){ this.b = 1; }; class D extends Base {}; console.log(new D().b, Object.getPrototypeOf(D.prototype) === Base.prototype);"),
        "1 true"
    );
    assert_eq!(
        one("function f(){ this.who = 'f'; }; function g(){}; g.prototype = {z: 9}; var r = Reflect.construct(f, [], g); console.log(r.who, Object.getPrototypeOf(r) === g.prototype, r.z);"),
        "f true 9"
    );
    assert_eq!(
        one("var p = new Proxy(function(){ this.q = 1 }, {}); var o = new p(); console.log(o.q, o instanceof p);"),
        "1 true"
    );
    assert_eq!(
        one("var P = new Proxy(function(){}, { construct(t, args, nt) { return { via: 'trap' }; } }); console.log(JSON.stringify(new P()));"),
        r#"{"via":"trap"}"#
    );
}

#[test]
fn lazy_prototype_integrity_and_seal_flags() {
    assert_eq!(
        one("function f(){}; console.log(Object.isFrozen(f), Object.isSealed(f), Object.isExtensible(f));"),
        "false false true"
    );
    assert_eq!(
        one("function f(){}; Object.freeze(f); var d = Object.getOwnPropertyDescriptor(f,'prototype'); console.log(Object.isFrozen(f), Object.isSealed(f), d.writable, d.configurable, d.enumerable, d.value.constructor === f);"),
        "true true false false false true"
    );
    assert_eq!(
        one("function f(){}; f.prototype; Object.freeze(f); var d = Object.getOwnPropertyDescriptor(f,'prototype'); console.log(Object.isFrozen(f), d.writable, d.configurable, d.value.constructor === f);"),
        "true false false true"
    );
    assert_eq!(
        one("function f(){}; Object.seal(f); var d = Object.getOwnPropertyDescriptor(f,'prototype'); console.log(Object.isSealed(f), Object.isFrozen(f), d.writable, d.configurable, d.enumerable);"),
        "true false true false false"
    );
    assert_eq!(
        one("function f(){}; Object.preventExtensions(f); console.log(Object.isExtensible(f), Object.isFrozen(f), Object.isSealed(f), JSON.stringify(Object.getOwnPropertyNames(f)));"),
        r#"false false true ["length","name","prototype"]"#
    );
    assert_eq!(
        one("function f(){}; Object.freeze(f); f.prototype.q = 1; console.log(f.prototype.q, Object.isFrozen(f.prototype));"),
        "1 false"
    );
}

#[test]
fn lazy_prototype_json_assign_spread_and_inspect_skip_it() {
    assert_eq!(
        one("function f(){}; f.a = 1; console.log(JSON.stringify({fn: f, n: 1}), JSON.stringify(Object.assign({}, f)), JSON.stringify({...f}), JSON.stringify(Object.entries(f)));"),
        r#"{"n":1} {"a":1} {"a":1} [["a",1]]"#
    );
    assert_eq!(
        one("function f(){}; f.prototype; f.a = 1; console.log(JSON.stringify({fn: f, n: 1}), JSON.stringify(Object.assign({}, f)), JSON.stringify({...f}), JSON.stringify(Object.entries(f)));"),
        r#"{"n":1} {"a":1} {"a":1} [["a",1]]"#
    );
    assert_eq!(
        one("function f(){}; console.log(f); console.log(f.prototype.constructor === f, f);"),
        "[Function: f]|true [Function: f]"
    );
    assert_eq!(
        one("function f(){}; f.e = 1; var ks = []; for (var k in f) ks.push(k); console.log(JSON.stringify(ks));"),
        r#"["e"]"#
    );
}

#[test]
fn lazy_prototype_generators_arrows_methods_and_bound_functions() {
    assert_eq!(
        one("function* gen(){}; var d = Object.getOwnPropertyDescriptor(gen,'prototype'); console.log(JSON.stringify(Object.getOwnPropertyNames(gen)), d.writable, d.enumerable, d.configurable, Object.getPrototypeOf(gen.prototype) === Object.getPrototypeOf(function*(){}).prototype, typeof gen.prototype);"),
        r#"["length","name","prototype"] true false false false object"#
    );
    assert_eq!(
        one("function* gen(){ yield 1; yield 2; }; console.log([...gen()].join(','), Object.getPrototypeOf(gen()) === gen.prototype);"),
        "1,2 true"
    );
    assert_eq!(
        one("var o = { m() {} }; var ar = () => 1; async function af(){}; console.log(o.m.hasOwnProperty('prototype'), ar.hasOwnProperty('prototype'), af.hasOwnProperty('prototype'), JSON.stringify(Object.getOwnPropertyNames(ar)), JSON.stringify(Object.getOwnPropertyNames(o.m)));"),
        r#"false false false ["length","name"] ["length","name"]"#
    );
    assert_eq!(
        one("function f(a){ this.k = a; }; var b = f.bind(null, 5); console.log(b.hasOwnProperty('prototype'), JSON.stringify(Object.getOwnPropertyNames(b)), new b().k, new b() instanceof f);"),
        r#"false ["length","name"] 5 true"#
    );
    assert_eq!(
        one("function f(a,b){}; console.log(f.call(null), f.apply(null, []), f.name, f.length, JSON.stringify(Object.getOwnPropertyNames(f)));"),
        r#"undefined undefined f 2 ["length","name","prototype"]"#
    );
    assert_eq!(
        one("function F(){}; F.prototype.m = function(){}; var inst = new F(); console.log(inst.m === F.prototype.m, JSON.stringify(Object.getOwnPropertyNames(F.prototype)), Object.getOwnPropertyDescriptor(F,'prototype').writable);"),
        r#"true ["constructor","m"] true"#
    );
    assert_eq!(
        one("function mk(){ return function(){}; }; var a = mk(), b = mk(); console.log(a.prototype !== b.prototype, a.prototype.constructor === a, b.prototype.constructor === b);"),
        "true true true"
    );
}

#[test]
fn lazy_prototype_plain_closures_free_their_scope() {
    let mut it = Interp::new();
    assert!(it.run("function zzLazyPlain() {}").is_ok());
    let def = it
        .fn_cache
        .values()
        .find(|d| d.name.as_deref() == Some("zzLazyPlain"))
        .cloned()
        .expect("cached definition");
    let scope = Scope::new(Some(it.global.clone()));
    let weak = Rc::downgrade(&scope);
    for _ in 0..10_000 {
        drop(it.closure_from_def(def.clone(), &scope));
    }
    drop(scope);
    assert!(
        weak.upgrade().is_none(),
        "10k plain functions that never touched `prototype` kept their scope alive"
    );
}

#[test]
fn lazy_prototype_name_length_call_and_bind_do_not_materialise_it() {
    let mut it = Interp::new();
    assert!(it.run("function zzLazyRead(a, b) {}").is_ok());
    let def = it
        .fn_cache
        .values()
        .find(|d| d.name.as_deref() == Some("zzLazyRead"))
        .cloned()
        .expect("cached definition");
    let scope = Scope::new(Some(it.global.clone()));
    let weak = Rc::downgrade(&scope);
    for _ in 0..10_000 {
        let f = it.closure_from_def(def.clone(), &scope);
        assert!(it.get(&f, "name").is_ok());
        assert!(it.get(&f, "length").is_ok());
        assert!(it.get(&f, "call").is_ok());
        assert!(it.get(&f, "bind").is_ok());
    }
    drop(scope);
    assert!(
        weak.upgrade().is_none(),
        "reading name/length/call/bind materialised `prototype` and kept the scope alive"
    );
}

// ----- `Response | Promise<Response>`-style returns: flag bit 2 awaits only a returned Promise -----

const UNION_RETURN_BUNDLE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"u/index.js": function(module, exports, require) {
  class Hub {
    later(v) { return Promise.resolve(v * 3); }
    twice(n) { return n * 2; }
    boom() { return Promise.reject(new TypeError('nope')); }
  }
  exports.Hub = Hub;
},
};
var __thaw_bundle_require_maps = { "u/index.js": {}, };
var __thaw_bundle_entry_key = "u/index.js";
})(globalThis.require);"#;

fn class_call_with_flags(
    symbol: &str,
    args: &str,
    receiver: i64,
    flags: u8,
) -> Result<String, String> {
    use std::ffi::{CStr, CString};
    let (symbol, args) = (CString::new(symbol).unwrap(), CString::new(args).unwrap());
    let result = crate::host::thaw_tjs_class_call(symbol.as_ptr(), args.as_ptr(), receiver, flags);
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
fn class_call_bit_two_settles_a_promise_result_only() {
    use std::ffi::CString;
    let (name, source) = (
        CString::new("u").unwrap(),
        CString::new(UNION_RETURN_BUNDLE).unwrap(),
    );
    assert_eq!(
        crate::host::thaw_tjs_register(name.as_ptr(), source.as_ptr()),
        1
    );
    let hub: i64 = class_call_with_flags("$new$u::Hub$arity0", "[]", 0, 1)
        .unwrap()
        .parse()
        .unwrap();
    // bit 2 alone settles a returned Promise to its value
    assert_eq!(
        class_call_with_flags("$method$u::Hub$later$overload0$arity1", "[5]", hub, 4).unwrap(),
        "15"
    );
    // a non-Promise result is passed through unchanged
    assert_eq!(
        class_call_with_flags("$method$u::Hub$twice$overload0$arity1", "[4]", hub, 4).unwrap(),
        "8"
    );
    // a rejected Promise under bit 2 is an error, like bit 1
    assert_eq!(
        class_call_with_flags("$method$u::Hub$boom$overload0$arity0", "[]", hub, 4).unwrap_err(),
        "TypeError: nope"
    );
    // bit 1 keeps its meaning
    assert_eq!(
        class_call_with_flags("$method$u::Hub$later$overload0$arity1", "[2]", hub, 2).unwrap(),
        "6"
    );
}

const SETTER_BUNDLE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"s/index.js": function(module, exports, require) {
  class Cell {
    constructor() { this.v = 'a'; }
    get() { return this.v; }
    freeze() { Object.freeze(this); return this; }
  }
  exports.Cell = Cell;
},
};
var __thaw_bundle_require_maps = { "s/index.js": {}, };
var __thaw_bundle_entry_key = "s/index.js";
})(globalThis.require);"#;

#[test]
fn class_setter_op_assigns_through_strict_set_and_throws_on_a_frozen_receiver() {
    use std::ffi::CString;
    let (name, source) = (
        CString::new("s").unwrap(),
        CString::new(SETTER_BUNDLE).unwrap(),
    );
    assert_eq!(
        crate::host::thaw_tjs_register(name.as_ptr(), source.as_ptr()),
        1
    );
    let cell: i64 = class_call_with_flags("$new$s::Cell$arity0", "[]", 0, 1)
        .unwrap()
        .parse()
        .unwrap();
    // the setter returns the assigned value; the write is visible to a method reading it back
    assert_eq!(
        class_call_with_flags("$setter$s::Cell$v$overload0$arity1", "[\"b\"]", cell, 0).unwrap(),
        "\"b\""
    );
    assert_eq!(
        class_call_with_flags("$method$s::Cell$get$overload0$arity0", "[]", cell, 0).unwrap(),
        "\"b\""
    );
    // strict: a write to a frozen receiver throws (Interp::set alone would ignore it)
    class_call_with_flags("$method$s::Cell$freeze$overload0$arity0", "[]", cell, 0).unwrap();
    let error = class_call_with_flags("$setter$s::Cell$v$overload0$arity1", "[\"c\"]", cell, 0)
        .unwrap_err();
    assert!(
        error.contains("Cannot assign to read only property 'v'"),
        "{error}"
    );
    assert_eq!(
        class_call_with_flags("$method$s::Cell$get$overload0$arity0", "[]", cell, 0).unwrap(),
        "\"b\""
    );
}

/// The `"$kind$` markers a generator source emits (`"$getter$`, `"$staticmethod$`, ...); a `$name$` in a doc
/// comment is not preceded by a quote and is skipped.
fn generated_class_op_kinds(source: &str) -> std::collections::BTreeSet<String> {
    let mut kinds = std::collections::BTreeSet::new();
    for (index, _) in source.match_indices("\"$") {
        let rest = &source[index + 2..];
        let kind: String = rest
            .chars()
            .take_while(|c| c.is_ascii_lowercase())
            .collect();
        if !kind.is_empty() && rest[kind.len()..].starts_with('$') {
            kinds.insert(kind);
        }
    }
    kinds
}

#[test]
fn every_generated_class_op_kind_is_handled_by_thaw_js() {
    // The kinds are read from the generator sources, not listed by hand: a kind emitted by shim_support.rs,
    // shim_generation.rs or thaw-hir lower.rs that class_call does not handle fails here (its "unsupported
    // class operation" error is the one the thaw-llvm build refusal used to stand for).
    let generated: std::collections::BTreeSet<String> = [
        include_str!("../../thaw-cli/src/registry_integration/shim_support.rs"),
        include_str!("../../thaw-cli/src/registry_integration/shim_generation.rs"),
        include_str!("../../thaw-hir/src/lower.rs"),
    ]
    .into_iter()
    .flat_map(generated_class_op_kinds)
    .collect();
    assert!(
        [
            "new",
            "method",
            "getter",
            "setter",
            "staticgetter",
            "staticsetter"
        ]
        .iter()
        .all(|kind| generated.contains(*kind)),
        "scanner found {generated:?}"
    );
    for kind in &generated {
        let symbol = format!("${kind}$s::Cell$v$overload0$arity0");
        if let Err(error) = class_call_with_flags(&symbol, "[]", 0, 0) {
            assert!(
                !error.contains("unsupported class operation"),
                "{symbol} (generated kind `{kind}`): {error}"
            );
        }
    }
}

const GETTER_BUNDLE: &str = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"g/index.js": function(module, exports, require) {
  class Box {
    constructor() { this.n = 4; }
    get doubled() { return this.n * 2; }
  }
  class Cfg {
    static freeze() { Object.freeze(this); }
    static get w() { return 'w:' + Cfg.v; }
  }
  Cfg.v = 'a';
  exports.Box = Box;
  exports.Cfg = Cfg;
},
};
var __thaw_bundle_require_maps = { "g/index.js": {}, };
var __thaw_bundle_entry_key = "g/index.js";
})(globalThis.require);"#;

#[test]
fn class_getter_op_reads_the_property_through_the_receiver_handle() {
    use std::ffi::CString;
    let (name, source) = (
        CString::new("gi").unwrap(),
        CString::new(GETTER_BUNDLE).unwrap(),
    );
    assert_eq!(
        crate::host::thaw_tjs_register(name.as_ptr(), source.as_ptr()),
        1
    );
    let box_handle: i64 = class_call_with_flags("$new$gi::Box$arity0", "[]", 0, 1)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        class_call_with_flags("$getter$gi::Box$doubled", "[]", box_handle, 0).unwrap(),
        "8"
    );
    assert_eq!(
        class_call_with_flags("$getter$gi::Box$n", "[]", box_handle, 0).unwrap(),
        "4"
    );
}

#[test]
fn static_getter_op_reads_from_the_class_constructor() {
    use std::ffi::CString;
    let (name, source) = (
        CString::new("sg").unwrap(),
        CString::new(GETTER_BUNDLE).unwrap(),
    );
    assert_eq!(
        crate::host::thaw_tjs_register(name.as_ptr(), source.as_ptr()),
        1
    );
    assert_eq!(
        class_call_with_flags("$staticgetter$sg::Cfg$v", "[]", 0, 0).unwrap(),
        "\"a\""
    );
    assert_eq!(
        class_call_with_flags("$staticgetter$sg::Cfg$w", "[]", 0, 0).unwrap(),
        "\"w:a\""
    );
}

#[test]
fn static_setter_op_assigns_on_the_constructor_and_throws_on_a_frozen_one() {
    use std::ffi::CString;
    let (name, source) = (
        CString::new("ss").unwrap(),
        CString::new(GETTER_BUNDLE).unwrap(),
    );
    assert_eq!(
        crate::host::thaw_tjs_register(name.as_ptr(), source.as_ptr()),
        1
    );
    // the setter returns the assigned value; the write is visible to the static getter
    assert_eq!(
        class_call_with_flags("$staticsetter$ss::Cfg$v", "[\"b\"]", 0, 0).unwrap(),
        "\"b\""
    );
    assert_eq!(
        class_call_with_flags("$staticgetter$ss::Cfg$v", "[]", 0, 0).unwrap(),
        "\"b\""
    );
    // strict: a write to a frozen constructor throws (Interp::set alone would ignore it)
    class_call_with_flags("$staticmethod$ss::Cfg$freeze$overload0$arity0", "[]", 0, 0).unwrap();
    let error = class_call_with_flags("$staticsetter$ss::Cfg$v", "[\"c\"]", 0, 0).unwrap_err();
    assert!(
        error.contains("Cannot assign to read only property 'v'"),
        "{error}"
    );
    assert_eq!(
        class_call_with_flags("$staticgetter$ss::Cfg$v", "[]", 0, 0).unwrap(),
        "\"b\""
    );
}

// Node 22 prints: tick1, uncaught:boom, tick2, tick3, done (exit 0).
#[test]
fn set_interval_keeps_ticking_after_a_throwing_tick_under_uncaught_exception() {
    let (out, code) = main_run(
        "process.on('uncaughtException', function (e) { console.log('uncaught:' + e.message); });
         var n = 0; var t = setInterval(function () {
           n++; console.log('tick' + n);
           if (n === 1) throw new Error('boom');
           if (n === 3) { clearInterval(t); console.log('done'); }
         }, 1);",
    );
    assert_eq!(out.join("|"), "tick1|uncaught:boom|tick2|tick3|done");
    assert_eq!(code, 0);
}

/// TJ171: async bodies run on coroutine threads with their own counters; a timer id handed out there must not
/// equal a host-thread timer's id, or `clearTimeout` on the async side deletes the host's interval.
/// Count-based (not a sentinel delay) so a slow first loop pass cannot fake the result.
/// Node prints `ok` for the same script (scratch: thaw-tmp/tmp-run/tj171/timer_collide.js).
#[test]
fn async_body_timers_do_not_collide_with_host_thread_timer_ids() {
    let mut it = Interp::new();
    it.run_main(
        "let ticks = 0; \
         const iv = setInterval(() => { if (++ticks === 3) { console.log('ok'); clearInterval(iv); clearTimeout(fb); } }, 2); \
         async function other() { await null; const t = setTimeout(() => {}, 1000); clearTimeout(t); } \
         other(); \
         const fb = setTimeout(() => { console.log('stalled:' + ticks); clearInterval(iv); }, 3000);",
    )
    .expect("runs");
    assert_eq!(it.out.join("|"), "ok");
}

// ----- the terminal loop drives the compiled native http server (`thaw_http_poll_servers`) -----

/// Serialises the tests that install `TEST_SERVER_POLL` (process-wide).
static SERVER_POLL_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
static SERVER_POLLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Live (bit 0) for the first calls, then idle; the first call also reports a ran callback (bit 1).
extern "C" fn fake_server_poll() -> u8 {
    match SERVER_POLLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
        0 => 3,
        1..=4 => 1,
        _ => 0,
    }
}

/// Runs `body` on a fresh thread (host tables are thread-local) and fails instead of hanging.
fn guarded(body: impl FnOnce() + Send + 'static) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        body();
        let _ = tx.send(());
    });
    rx.recv_timeout(std::time::Duration::from_secs(20))
        .expect("event loop did not finish within 20s");
}

#[test]
fn terminal_loop_polls_native_servers_until_idle() {
    use crate::host::{thaw_tjs_run_event_loop, TEST_SERVER_POLL};
    let _guard = SERVER_POLL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    *TEST_SERVER_POLL.lock().unwrap() = Some(fake_server_poll);
    SERVER_POLLS.store(0, std::sync::atomic::Ordering::SeqCst);
    guarded(|| {
        thaw_tjs_run_event_loop();
    });
    *TEST_SERVER_POLL.lock().unwrap() = None;
    // before the fix the loop has no packages and breaks at once: 0 polls
    assert!(SERVER_POLLS.load(std::sync::atomic::Ordering::SeqCst) >= 6);
}

#[test]
fn package_timer_wait_is_capped_while_a_native_server_is_live() {
    use crate::host::{thaw_tjs_register, thaw_tjs_run_event_loop, TEST_SERVER_POLL};
    let _guard = SERVER_POLL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    *TEST_SERVER_POLL.lock().unwrap() = Some(fake_server_poll_40);
    SERVER_POLLS.store(0, std::sync::atomic::Ordering::SeqCst);
    guarded(|| {
        let (name, source) = (
            std::ffi::CString::new("tmr").unwrap(),
            std::ffi::CString::new(
                r#"module.exports = (function(require){
var __thaw_bundle_factories = { "tmr/index.js": function(module, exports, require) {
  exports.t = function () { setTimeout(function () {}, 150); };
}, };
var __thaw_bundle_require_maps = { "tmr/index.js": {}, };
var __thaw_bundle_entry_key = "tmr/index.js";
})(globalThis.require);"#,
            )
            .unwrap(),
        );
        assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
        typed_call("tmr::t", "[]").expect("arm timer");
        thaw_tjs_run_event_loop();
    });
    *TEST_SERVER_POLL.lock().unwrap() = None;
    // a 2 ms cap over the 150 ms timer gives ~40 polls while the server is live; a single sleep gives ~0
    assert!(SERVER_POLLS.load(std::sync::atomic::Ordering::SeqCst) >= 20);
}

/// Live for 40 calls, then idle (the 150 ms timer test).
extern "C" fn fake_server_poll_40() -> u8 {
    let n = SERVER_POLLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    u8::from(n < 40)
}

#[test]
fn q9_process_config_and_error_prepare_stack_trace_exist() {
    let (out, code) = main_run(
        "console.log(typeof process.config, typeof process.config.variables, typeof process.config.target_defaults);\n\
         console.log(Error.prepareStackTrace === undefined);\n\
         Error.prepareStackTrace = function (e, frames) { return 'x'; };\n\
         console.log(typeof Error.prepareStackTrace);",
    );
    assert_eq!(code, 0);
    assert_eq!(out.join("|"), "object object object|true|function");
}

#[test]
fn q9_scan_clean_for_process_config_and_prepare_stack_trace() {
    let src = "module.exports = (function(require){ var __thaw_bundle_factories = { \"p/index.js\": function(module, exports, require) { exports.c = process.config.variables; exports.p = Error.prepareStackTrace; } }; var __thaw_bundle_require_maps = { \"p/index.js\": {} }; var __thaw_bundle_entry_key = \"p/index.js\"; })(globalThis.require);";
    let report = crate::scan::scan_bundle(src).ok().unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
}

#[test]
fn q9_addon_mode_tolerates_method_exports_only() {
    let src = "module.exports = (function(require){ var __thaw_bundle_factories = { \"p/index.js\": function(module, exports, require) { var b = require('./build/Release/x.node'); exports.v = b.encrypt_sync('a'); } }; var __thaw_bundle_require_maps = { \"p/index.js\": {} }; var __thaw_bundle_entry_key = \"p/index.js\"; })(globalThis.require);";
    let plain = crate::scan::scan_bundle(src).ok().unwrap();
    assert!(
        plain.blockers.contains_key("method:encrypt_sync"),
        "{:?}",
        plain.blockers
    );
    let addon = crate::scan::scan_addon_bundle(src).ok().unwrap();
    assert!(addon.is_clean(), "{:?}", addon.blockers);
    // addon mode keeps the other rules: an unrelated direct eval is still reported
    let with_eval = src.replace(
        "exports.v = b.encrypt_sync('a');",
        "exports.v = b.encrypt_sync('a'); eval('1');",
    );
    let report = crate::scan::scan_addon_bundle(&with_eval).ok().unwrap();
    assert_eq!(
        report
            .blockers
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["direct-eval"]
    );
}

#[test]
fn scan_addon_mode_keeps_process_dlopen_blocked() {
    // process.dlopen is not served on thaw-js: addon mode must not tolerate a dlopen method either
    let bundle = r#"module.exports = (function(require){
var __thaw_bundle_factories = {
"addon/index.js": function(module, exports, require) {
  exports.load = function (h, m, p) { return process.dlopen(m, p) || h.dlopen(m, p); };
},
};
var __thaw_bundle_require_maps = { "addon/index.js": {}, };
var __thaw_bundle_entry_key = "addon/index.js";
})(globalThis.require);"#;
    let addon = crate::scan::scan_addon_bundle(bundle).ok().unwrap();
    assert!(
        !addon.is_clean(),
        "dlopen must stay blocked: {:?}",
        addon.blockers
    );
    assert!(
        addon.blockers.keys().any(|k| k == "method:dlopen"),
        "{:?}",
        addon.blockers
    );
}

/// W2 guard: reads of outer names through a 100-variable scope, and a shadowed name one hop deeper, give the
/// binding a by-name lookup gives (hinted reads included). Holds with or without the exact-membership check.
#[test]
fn hinted_reads_through_large_scopes_keep_the_visible_binding() {
    let decls: String = (0..100).map(|i| format!("let m{i} = {i};")).collect();
    let src = format!(
        "{decls} function f() {{ let m7 = 'inner'; return (() => m7 + m8)(); }} \
         function g() {{ return m50 + m99; }} console.log(f(), g(), f());"
    );
    let mut it = Interp::new();
    it.hint_stats.on = true;
    assert!(it.run(&src).is_ok());
    assert_eq!(it.out, vec!["inner8 149 inner8".to_string()]);
    let mut on = Interp::with_compile_mode(crate::compile::CompileMode::On);
    assert!(on.run(&src).is_ok());
    assert_eq!(on.out, it.out);
}

#[test]
fn release_all_handles_drops_every_live_handle_and_reports_the_count() {
    let first = crate::host::store_handle(0, Value::Number(1.0));
    let second = crate::host::store_handle(0, Value::Number(2.0));
    assert_eq!(crate::host::thaw_tjs_retain_handle(second), 1);
    assert_eq!(crate::host::thaw_tjs_release_all_handles(), 2);
    assert!(!crate::host::handle_is_live(first));
    assert!(!crate::host::handle_is_live(second));
    assert_eq!(crate::host::thaw_tjs_release_all_handles(), 0);
}

// Native uncaught/unhandled reports reach the `process` listeners registered in this realm.
#[test]
fn native_reports_emit_to_registered_process_listeners() {
    use crate::host::{
        thaw_tjs_emit_rejection_handled_result, thaw_tjs_emit_uncaught_result,
        thaw_tjs_emit_unhandled_rejection_result, thaw_tjs_load_script,
    };
    use std::ffi::CString;
    let src = CString::new(
        "globalThis.seen = []; process.on('unhandledRejection', function (r) { seen.push('rej:' + r); return true; });
         process.on('uncaughtException', function (e) { seen.push('unc:' + e.message); });
         process.on('rejectionHandled', function () { seen.push('handled'); });",
    )
    .unwrap();
    assert_eq!(thaw_tjs_load_script(src.as_ptr()), 1);

    let rejection = CString::new("boom").unwrap();
    let r = thaw_tjs_emit_unhandled_rejection_result(rejection.as_ptr());
    assert!(r.error.is_null());
    assert_eq!(r.value, 1, "a registered listener handles the rejection");

    let tagged = CString::new("\u{1}Error\u{1}oops").unwrap();
    let u = thaw_tjs_emit_uncaught_result(tagged.as_ptr());
    assert!(u.error.is_null());
    assert_eq!(u.value, 1, "a registered listener handles the exception");

    let h = thaw_tjs_emit_rejection_handled_result();
    assert!(h.error.is_null());
    assert_eq!(h.value, 1);

    let probe = CString::new(
        "if (seen.join() !== 'rej:boom,unc:oops,handled') throw new Error('got ' + seen.join());",
    )
    .unwrap();
    assert_eq!(thaw_tjs_load_script(probe.as_ptr()), 1);
}

#[test]
fn native_reports_are_unhandled_without_a_process_listener() {
    use crate::host::{
        thaw_tjs_emit_uncaught_result, thaw_tjs_emit_unhandled_rejection_result,
        thaw_tjs_load_script,
    };
    use std::ffi::CString;
    let src = CString::new("globalThis.quiet = 1;").unwrap();
    assert_eq!(thaw_tjs_load_script(src.as_ptr()), 1);
    let msg = CString::new("boom").unwrap();
    let r = thaw_tjs_emit_unhandled_rejection_result(msg.as_ptr());
    assert!(r.error.is_null());
    assert_eq!(r.value, 0);
    let u = thaw_tjs_emit_uncaught_result(msg.as_ptr());
    assert!(u.error.is_null());
    assert_eq!(u.value, 0);
}

/// Each due timer gets a turn-hook checkpoint (the compiled runtime drains native promise jobs there), so a
/// promise queued by `timer-1` runs before `timer-2`, as in Node.
#[test]
fn due_timers_each_get_a_turn_hook_checkpoint() {
    use std::sync::Mutex;
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    fn hook(it: &mut Interp) -> bool {
        let snapshot = it
            .run("events.join(',')")
            .map(|v| it.to_str(&v).to_string());
        SEEN.lock().unwrap().push(snapshot.unwrap_or_default());
        false
    }
    let mut it = Interp::new();
    it.turn_hook = Some(hook);
    it.run_main(
        "var events = []; setTimeout(() => events.push('timer-1'), 0); setTimeout(() => events.push('timer-2'), 0);",
    )
    .unwrap();
    let seen = SEEN.lock().unwrap().clone();
    assert!(
        seen.iter().any(|s| s == "timer-1"),
        "no checkpoint between timers: {seen:?}"
    );
}

/// `drain_ticks` runs only the `nextTick` queue; promise jobs stay queued until `run_jobs`.
#[test]
fn drain_ticks_runs_next_ticks_and_leaves_promise_jobs_queued() {
    let mut it = Interp::new();
    it.run("globalThis.log = []; process.nextTick(() => log.push('tick')); Promise.resolve().then(() => log.push('promise'));")
        .unwrap();
    it.drain_ticks().unwrap();
    let after_ticks = it.run("log.join(',')").unwrap();
    assert_eq!(it.to_str(&after_ticks).to_string(), "tick");
    it.run_jobs().unwrap();
    let after_jobs = it.run("log.join(',')").unwrap();
    assert_eq!(it.to_str(&after_jobs).to_string(), "tick,promise");
}

/// Node calls `uncaughtExceptionMonitor` listeners before `uncaughtException` for a thrown error.
#[test]
fn uncaught_exception_monitor_precedes_the_handler_for_a_timer_throw() {
    let mut it = Interp::new();
    it.run_main(
        "globalThis.order = []; \
         process.on('uncaughtExceptionMonitor', () => order.push('monitor')); \
         process.on('uncaughtException', () => order.push('handler')); \
         setTimeout(() => { throw new Error('boom'); }, 0);",
    )
    .unwrap();
    let order = it.run("order.join(',')").unwrap();
    assert_eq!(it.to_str(&order).to_string(), "monitor,handler");
}

/// Node: `Iterator.from` + the eager-looking helpers return the same values (checked with node 22).
#[test]
fn iterator_from_and_helpers_match_node() {
    assert_eq!(
        one("console.log(Iterator.from([1,2,3]).map(x=>x*2).toArray().join())"),
        "2,4,6"
    );
    assert_eq!(
        one("console.log(Iterator.from([1,2,3,4]).reduce((a,b)=>a+b,0), Iterator.from([1,2,3,4]).some(x=>x>3), Iterator.from([1,2,3]).every(x=>x>0), Iterator.from([1,2,3]).find(x=>x>1))"),
        "10 true true 2"
    );
    assert_eq!(
        one("console.log(Iterator.from([1,2]).flatMap(x=>[x,x*10]).toArray().join(), Iterator.from([1,2,3,4,5]).drop(2).take(2).toArray().join())"),
        "1,10,2,20 3,4"
    );
    assert_eq!(
        one("let c=0; Iterator.from([5,6]).forEach(x=>{c+=x}); console.log(c)"),
        "11"
    );
    assert_eq!(one("console.log(typeof Iterator, Object.getPrototypeOf(Iterator.prototype)===Object.prototype)"), "function true");
    assert_eq!(
        one("try { new Iterator(); } catch (e) { console.log(e.constructor.name); }"),
        "TypeError"
    );
}

/// Helpers stay lazy: `take` ends an infinite generator and nothing runs before a consumer pulls.
#[test]
fn iterator_helpers_are_lazy() {
    assert_eq!(
        one("function* nat(){let i=0; while(true) yield i++;} console.log(nat().map(x=>x*x).filter(x=>x%2).take(3).toArray().join())"),
        "1,9,25"
    );
    assert_eq!(
        one("let n=0; const it = Iterator.from({next(){n++; return {value:n,done:false}}}).map(x=>x); console.log('before', n); console.log(it.next().value, n)"),
        "before 0|1 1"
    );
}

/// Generator and Array iterators inherit the helpers from %IteratorPrototype%. (Map/Set iterators are
/// arrays in thaw-js, so they do not inherit it yet.)
#[test]
fn builtin_iterators_inherit_iterator_prototype() {
    assert_eq!(
        one("function* g(){yield 1; yield 2;} console.log(g() instanceof Iterator, [].values() instanceof Iterator)"),
        "true true"
    );
    assert_eq!(
        one("function* g(){yield 1; yield 2;} console.log(g().map(x=>x+1).toArray().join())"),
        "2,3"
    );
}

/// Node: `Map`/`Set` `keys()`/`values()`/`entries()` return real iterator objects (not arrays), with
/// `Map Iterator`/`Set Iterator` tags and the `Iterator.prototype` helpers.
#[test]
fn map_set_iterators_are_real_iterator_objects() {
    assert_eq!(
        one(
            r#"const it=new Map([[1,2]]).keys(); const si=new Set([1]).values();
console.log(Array.isArray(it), Object.prototype.toString.call(it), it instanceof Iterator, typeof it.map, it[Symbol.iterator]()===it, Object.prototype.toString.call(si), si instanceof Iterator, Object.getPrototypeOf(new Map().entries()) === Object.getPrototypeOf(new Map().keys()));"#
        ),
        "false [object Map Iterator] true function true [object Set Iterator] true true"
    );
}

/// Node: map/set iterators are live: additions and deletions during iteration are observed, and
/// `clear()` followed by `set()` keeps the old iterator going over the new entries.
#[test]
fn map_set_iterators_are_live() {
    assert_eq!(
        one(
            r#"const m=new Map(); m.set("a",1); m.set("b",2); m.set("c",3);
const k=m.keys(), v=m.values(), e=m.entries();
console.log(k.next().value, v.next().value, JSON.stringify(e.next().value));
m.delete("b"); m.set("c",30); m.set("d",4);
console.log(k.next().value, v.next().value, JSON.stringify(e.next().value));
m.clear(); m.set("a",10);
console.log(k.next().value, v.next().value, JSON.stringify(e.next().value));
console.log(k.next().done); m.set("e",5); console.log(k.next().done);
console.log(v.next().value, JSON.stringify(e.next().value));
const seen=[]; m.forEach((x,y)=>{seen.push(y+"="+x); if(y==="a"){m.delete("e"); m.set("f",6);}}); console.log(seen.join(","));
const s=new Set([1,2,3]); const si=s.values(), se=s.entries();
console.log(si.next().value, JSON.stringify(se.next().value));
s.delete(2); s.delete(1); s.add(1); s.add(4);
console.log(si.next().value, JSON.stringify(se.next().value));
console.log(si.next().value, JSON.stringify(se.next().value));
s.clear(); s.add(5);
console.log(si.next().value, JSON.stringify(se.next().value));
const sv=[]; s.forEach(x=>{sv.push(x); if(x===5) s.add(6);}); console.log(sv.join(","));"#
        ),
        [
            r#"a 1 ["a",1]"#,
            r#"c 30 ["c",30]"#,
            r#"a 10 ["a",10]"#,
            "true",
            "true",
            r#"5 ["e",5]"#,
            "a=10,f=6",
            "1 [1,1]",
            "3 [3,3]",
            "1 [1,1]",
            "5 [5,5]",
            "5,6",
        ]
        .join("|")
    );
}

/// Node: a cursor survives deletion of an already-visited entry, and `clear()` restarts it.
#[test]
fn map_iterator_cursor_survives_delete_and_clear() {
    assert_eq!(
        one(
            r#"const m=new Map([[1,1],[2,2],[3,3]]); const it=m.keys(); console.log(it.next().value); m.delete(1); console.log(it.next().value, it.next().value, it.next().done);
const m2=new Map([['a',1]]); const it2=m2.keys(); it2.next(); m2.clear(); m2.set('b',2); console.log(it2.next().value, it2.next().done);"#
        ),
        "1|2 3 true|b true"
    );
}

/// Node: `next` on an incompatible receiver throws a TypeError; `next.call` on a real iterator works.
#[test]
fn collection_iterator_next_validates_receiver() {
    assert_eq!(
        one(
            r#"const n=new Map().keys().next; try{n()}catch(e){console.log(e instanceof TypeError)} try{n.call(new Set().values())}catch(e){console.log(e instanceof TypeError)} console.log(n.call(new Map([['x',1]]).keys()).value);"#
        ),
        "true|true|x"
    );
}

/// Node: spread over a map iterator calls its own (overridden) `next`, so its error propagates.
#[test]
fn collection_iterator_spread_honours_overridden_next() {
    assert_eq!(
        one(
            r#"const it=new Map([[1,2]]).keys(); it.next=()=>{throw new Error('next failed')}; try{[...it]}catch(e){console.log(e.message)}"#
        ),
        "next failed"
    );
}

/// Node: iterator helpers chain over map/set iterators.
#[test]
fn map_set_iterator_helpers_chain() {
    assert_eq!(
        one(
            r#"console.log(new Map([['a',1],['b',2]]).keys().map(x=>x+'!').toArray().join(), new Set([1,2,3]).values().filter(x=>x>1).take(1).toArray().join());"#
        ),
        "a!,b! 2"
    );
}

/// Node: `JSON.rawJSON` values serialize as their raw text and are frozen null-prototype objects.
#[test]
fn json_raw_json_matches_node() {
    assert_eq!(
        one("console.log(JSON.stringify({a: JSON.rawJSON('123456789012345678901234567890')}))"),
        r#"{"a":123456789012345678901234567890}"#
    );
    assert_eq!(
        one("const r = JSON.rawJSON('1'); console.log(JSON.isRawJSON(r), JSON.isRawJSON({}), Object.isFrozen(r), Object.getPrototypeOf(r)===null, r.rawJSON)"),
        "true false true true 1"
    );
    assert_eq!(
        one("try { JSON.rawJSON('{}'); } catch (e) { console.log(e.constructor.name); }"),
        "SyntaxError"
    );
}

/// `process.hrtime.bigint()` returns a monotonic BigInt of nanoseconds.
#[test]
fn process_hrtime_bigint_is_monotonic_bigint() {
    assert_eq!(
        one("const a=process.hrtime.bigint(); const b=process.hrtime.bigint(); console.log(typeof a, a > 0n, b >= a)"),
        "bigint true true"
    );
}

// A framed Error built with no package loaded (the registered error factory of a program with no JS values).
#[test]
fn thaw_tjs_error_from_tagged_handle_builds_with_no_package_loaded() {
    use crate::host::*;
    use std::ffi::CString;
    std::thread::spawn(|| {
        let framed = CString::new("\u{1}TypeError\u{1}boom").unwrap();
        let made = thaw_tjs_error_from_tagged_handle(framed.as_ptr());
        assert!(made.error.is_null());
        assert_ne!(made.value, 0);
        let key = CString::new("message").unwrap();
        assert!(thaw_tjs_get_property_result(made.value, key.as_ptr())
            .error
            .is_null());
    })
    .join()
    .unwrap();
}

/// Node: SharedArrayBuffer is its own constructor/prototype (not an ArrayBuffer), `slice` copies.
#[test]
fn shared_array_buffer_matches_node() {
    assert_eq!(
        one("const b=new SharedArrayBuffer(8); console.log(b.byteLength, b.slice(2).byteLength, b instanceof SharedArrayBuffer, b instanceof ArrayBuffer, Object.prototype.toString.call(b))"),
        "8 6 true false [object SharedArrayBuffer]"
    );
    assert_eq!(
        one("console.log(new Uint8Array(new SharedArrayBuffer(4)).length, new DataView(new SharedArrayBuffer(4), 1).byteLength)"),
        "4 3"
    );
}

/// Node: Atomics read-modify-write ops return the OLD value; store returns the stored value.
#[test]
fn atomics_return_old_value_matches_node() {
    assert_eq!(
        one("const a=new Int32Array(new SharedArrayBuffer(16)); console.log(Atomics.store(a,0,5), Atomics.add(a,0,3), Atomics.sub(a,0,2), Atomics.exchange(a,0,7), Atomics.compareExchange(a,0,7,9), Atomics.compareExchange(a,0,1,4), Atomics.load(a,0))"),
        "5 5 8 6 7 9 9"
    );
    assert_eq!(
        one("const a=new Int32Array(new SharedArrayBuffer(4)); Atomics.store(a,0,10); console.log(Atomics.and(a,0,6), Atomics.or(a,0,1), Atomics.xor(a,0,3), Atomics.load(a,0))"),
        "10 2 3 0"
    );
}

/// Node: Atomics rejects non-integer typed arrays (TypeError) and out-of-range indices (RangeError).
#[test]
fn atomics_argument_errors_match_node() {
    assert_eq!(
        one("try { Atomics.add(new Float64Array(2),0,1) } catch(e){ console.log(e.constructor.name) }"),
        "TypeError"
    );
    assert_eq!(
        one("try { Atomics.load(new Uint8ClampedArray(1),0) } catch(e){ console.log(e.constructor.name) }"),
        "TypeError"
    );
    assert_eq!(
        one(
            "try { Atomics.load(new Int32Array(1),5) } catch(e){ console.log(e.constructor.name) }"
        ),
        "RangeError"
    );
}

/// Node: Atomics.wait on the main thread returns "not-equal" or "timed-out"; notify with no waiters is 0.
#[test]
fn atomics_wait_and_notify_match_node() {
    assert_eq!(
        one("const ia=new Int32Array(new SharedArrayBuffer(8)); console.log(Atomics.wait(ia,0,1,0), Atomics.wait(ia,0,0,0), Atomics.wait(ia,0,0,5), Atomics.notify(ia,0,1))"),
        "not-equal timed-out timed-out 0"
    );
    assert_eq!(
        one("console.log(Atomics.isLockFree(4), Object.prototype.toString.call(Atomics))"),
        "true [object Atomics]"
    );
}

/// Host property predicates 3 (own and enumerable) and 4 (hide). Lookups and the `missing` key
/// must not create a property.
#[test]
fn host_property_predicates_enumerable_and_hide() {
    let mut it = Interp::new();
    assert!(it
        .run("var o = {a: 1}; Object.defineProperty(o, 'h', {value: 2, enumerable: false});")
        .is_ok());
    let o = it.lookup(&it.global.clone(), "o").unwrap();
    let pred = |it: &mut Interp, key: &str, op: u8| {
        crate::host::property_predicate_value(it, &o, key, op).ok()
    };
    assert_eq!(pred(&mut it, "a", 3), Some(1));
    assert_eq!(pred(&mut it, "h", 3), Some(0));
    assert_eq!(pred(&mut it, "missing", 3), Some(0));
    assert_eq!(pred(&mut it, "missing", 4), Some(0));
    assert_eq!(pred(&mut it, "missing", 1), Some(0));
    assert_eq!(pred(&mut it, "a", 4), Some(1));
    assert_eq!(pred(&mut it, "a", 3), Some(0));
    assert_eq!(pred(&mut it, "a", 1), Some(1));
    assert!(it.run("console.log(Object.keys(o).length, o.a);").is_ok());
    assert_eq!(it.out.join("|"), "0 1");
}

/// for-in keys (prelude operation 9): enumerable string keys along the prototype chain, with a
/// shadowing non-enumerable own property hiding the inherited name.
#[test]
fn host_for_in_keys_walk_prototype_chain_with_shadowing() {
    let mut it = Interp::new();
    assert!(it.run_sloppy(include_str!("dynamic_prelude.js")).is_ok());
    assert!(it
        .run("const p = {a: 1, b: 2, s: 3}; const o = Object.create(p); Object.defineProperty(o, 's', {value: 9, enumerable: false}); o.c = 4; console.log(__thaw_json_host_enumerate(o, 9).join(','));")
        .is_ok());
    assert_eq!(it.out.join("|"), "c,a,b");
}

/// Standard globals that thaw-hir lowers by name (`platform_globals`) must
/// exist in the thaw-js realm, or be listed here. Shrink-only: an entry is
/// removed in the PR that implements it, and a stale entry fails this test.
const ABSENT_IN_THAW_JS: &[&str] = &[
    "fetch",
    "WebSocket",
    "Crypto",
    "CryptoKey",
    "SubtleCrypto",
    "DisposableStack",
    "AsyncDisposableStack",
];

#[test]
fn platform_globals_exist_in_thaw_js_realm_or_are_ledgered() {
    // ES typed arrays are lowered by thaw-hir's intrinsic arm, not the table.
    let names: Vec<&str> = thaw_hir::platform_globals::PLATFORM_GLOBALS
        .iter()
        .map(|(name, _)| *name)
        .chain(["BigInt64Array", "BigUint64Array"])
        .collect();
    let list = names
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(",");
    // `Intl` is installed only with the `intl` feature.
    let intl_gated: &[&str] = if cfg!(feature = "intl") {
        &[]
    } else {
        &["Intl"]
    };
    let expected: Vec<&str> = ABSENT_IN_THAW_JS
        .iter()
        .chain(intl_gated)
        .copied()
        .collect();
    let mut it = Interp::new();
    it.run(&format!(
        "const missing = []; for (const n of [{list}]) if (typeof globalThis[n] === 'undefined') missing.push(n); console.log(missing.join(' '));"
    ))
    .expect("probe runs");
    let missing: Vec<&str> = it
        .out
        .last()
        .expect("probe prints")
        .split_whitespace()
        .collect();
    let unexpected: Vec<&&str> = missing
        .iter()
        .filter(|name| !expected.contains(name))
        .collect();
    let stale: Vec<&&str> = expected
        .iter()
        .filter(|name| !missing.contains(name))
        .collect();
    assert!(unexpected.is_empty(), "platform globals missing from the thaw-js realm (add to ABSENT_IN_THAW_JS or implement): {unexpected:?}");
    assert!(
        stale.is_empty(),
        "ABSENT_IN_THAW_JS entries now exist; remove them: {stale:?}"
    );
}

/// `[Symbol.toPrimitive]` hints and the non-callable rejection, values recorded with Node 22.23.1.
#[test]
fn symbol_to_primitive_hints_match_node() {
    assert_eq!(
        one("class Q { [Symbol.toPrimitive](h) { return 'Q:' + h; } } var q = new Q(); console.log(q + '', `${q}`, q * 1, String(q), q == 'Q:default', q < 'R');"),
        "Q:default Q:string NaN Q:string true true"
    );
    assert_eq!(
        one("class Qn { [Symbol.toPrimitive](h) { return h === 'number' ? 7 : 'S'; } } var qn = new Qn(); console.log(qn * 2, qn + '', qn == 7, +qn, `${qn}`, qn > 6);"),
        "14 S false 7 S true"
    );
    assert_eq!(
        one("var bad = { [Symbol.toPrimitive]: 5 }; try { bad + 1 } catch (e) { console.log(e.constructor.name) }"),
        "TypeError"
    );
}

// Cyclic arrays/objects in native walks (Array.prototype.join and its callers, inspect).
// A reference back to an array still being joined is the empty string (ES Array.prototype.join);
// inspect prints `[Circular *1]` and marks the target `<ref *1>`, as Node does.

#[test]
fn cyclic_array_join_is_empty_at_the_cycle() {
    assert_eq!(
        one(
            r#"var a=[1,2]; a.push(a); console.log(JSON.stringify([String(a), a.join("-"), ""+a, `${a}`, String(Number(a))]))"#
        ),
        r#"["1,2,","1-2-","1,2,","1,2,","NaN"]"#
    );
}

#[test]
fn cyclic_array_nested_and_mutual_join() {
    assert_eq!(
        one(
            r#"var p=[1],q=[p]; p.push(q); var x=[1,[2]]; x[1].push(x); console.log(JSON.stringify([String(p),String(q),String(x),[x].toString()]))"#
        ),
        r#"["1,","1,","1,2,","1,2,"]"#
    );
}

#[test]
fn cyclic_array_join_with_separators() {
    assert_eq!(
        one(
            r#"var m=[1,2,3]; m.push(m); console.log(JSON.stringify(m.join(", ") + " ; " + m.join("")))"#
        ),
        r#""1, 2, 3,  ; 123""#
    );
}

#[test]
fn cyclic_array_json_and_flat_errors() {
    assert_eq!(
        one(r#"var a=[1]; a.push(a); try { JSON.stringify(a) } catch (e) { console.log(e.name, e.message.split("\n")[0]) } try { a.flat(Infinity) } catch (e) { console.log(e.name, e.message) }"#),
        "TypeError Converting circular structure to JSON|RangeError Maximum call stack size exceeded"
    );
}

#[test]
fn cyclic_array_structured_clone_keeps_the_cycle() {
    assert_eq!(
        one(r#"var a=[1,2]; a.push(a); var c=structuredClone(a); console.log(c.length, c[2]===c)"#),
        "3 true"
    );
}

#[test]
fn cyclic_values_inspect_as_circular() {
    assert_eq!(
        one(r#"var a=[1,2]; a.push(a); console.log(a); var o={n:1}; o.self=o; console.log(o)"#),
        "<ref *1> [ 1, 2, [Circular *1] ]|<ref *1> { n: 1, self: [Circular *1] }"
    );
}
