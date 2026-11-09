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

/// The operand corner values of plan TJ150 section 7 (PR2), as source literals.
const CORNERS: &str = r#"[0, -0, 1, -1, 0.5, -1.5, 2147483647, 2147483648, -2147483648, -2147483649, 4294967295, 4294967296, 4294967297, 1e19, -1e19, 9007199254740993, NaN, Infinity, -Infinity, "5", "abc", "", true, false, null, undefined, 3n, 31, 32, 33, -1, 1e10]"#;

#[test]
fn pr2_binary_operators_over_the_corner_table() {
    const OPS: &[&str] = &[
        "+", "-", "*", "/", "%", "**", "&", "|", "^", "<<", ">>", ">>>", "<", "<=", ">", ">=",
        "==", "!=", "===", "!==",
    ];
    for op in OPS {
        let src = format!(
            r#"
            var T = {CORNERS};
            function f(a, b) {{ return a {op} b }}
            function show(r) {{ return typeof r + ":" + ("" + r) + (r === 0 && 1 / r < 0 ? "(-0)" : "") }}
            for (var i = 0; i < T.length; i++) {{
                for (var j = 0; j < T.length; j++) {{
                    try {{ console.log(show(f(T[i], T[j]))) }} catch (e) {{ console.log("ERR " + String(e)) }}
                }}
            }}
            "#
        );
        same(&src, 2);
    }
}

#[test]
fn pr2_int32_conversions_and_shifts() {
    let out = same(
        r#"
        function u(x) { return x >>> 0 }
        function o(x) { return x | 0 }
        function sh(x, y) { return x << y }
        function md(x, y) { return x % y }
        console.log(String(u(-1)), String(o(2147483648)), String(o(4294967297)), String(sh(1, 31)), String(sh(1, 32)), String(sh(1, 33)), String(sh(1, -1)));
        console.log(String(1 / md(-4, 2)), String(md(-7, 2)), String(md(7, -7)));
        console.log(String(u(-1)), String(o(2147483648)), String(o(4294967297)), String(sh(1, 31)), String(sh(1, 32)), String(sh(1, 33)), String(sh(1, -1)));
        "#,
        4,
    );
    assert_eq!(
        out,
        lines(&[
            "4294967295 -2147483648 1 -2147483648 1 2 -2147483648",
            "-Infinity -1 0",
            "4294967295 -2147483648 1 -2147483648 1 2 -2147483648",
        ])
    );
}

#[test]
fn pr2_string_append_aliasing_and_literal_sharing() {
    let out = same(
        r#"
        function app(p) { p += "x"; return p }
        function lit() { var s = "ab"; s += "c"; return s }
        function alias() { var s = "q"; var t = s; s += "w"; return t + "|" + s }
        var acc = "";
        for (var i = 0; i < 1000; i++) { acc = app(acc) }
        console.log(String(acc.length), lit(), lit(), alias(), alias());
        var t0 = "ab";
        var u0 = app(t0);
        console.log(t0, u0, lit(), u0);
        "#,
        3,
    );
    assert_eq!(out, lines(&["1000 abc abc q|qw q|qw", "ab abx abc abx"]));
}

#[test]
fn pr2_const_writes_throw_after_the_right_hand_side() {
    let out = same(
        r#"
        var g = 0;
        function k1() { const c = 1; c = (g = g + 1); return 0 }
        function k2() { const c = 1; c += (g = g + 10); return 0 }
        function k3() { const c = 1; c++; return 0 }
        function k4() { const c = 1; return --c }
        var fs = [k1, k2, k3, k4];
        for (var r = 0; r < 2; r++) {
            for (var i = 0; i < fs.length; i++) {
                try { fs[i]() } catch (e) { console.log(String(e.message) + " " + g) }
            }
        }
        "#,
        4,
    );
    assert_eq!(
        out,
        lines(&[
            "Assignment to constant variable. 1",
            "Assignment to constant variable. 11",
            "Assignment to constant variable. 11",
            "Assignment to constant variable. 11",
            "Assignment to constant variable. 12",
            "Assignment to constant variable. 22",
            "Assignment to constant variable. 22",
            "Assignment to constant variable. 22",
        ])
    );
}

#[test]
fn pr2_free_const_write_throws() {
    let out = same(
        r#"
        const K = 1;
        function wk() { K = 2; return 0 }
        try { wk() } catch (e) { console.log(String(e.message)) }
        try { wk() } catch (e) { console.log(String(e.message)) }
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
fn pr2_typeof_including_undeclared_names() {
    let out = same(
        r#"
        function ty(a) { var u; return typeof a + "," + typeof u + "," + typeof nowhere + "," + typeof String + "," + typeof null + "," + typeof 3n + "," + typeof "s" }
        console.log(ty(5) + " / " + ty("x"));
        console.log(ty(5) + " / " + ty("x"));
        "#,
        1,
    );
    let line = "number,undefined,undefined,function,object,bigint,string / string,undefined,undefined,function,object,bigint,string";
    // `typeof "s"` is a string literal: the same shape for both calls
    assert_eq!(out, lines(&[line, line]));
}

#[test]
fn pr2_conditional_and_logical_operators() {
    let out = same(
        r#"
        function cf(v) { return v ? "t" : "f" }
        function and_(a, b) { return a && b }
        function or_(a, b) { return a || b }
        function nc(a, b) { return a ?? b }
        for (var i = 0; i < 2; i++) {
            console.log(cf(0) + cf(-0) + cf(NaN) + cf("") + cf(null) + cf(undefined) + cf(0n) + cf(1n) + cf("0") + cf(false));
            console.log(String(and_(0, "x")) + "|" + String(and_("", 1)) + "|" + String(or_(0, "y")) + "|" + String(or_(2, "y")) + "|" + String(nc(null, "z")) + "|" + String(nc(0, "z")));
        }
        "#,
        4,
    );
    assert_eq!(
        out,
        lines(&["fffffffttf", "0||y|2|z|0", "fffffffttf", "0||y|2|z|0"])
    );
}

#[test]
fn pr2_unary_update_and_template_operators() {
    let out = same(
        r#"
        function un(x) { return -x }
        function pl(x) { return +x }
        function ti(x) { return ~x }
        function bg(x) { return !x }
        function vd(x) { return void x }
        function inc(x) { x++; return x }
        function pre(x) { return ++x }
        function dec(x) { return x-- }
        function tp(a, b) { return `a${a}b${b}c` }
        for (var i = 0; i < 2; i++) {
            console.log(String(un("3")), String(pl("12")), String(ti(5)), String(bg("")), String(vd(1)), String(un(3n)), String(ti(3n)));
            console.log(String(inc("5")), String(inc(undefined)), String(dec("5")), String(pre(3n)), String(dec(10n)));
            console.log(tp(1, null), tp(1n, undefined), tp(-0, 2.5));
            try { pl(3n) } catch (e) { console.log(String(e.message)) }
        }
        "#,
        9,
    );
    assert_eq!(
        out,
        lines(&[
            "-3 12 -6 true undefined -3 -4",
            "6 NaN 5 4 10",
            "a1bnullc a1bundefinedc a0b2.5c",
            "Cannot convert a BigInt value to a number",
            "-3 12 -6 true undefined -3 -4",
            "6 NaN 5 4 10",
            "a1bnullc a1bundefinedc a0b2.5c",
            "Cannot convert a BigInt value to a number",
        ])
    );
}

#[test]
fn pr2_compound_and_logical_assignment_on_slots_and_free_names() {
    let out = same(
        r#"
        var gx = 3;
        function ops(a) {
            var x = a;
            x += 2; x -= 1; x *= 3; x /= 2; x %= 5; x **= 2; x <<= 1; x >>= 1; x >>>= 0; x |= 8; x &= 15; x ^= 3;
            var y = null; y ??= 4; y ||= 9; y &&= 7;
            return x + "," + y
        }
        function fops() { gx += 2; gx *= 2; gx ??= 1; gx ||= 0; gx &&= gx - 1; gx **= 2; return gx }
        function nope() { missingVar += 1; return 0 }
        console.log(ops(1), ops(-2.5), fops());
        console.log(ops(1), ops(-2.5), fops());
        try { nope() } catch (e) { console.log(String(e.message)) }
        "#,
        3,
    );
    assert_eq!(out.len(), 3);
    assert_eq!(out[2], "missingVar is not defined");
}

#[test]
fn pr2_evaluation_order_of_operands_and_free_writes() {
    let out = same(
        r#"
        var seq = "";
        function ev() { return (seq += "L", 1) + (seq += "R", 2) }
        console.log(String(ev()), seq);
        console.log(String(ev()), seq);
        "#,
        1,
    );
    assert_eq!(out, lines(&["3 LR", "3 LRLR"]));
}

/// Member reads are C2 PR4: the function is analysed but stays on the AST at codegen, with the same output.
#[test]
fn pr2_member_read_stays_on_the_ast_with_the_same_output() {
    same(
        r#"
        function ml(a) { return a.length + 1 }
        console.log(String(ml("abc")), String(ml("")));
        console.log(String(ml("abc")), String(ml("")));
        "#,
        0,
    );
}
