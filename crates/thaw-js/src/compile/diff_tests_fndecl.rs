//! Compiled function declarations in a body and methods/accessors in object literals: programs run under
//! `off` (the reference), `on` and `check` with `compile_after = 1`.

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
fn inner_function_declarations() {
    let out = same(
        r#"
        function a(n) { return twice(n) + 1; function twice(x) { return x * 2 } }
        function fact(n) { function go(k) { return k <= 1 ? 1 : k * go(k - 1) } return go(n) }
        function mutual(n) { function ev(k) { return k == 0 ? 'even' : od(k - 1) } function od(k) { return k == 0 ? 'odd' : ev(k - 1) } return ev(n) }
        function over(x) { function x() { return 'fn' } return typeof x }
        function overvar() { var h = 1; function h() {} return typeof h }
        function dup() { function d() { return 1 } function d() { return 2 } return d() }
        function arrow(n) { function inner() { return n } return [1, 2].map(() => inner())[1] }
        function capture() { let c = 0; function inc() { c++ } inc(); inc(); return c }
        function name() { function foo() {} return foo.name + foo.length }
        function thisv() { function t() { return typeof this } return t() }
        function gen() { function* g() { yield 1; yield 2 } return [...g()].join() }
        function asyncf() { async function f() { return 3 } return typeof f().then }
        function closure(k) { function mk() { return () => k } return mk()() }
        console.log(a(3), fact(5), mutual(5), over(1), overvar(), dup(), arrow(7), capture(), name(), thisv(), gen(), asyncf(), closure(9));
        function unused() { function never() {} return 1 }
        console.log(unused(), unused());
        "#,
        8,
    );
    assert!(
        out[0].starts_with("7 120 odd function number 2 7 2 foo0"),
        "{}",
        out[0]
    );
}

#[test]
fn object_methods_and_accessors() {
    let out = same(
        r#"
        function a(v) { return { v, get x() { return this.v + 1 }, set x(n) { this.v = n }, m() { return this.v * 2 } } }
        function b(k) { return { [k]() { return k }, get [k + 'g']() { return 1 } } }
        function c() { const o = { f() { return 1 }, g: function () {}, h: () => 1 }; return o.f.name + o.g.name + o.h.name }
        function d() { return { a() {}, a: 5 } }
        function e1() { return { async am() { return 1 }, *gm() { yield 1 } } }
        const o = a(5);
        console.log(o.x, o.m(), (o.x = 9, o.v), Object.keys(o).join());
        console.log(b('kk').kk(), b('kk').kkg, c(), typeof d().a, typeof e1().am().then, [...e1().gm()].join());
        const desc = Object.getOwnPropertyDescriptor(a(1), 'x');
        console.log(typeof desc.get, typeof desc.set, desc.enumerable, desc.get.name, a(1).m.name);
        "#,
        4,
    );
    assert!(out[0].starts_with("6 10 9 v,x,m"), "{}", out[0]);
}

#[test]
fn captured_top_level_lexicals() {
    let out = same(
        r#"
        function a(n) { let c = n; const k = 10; const add = (x) => { c += x; return c + k }; add(1); return add(2) }
        function b() { const fns = []; let i = 0; fns.push(() => i); i = 5; return fns[0]() }
        function c1() { const x = 1; const f = () => x; try { f.x = 1; return (() => { x = 2 })() } catch (e) { return e.name } }
        function d() { let { p, q = 7 } = { p: 1 }; const [r] = [3]; return [() => p, () => q, () => r].map((f) => f()).join() }
        function e1() { const early = () => later; let later = 'L'; return early() }
        function f(flag) { if (flag) { let inner = 1; return () => inner } let t = 2; return () => t }
        function g() { let x = 'outer'; { let x = 'blk'; var f1 = () => x } return f1() + x }
        function h() { return typeof early(); function early() { return typeof late } let late = 1 }
        function early2() { try { return pre() } catch (e) { return e.name } function pre() { return z } let z = 1 }
        console.log(a(1), b(), c1(), d(), e1(), f(true)(), f(false)(), g(), h(), early2());
        "#,
        6,
    );
    assert!(
        out[0].starts_with("14 5 TypeError 1,7,3 L 1 2 blkouter"),
        "{}",
        out[0]
    );
}

#[test]
fn nested_bindings_do_not_capture_same_named_outer_names() {
    let out = same(
        r#"
        function a() { const h = err => err; try { throw 1 } catch (err) { return h(err) + 1 } }
        function b() { try { throw 1 } catch (err) { return (() => err)() } }
        function c() { let x = 1; const f = function (x) { return x }; return f(5) + x }
        function d() { let x = 1; const f = function () { var x = 2; return x }; return f() + x }
        function e1() { let x = 1; const f = function (y = x) { var x = 5; return y }; return f() }
        function f2() { let x = 1; const f = () => { { let x = 7 } return x }; return f() }
        function g() { let x = 1; const f = () => { const x = 3; return () => x }; return f()() + x }
        function h() { const k = 1; const f = (k) => k; try { throw 5 } catch (k) { return f(k) } }
        console.log(a(), b(), c(), d(), e1(), f2(), g(), h());
        console.log(a(), b(), c(), d(), e1(), f2(), g(), h());
        "#,
        6,
    );
    assert_eq!(out[0], "2 1 6 3 1 1 4 5");
}
