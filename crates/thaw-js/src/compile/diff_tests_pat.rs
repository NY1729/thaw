//! Compiled binding patterns (declarations and parameters): programs run under `off` (the reference), `on`
//! and `check` with `compile_after = 1`; output must match and the functions must really be compiled.

use super::*;
use crate::Interp;

fn run_mode(src: &str, mode: CompileMode) -> (Vec<String>, bool, Interp) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it)
}

fn same(src: &str, min_compiled: u64) -> Vec<String> {
    let (want, ok_want, _) = run_mode(src, CompileMode::Off);
    for mode in [CompileMode::On, CompileMode::Check] {
        let (got, ok, it) = run_mode(src, mode);
        assert_eq!(ok, ok_want, "{mode:?}: {src}");
        assert_eq!(got, want, "{mode:?}: {src}");
        assert!(
            it.cstats.compiled() >= min_compiled,
            "{mode:?}: compiled {} < {min_compiled}: {src}",
            it.cstats.compiled()
        );
    }
    want
}

#[test]
fn declaration_patterns() {
    let out = same(
        r#"
        function f(o) {
          const { a, b: bb, c = 5, ...rest } = o;
          let [x, , y = 9, ...z] = [1, 2, undefined, 4, 5];
          var { q: { r } } = { q: { r: 'deep' } };
          return [a, bb, c, JSON.stringify(rest), x, y, JSON.stringify(z), r].join('|');
        }
        console.log(f({ a: 1, b: 2, d: 4, e: 5 }));
        console.log(f({ a: 1, b: 2, c: 3 }));
        function g(s) { const [h, i] = s; const { length } = s; return h + i + length }
        console.log(g('xyz'));
        function k(k1) { const key = 'zz'; const { [key]: v = 'dflt', [k1]: w } = { zz: undefined, ab: 3 }; return v + w }
        console.log(k('ab'));
        "#,
        3,
    );
    assert_eq!(out[0], "1|2|5|{\"d\":4,\"e\":5}|1|9|[4,5]|deep");
}

#[test]
fn declaration_pattern_errors_match() {
    same(
        r#"
        function nul(o) { const { a } = o; return a }
        function nit(o) { const [a] = o; return a }
        function cnst(o) { const { a } = o; a = 1; return a }
        function thr(o) { let { a = (() => { throw new RangeError('dflt') })() } = o; return a }
        function idx(i) { var [a, b] = { [Symbol.iterator]() { return { next() { console.log('next'); return { done: false, value: 1 } }, return() { console.log('ret'); return {} } } } }; return a + b }
        for (const t of [() => nul(null), () => nul(undefined), () => nit(5), () => nit(null), () => cnst({ a: 2 }), () => thr({}), () => thr({ a: 1 }), () => idx()]) {
          try { console.log('ok', t()) } catch (e) { console.log(e.name + ': ' + e.message) }
        }
        "#,
        4,
    );
}

#[test]
fn parameter_patterns() {
    let out = same(
        r#"
        function f({ a, b = 2 }, [c, d], e) { return a + b + c + d + e }
        console.log(f({ a: 1 }, [3, 4], 5));
        function g({ a }, ...r) { return a + r.length }
        console.log(g({ a: 1 }, 1, 2));
        const h = ({ x, y }) => x * y;
        console.log(h({ x: 3, y: 4 }));
        function cap({ a, b }) { return () => a + b }
        console.log(cap({ a: 1, b: 2 })());
        function dup({ a }, b = a) { return a + b }
        console.log(dup({ a: 4 }));
        function arg({ a }) { return arguments.length + a }
        console.log(arg({ a: 1 }, 9));
        function tdz({ a = b }, b) { return a }
        try { console.log(tdz({}, 1)) } catch (e) { console.log(e.name) }
        function vv({ a = x }) { var x = 3; return a }
        try { console.log(vv({})) } catch (e) { console.log(e.name) }
        function bad({ a }) { return a }
        for (const v of [undefined, null, 7]) { try { console.log(bad(v)) } catch (e) { console.log(e.name + ': ' + e.message) } }
        "#,
        3,
    );
    assert_eq!(out[0], "15");
}

#[test]
fn pattern_names_functions() {
    let out = same(
        r#"
        function f() { const { a = function () {}, b = () => 1 } = {}; const [c = class {}] = []; return a.name + b.name + c.name }
        console.log(f());
        "#,
        0,
    );
    assert_eq!(out[0], "abc");
}

#[test]
fn default_parameters() {
    let out = same(
        r#"
        function f(a, b = a + 1, c = b * 2) { return [a, b, c].join(',') }
        console.log(f(1), f(1, 5), f(1, undefined, 7), f(1, null), f(1, 0));
        function g(x = (() => 3)()) { return x }
        console.log(g(), g(1));
        function side(n) { console.log('side ' + n); return n }
        function h(a = side(1), b = side(2)) { return a + b }
        console.log(h(), h(9), h(9, 8));
        const arrow = (a, b = () => a) => b();
        console.log(arrow(4));
        function cap(a = 1) { return () => a }
        console.log(cap()(), cap(5)());
        function thr(a = (() => { throw new RangeError('d') })()) { return a }
        for (const t of [() => thr(), () => thr(1)]) { try { console.log(t()) } catch (e) { console.log(e.name) } }
        function later(a = b, b = 2) { return a }
        try { console.log(later()) } catch (e) { console.log(e.name) }
        function self(a = self) { return typeof a }
        console.log(self());
        function nm(a = function () {}, b = () => 1) { return a.name + b.name }
        console.log(nm());
        function usethis(a = this) { return a === undefined }
        console.log(usethis());
        function len(a, b = 1, c) {}
        console.log(len.length);
        "#,
        3,
    );
    assert!(out[0].starts_with("1,2,4 1,5,10 1,2,7 "), "{}", out[0]);
}

#[test]
fn rest_parameters() {
    let out = same(
        r#"
        function f(a, ...r) { return a + ':' + r.length + ':' + r.join('-') }
        console.log(f(), f(1), f(1, 2), f(1, 2, 3, 4));
        function g(...[x, y = 5]) { return x + y }
        console.log(g(1), g(1, 2));
        const h = (...r) => r.length;
        console.log(h(), h(1, 2, 3));
        function cap(...r) { return () => r }
        console.log(cap(1, 2)().length);
        function both(a = 1, { b } = { b: 2 }, ...r) { return a + b + r.length }
        console.log(both(), both(5, { b: 1 }, 1, 1));
        function arr(...r) { return Array.isArray(r) + ':' + arguments.length }
        console.log(arr(1, 2));
        console.log(f.length, g.length);
        "#,
        3,
    );
    assert_eq!(out[0], "undefined:0: 1:0: 1:1:2 1:3:2-3-4");
}

#[test]
fn spread_in_calls_arrays_and_objects() {
    let out = same(
        r#"
        function sum() { let s = 0; for (let i = 0; i < arguments.length; i++) s += arguments[i]; return s }
        function f(a, b) { return sum(...a, 10, ...b) }
        console.log(f([1, 2], [3]), f('ab'.split(''), []), f([], [5, 6]));
        function g(a) { return [0, ...a, , ...'xy'].length }
        console.log(g([1, 2, 3]), g(new Set([1])), g(''));
        function h(o) { return JSON.stringify({ z: 0, ...o, y: 1, ...{ x: 2 } }) }
        console.log(h({ a: 1, y: 9 }), h(null), h(undefined), h('ab'), h(5));
        function m(o, args) { return o.f(...args) }
        console.log(m({ f(a, b) { return this.k + a + b }, k: 100 }, [1, 2]));
        function nw(args) { return new Date(...args).getTime() }
        console.log(nw([0]), nw([1970, 0, 2]));
        function* gen() { yield 1; yield 2 }
        function it(x) { return Math.max(...x) }
        console.log(it(gen()), it(new Set([4, 9, 2])), it([]));
        function bad(x) { return sum(...x) }
        for (const v of [5, null, undefined, {}]) { try { console.log(bad(v)) } catch (e) { console.log(e.name + ': ' + e.message) } }
        function badarr(x) { return [...x] }
        for (const v of [5, null, {}]) { try { console.log(badarr(v)) } catch (e) { console.log(e.name + ': ' + e.message) } }
        function order() { const log = []; const t = (n) => (log.push(n), [n]); const r = [...t(1), ...t(2)]; sum(...t(3), ...t(4)); return log.join() + r.join() }
        console.log(order());
        const thrower = { [Symbol.iterator]() { throw new RangeError('it') } };
        function th() { return sum(...thrower) }
        try { th() } catch (e) { console.log(e.name) }
        function getter(o) { return { ...o } }
        let calls = 0;
        console.log(JSON.stringify(getter({ get a() { calls++; return 1 }, b: 2 })), calls);
        "#,
        6,
    );
    assert_eq!(out[0], "16 0ab10 21");
}
