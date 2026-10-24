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
    assert_eq!(one("class A {}"), "unsupported: class declaration");
    assert_eq!(
        one("var {a} = {a: 1}"),
        "unsupported: destructuring declaration"
    );
    assert_eq!(
        one("async function f(){}"),
        "unsupported: async/generator function"
    );
    assert_eq!(
        one("var r = /x/"),
        "unsupported: literal kind (regexp/bigint)"
    );
    assert_eq!(one("import x from 'y'"), "unsupported: ES module syntax");
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
    let exports = it.run_commonjs(bundle).ok().expect("bundle evaluates");
    let n = |v: R<Value>| v.ok().expect("call ok").as_number().expect("number");
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
