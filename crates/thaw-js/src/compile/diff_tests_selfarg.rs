//! Differential tests of compiled functions whose own name is captured by an inner closure, and of functions
//! that read `arguments`. `off` is the reference; `on` and `check` must match it and must compile.

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
        if mode == CompileMode::On {
            assert!(
                it.cstats.compiled() >= min_compiled,
                "compiled {} < {min_compiled}: {src}",
                it.cstats.compiled()
            );
        }
    }
    want
}

#[test]
fn captured_self_name() {
    let out = same(
        r#"
        var g = function fact(n) { var h = function () { return fact; }; return n <= 1 ? 1 : n * h()(n - 1); };
        console.log(g(5)); console.log(g(6));
        function cnt() { return (() => cnt === cnt)(); }
        console.log(cnt()); console.log(cnt());
        "#,
        2,
    );
    assert_eq!(out, ["120", "720", "true", "true"]);
}

#[test]
fn captured_self_name_shadows() {
    same(
        r#"
        function a(a) { return () => a; }
        console.log(typeof a(1)()); console.log(a(2)());
        function b() { var b; return () => typeof b; }
        console.log(b()()); console.log(b()());
        function c() { var c = 3; return () => c; }
        console.log(c()()); console.log(c()());
        function d() { d = 5; return () => typeof d; }
        console.log(d()()); console.log(d()());
        "#,
        1,
    );
}

#[test]
fn arguments_reads() {
    let out = same(
        r#"
        function f(a, b) { return arguments.length + ":" + arguments[0] + ":" + arguments[2]; }
        console.log(f(1)); console.log(f(1, 2, 3)); console.log(f());
        function g() { var s = 0; for (var i = 0; i < arguments.length; i++) s += arguments[i]; return s; }
        console.log(g(1, 2, 3)); console.log(g(4, 5));
        function h() { return Array.prototype.slice.call(arguments, 1).join("-"); }
        console.log(h(1, 2, 3)); console.log(h(4, 5, 6));
        function k(a) { return Math.max.apply(null, arguments); }
        console.log(k(1, 9, 3)); console.log(k(4));
        "#,
        4,
    );
    assert_eq!(out[0], "1:1:undefined");
}

#[test]
fn arguments_sloppy_and_strict_unmapped_like_interpreter() {
    same(
        r#"
        function f(a) { a = 9; return arguments[0]; }
        console.log(f(1)); console.log(f(1));
        function s(a) { "use strict"; a = 9; return arguments[0]; }
        console.log(s(1)); console.log(s(1));
        function w(a) { arguments[0] = 7; return a; }
        console.log(w(1)); console.log(w(1));
        function m(a) { arguments = 3; return arguments; }
        console.log(m(1)); console.log(m(1));
        "#,
        1,
    );
}

#[test]
fn arguments_with_inner_closures_and_this() {
    same(
        r#"
        function f() { var g = () => arguments.length; return g() + arguments[0]; }
        console.log(f(1, 2)); console.log(f(5));
        function t() { return this.x + arguments[0]; }
        var o = { x: 1, t: t };
        console.log(o.t(2)); console.log(o.t(3));
        function n() { return function () { return arguments.length; }() + arguments.length; }
        console.log(n(1, 2, 3)); console.log(n());
        function j() { return arguments; }
        console.log(Array.isArray(j(1)), j(1, 2).length); console.log(j().length);
        "#,
        1,
    );
}

#[test]
fn arguments_named_declarations_stay_correct() {
    same(
        r#"
        function a(arguments) { return arguments; }
        console.log(a(5)); console.log(a(6));
        function b() { var arguments; return typeof arguments; }
        console.log(b()); console.log(b());
        function c() { var arguments = 2; return arguments; }
        console.log(c()); console.log(c());
        "#,
        0,
    );
}

#[test]
fn arguments_throwing_and_recursion() {
    same(
        r#"
        function r(n) { return n ? arguments[0] + r(n - 1) : 0; }
        console.log(r(5)); console.log(r(6));
        function t() { if (arguments.length > 1) throw new Error("many" + arguments[1]); return 1; }
        try { t(1, 2); } catch (e) { console.log(e.message); }
        try { t(1, 2); } catch (e) { console.log(e.message); }
        console.log(t(1));
        "#,
        2,
    );
}
