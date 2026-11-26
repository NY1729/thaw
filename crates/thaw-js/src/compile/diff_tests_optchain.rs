//! Compiled optional chains: programs run under `off` (the reference), `on` and `check` with
//! `compile_after = 1`; output must match and the functions must really be compiled.

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
fn optional_chains() {
    let out = same(
        r#"
        function m(o) { return o?.a }
        function k(o, key) { return o?.[key] }
        function c(f) { return f?.(1) }
        function deep(o) { return o?.a.b.c }
        function meth(o) { return o?.m() }
        function meth2(o) { return o.m?.() }
        function meth3(o) { return o?.m?.(2) }
        function th(o) { return o?.who() }
        function side(o) { const log = []; const t = (n) => { log.push(n); return n }; o?.[t('k')]?.(t('arg')); return log.join() }
        function chain3(o) { return o?.a?.b?.c }
        function bad(o) { return o.nothing?.() }
        function notfn(o) { return o?.a() }
        function getter(o) { return o?.g }
        console.log(m(null), m(undefined), m({ a: 1 }), m(0), k(null, 'a'), k({ a: 2 }, 'a'), c(undefined), c(null), c((x) => x + 1));
        console.log(deep(null), deep({ a: { b: { c: 3 } } }), meth(null), meth({ m() { return 'm' } }), meth2({}), meth2({ m() { return 'm2' } }), meth3({}), meth3({ m(x) { return x } }));
        console.log(th({ who() { return this.n }, n: 'me' }), th(null), side(null), side({ k: (x) => x }), side({}), chain3({ a: null }), chain3({ a: { b: { c: 7 } } }), bad({}));
        for (const t of [() => deep({ a: null }), () => notfn({ a: 1 }), () => m.x?.y.z, () => getter({ get g() { throw new RangeError('g') } }), () => meth2({ m: 5 })]) {
          try { console.log('ok', t()) } catch (e) { console.log(e.name + ': ' + e.message) }
        }
        "#,
        12,
    );
    assert!(
        out[0].starts_with("undefined undefined 1 undefined undefined 2 undefined undefined 2"),
        "{}",
        out[0]
    );
}
