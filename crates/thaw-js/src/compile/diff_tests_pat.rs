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
