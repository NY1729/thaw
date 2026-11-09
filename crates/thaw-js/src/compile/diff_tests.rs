//! Differential tests of the C2 PR1 subset (plan TJ150 section 7). Each program runs under `off` (the
//! reference) and under `on` and `check` (with `compile_after = 1`); output and ok-ness must match, and the
//! compiled-function count must reach the minimum, so a program cannot pass by never compiling.

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
    assert!(
        !want.is_empty() || !ok_want,
        "reference printed nothing: {src}"
    );
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

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn pr1_literals_of_each_type() {
    let out = same(
        r#"
        function n() { return 42 }
        function s() { return "xé" }
        function b() { return true }
        function z() { return null }
        function u() { }
        function big() { return 12345678901234567890n }
        console.log(String(n()), s(), String(b()), String(z()), String(u()), String(big()));
        console.log(String(n()), s(), String(b()), String(z()), String(u()), String(big()));
        "#,
        6,
    );
    let line = "42 xé true null undefined 12345678901234567890";
    assert_eq!(out, lines(&[line, line]));
}

#[test]
fn pr1_params_fewer_and_more_than_args() {
    let out = same(
        r#"
        function g(a, b) { return b }
        console.log(String(g(1)), String(g(1, 2, 3)), String(g(1, 2)));
        console.log(String(g(1)), String(g(1, 2, 3)), String(g(1, 2)));
        "#,
        1,
    );
    assert_eq!(out, lines(&["undefined 2 2", "undefined 2 2"]));
}

#[test]
fn pr1_duplicate_parameter_takes_the_last_argument() {
    let out = same(
        r#"
        function d(a, a) { return a }
        console.log(String(d(1, 2)), String(d(3)));
        console.log(String(d(1, 2)), String(d(3)));
        "#,
        1,
    );
    assert_eq!(out, lines(&["2 undefined", "2 undefined"]));
}

#[test]
fn pr1_var_hoisting_and_parameter_shadowing() {
    let out = same(
        r#"
        function h() { var r = q; var q = 1; return r }
        function hv(a) { var a; return a }
        function hw(a) { var a = 9; return a }
        function vb() { { var z = 3 } return z }
        console.log(String(h()), String(hv(7)), String(hw(7)), String(vb()));
        console.log(String(h()), String(hv(7)), String(hw(7)), String(vb()));
        "#,
        4,
    );
    assert_eq!(out, lines(&["undefined 7 9 3", "undefined 7 9 3"]));
}

#[test]
fn pr1_block_let_const_shadowing() {
    let out = same(
        r#"
        var b = 'g';
        function sh() { let a = 1; { let a = 2; { const a = 3; } } return a }
        function sh3() { { let b = 5; } { let b = 6; } return b }
        function sh4() { let a = 1; { a = 5 } return a }
        function sh5() { { let a = 9; a = 10; return a } }
        function sh6() { const c = 1; return c }
        console.log(String(sh()), String(sh3()), String(sh4()), String(sh5()), String(sh6()));
        console.log(String(sh()), String(sh3()), String(sh4()), String(sh5()), String(sh6()));
        "#,
        5,
    );
    assert_eq!(out, lines(&["1 g 5 10 1", "1 g 5 10 1"]));
}

#[test]
fn pr1_free_read_of_a_global_and_of_an_outer_function_variable() {
    let out = same(
        r#"
        var gv = 'global';
        function rg() { return gv }
        function outer() { var v = 'outer'; function leaf() { return v } return leaf() }
        console.log(String(rg()), outer(), String(rg()), outer());
        "#,
        2,
    );
    assert_eq!(out, lines(&["global outer global outer"]));
}

#[test]
fn pr1_free_write_creates_an_implicit_global() {
    let out = same(
        r#"
        function w() { gg = 4; return gg }
        console.log(String(w()), String(gg));
        console.log(String(w()), String(gg));
        "#,
        1,
    );
    assert_eq!(out, lines(&["4 4", "4 4"]));
}

#[test]
fn pr1_this_sloppy_strict_method_and_arrow() {
    let out = same(
        r#"
        function th() { return this }
        function ts() { 'use strict'; return this }
        var ar = () => this;
        var o = {};
        var r1 = th();
        var r2 = th.call(o);
        console.log(String(r1 === globalThis), String(r2 === o), String(ts() === undefined), String(ar() === this));
        console.log(String(th() === globalThis), String(th.call(o) === o), String(ts.call(o) === o));
        "#,
        3,
    );
    assert_eq!(out, lines(&["true true true true", "true true true"]));
}

#[test]
fn pr1_return_without_value_and_fallthrough() {
    let out = same(
        r#"
        function nv() { return }
        function ft() { 1 }
        function em() { ; }
        console.log(String(nv()), String(ft()), String(em()));
        console.log(String(nv()), String(ft()), String(em()));
        "#,
        3,
    );
    assert_eq!(
        out,
        lines(&[
            "undefined undefined undefined",
            "undefined undefined undefined"
        ])
    );
}

#[test]
fn pr1_const_assignment_throws_the_interpreter_message() {
    let out = same(
        r#"
        function ca() { const c = 1; c = 2; return c }
        for (let i = 0; i < 2; i++) {
            try { ca() } catch (e) { console.log(String(e.message)) }
        }
        "#,
        1,
    );
    assert_eq!(
        out,
        lines(&[
            "Assignment to constant variable.",
            "Assignment to constant variable."
        ])
    );
}

#[test]
fn pr1_errors_thrown_through_compiled_frames() {
    let out = same(
        r#"
        function thr() { return missingName }
        function pass() { return thr() }
        try { thr() } catch (e) { console.log(String(e.message)) }
        try { pass() } catch (e) { console.log(String(e.message)) }
        "#,
        1,
    );
    assert_eq!(
        out,
        lines(&["missingName is not defined", "missingName is not defined"])
    );
}

#[test]
fn pr1_sequence_paren_and_arrow_expression_body() {
    let out = same(
        r#"
        function sq() { return (1, 2, "x") }
        function pa() { return (7) }
        var ar2 = (a, b) => b;
        console.log(String(sq()), String(pa()), String(ar2(1, 2)), String(ar2(3)));
        console.log(String(sq()), String(pa()), String(ar2(1, 2)), String(ar2(3)));
        "#,
        3,
    );
    assert_eq!(out, lines(&["x 7 2 undefined", "x 7 2 undefined"]));
}

#[test]
fn pr1_string_literal_is_fresh_per_call() {
    let out = same(
        r#"
        function lit() { return "ab" }
        console.log(lit(), lit());
        console.log(lit(), lit());
        "#,
        1,
    );
    assert_eq!(out, lines(&["ab ab", "ab ab"]));
}

/// A function that calls something is outside the C2 analyzer scope: it must stay on the AST.
#[test]
fn pr1_function_with_a_call_stays_on_the_ast() {
    let it = run_mode(
        r#"
        function c1() { return String(1) }
        console.log(c1(), c1());
        "#,
        CompileMode::On,
    )
    .2;
    assert_eq!(it.cstats.compiled(), 0);
    assert!(it.cstats.rejected(FallbackReason::NotYet) >= 1);
}

/// `new` of a leaf function returns the constructed object, whatever the compiled body would return.
#[test]
fn pr1_construct_of_a_leaf_function_is_correct() {
    let out = same(
        r#"
        function F(a) { return a }
        var o = new F(1);
        var p = new F(2);
        console.log(String(o instanceof F), String(p instanceof F), String(F(3)));
        console.log(String(o instanceof F), String(p instanceof F), String(F(3)));
        "#,
        1,
    );
    assert_eq!(out, lines(&["true true 3", "true true 3"]));
}

/// A function created inside `with` reads the dynamic object record: it must not compile.
#[test]
fn pr1_function_created_inside_with_is_not_compiled() {
    let (out, ok, it) = run_mode(
        r#"
        var o = { q: 5 };
        with (o) { var wf = function (x) { return x } }
        console.log(String(wf(3)), String(wf(4)));
        console.log(String(wf(3)), String(wf(4)));
        "#,
        CompileMode::On,
    );
    assert!(ok);
    assert_eq!(out, lines(&["3 4", "3 4"]));
    assert_eq!(it.cstats.compiled(), 0);
}

#[test]
fn pr1_report_names_compiled_functions() {
    let (_, _, it) = run_mode(
        r#"
        function one() { return 1 }
        one(); one();
        "#,
        CompileMode::On,
    );
    assert_eq!(it.cstats.compiled(), 1);
    assert!(
        it.cstats.report().contains("compiled=1"),
        "{}",
        it.cstats.report()
    );
}
