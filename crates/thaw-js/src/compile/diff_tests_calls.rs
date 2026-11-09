//! Differential tests of calls and `new` (plan TJ151 section 5, PR1). Each program runs under `off` (the reference),
//! `on` and `check` (with `compile_after = 1`); output and ok-ness must match, and the compiled-function count must
//! reach the minimum, so a program cannot pass by never compiling.

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
fn calls_plain_global_function() {
    let out = same(
        r#"
        function add(a, b) { return a + b }
        function go() { return String(add(1, 2)) + " " + String(add("x", "y")) }
        console.log(go());
        console.log(go());
        "#,
        2,
    );
    assert_eq!(out, lines(&["3 xy", "3 xy"]));
}

#[test]
fn calls_method_and_computed_method() {
    let out = same(
        r#"
        const o = { n: 4, m(x) { return this.n + x }, k: "m" };
        function go() { return String(o.m(1)) + " " + String(o[o.k](2)) + " " + String(o["m"](3)) }
        console.log(go());
        console.log(go());
        "#,
        2,
    );
    assert_eq!(out, lines(&["5 6 7", "5 6 7"]));
}

#[test]
fn calls_this_sloppy_and_strict() {
    let out = same(
        r#"
        function sl() { return typeof this }
        function st() { "use strict"; return typeof this }
        function go() { return sl() + " " + st() }
        console.log(go());
        console.log(go());
        "#,
        2,
    );
    assert_eq!(out, lines(&["object undefined", "object undefined"]));
}

#[test]
fn calls_this_of_primitive_receiver_matches_ast() {
    same(
        r#"
        String.prototype.kind = function () { return typeof this };
        function go() { return "abc".kind() }
        console.log(go());
        console.log(go());
        "#,
        1,
    );
}

#[test]
fn calls_parenthesised_member_has_undefined_this() {
    let out = same(
        r#"
        const o = { m() { return this === o } };
        function go() { return String((o.m)()) + " " + String(o.m()) }
        console.log(go());
        console.log(go());
        "#,
        1,
    );
    assert_eq!(out, lines(&["false true", "false true"]));
}

#[test]
fn calls_evaluation_order_with_getter_and_arguments() {
    let out = same(
        r#"
        const log = [];
        function t(x) { log.push(x); return x }
        const o = { get g() { log.push("get"); return function (v) { return v } } };
        function go() { return o.g(t("a")) + t("b") }
        console.log(go());
        console.log(log.join(","));
        "#,
        1,
    );
    assert_eq!(out, lines(&["ab", "get,a,b"]));
}

#[test]
fn calls_not_callable_texts_and_order() {
    let out = same(
        r#"
        const five = 5;
        const o = { x: 1 };
        const log = [];
        function arg(v) { log.push(v); return v }
        function go() {
            try { five() } catch (e) { console.log(e.message) }
            try { o.nope() } catch (e) { console.log(e.message) }
            try { (1, 2)() } catch (e) { console.log(e.message) }
            try { five(arg("first")) } catch (e) { console.log(e.message + " " + log.join(",")) }
        }
        go();
        go();
        "#,
        1,
    );
    assert_eq!(out.len(), 8);
    assert_eq!(out[0], "five is not a function");
    assert_eq!(out[1], "o.nope is not a function");
    assert_eq!(out[3], "five is not a function first");
}

#[test]
fn calls_null_and_undefined_receivers() {
    let out = same(
        r#"
        const n = null;
        let u;
        function go() {
            try { n.x() } catch (e) { console.log(e.message) }
            try { u.m() } catch (e) { console.log(e.message) }
        }
        go();
        go();
        "#,
        1,
    );
    assert_eq!(out.len(), 4);
    assert_eq!(out[0], out[2]);
    assert_eq!(out[1], out[3]);
}

#[test]
fn calls_natives_and_bound_functions() {
    let out = same(
        r#"
        function add(a, b) { return a + b }
        const bound = add.bind(null, 10);
        function go(s, i) { return [Math.max(1, 2), String(s.charCodeAt(i)), s.length, bound(5), Number("7")].join(" ") }
        console.log(go("abc", 1));
        console.log(go("abc", 1));
        "#,
        1,
    );
    assert_eq!(out, lines(&["2 98 3 15 7", "2 98 3 15 7"]));
}

#[test]
fn calls_arity_fewer_more_and_duplicate_params() {
    let out = same(
        r#"
        function f(a, b, c) { return String(a) + "|" + String(b) + "|" + String(c) }
        function d(a, a) { return a }
        function go() { return f(1) + " " + f(1, 2, 3, 4) + " " + String(d(7, 8)) }
        console.log(go());
        console.log(go());
        "#,
        2,
    );
    assert_eq!(
        out,
        lines(&[
            "1|undefined|undefined 1|2|3 8",
            "1|undefined|undefined 1|2|3 8"
        ])
    );
}

#[test]
fn calls_new_constructor_return_forms() {
    let out = same(
        r#"
        function P(x) { this.v = x }
        function Q(x) { this.v = x; return 5 }
        function R(x) { this.v = x; return { w: x * 2 } }
        function go() {
            const p = new P(3);
            const q = new Q(4);
            const r = new R(5);
            const bare = new P;
            return [String(p.v), String(q.v), String(r.w), String(r.v), String(bare.v), String(p instanceof P)].join(" ");
        }
        console.log(go());
        console.log(go());
        "#,
        2,
    );
    assert_eq!(
        out,
        lines(&[
            "3 4 10 undefined undefined true",
            "3 4 10 undefined undefined true"
        ])
    );
}

#[test]
fn calls_new_not_a_constructor_and_builtins() {
    let out = same(
        r#"
        const arrow = () => 1;
        const obj = {};
        function go() {
            try { new arrow() } catch (e) { console.log(e.message) }
            try { new obj() } catch (e) { console.log(e.message) }
            const a = new Array(3);
            const err = new Error("boom");
            return a.length + " " + err.message;
        }
        console.log(go());
        console.log(go());
        "#,
        1,
    );
    assert_eq!(out.len(), 6);
    assert_eq!(out[0], out[3]);
    assert_eq!(out[1], out[4]);
    assert_eq!(out[2], "3 boom");
    assert_eq!(out[5], "3 boom");
    assert_eq!(out[1], "obj is not a constructor");
}

#[test]
fn calls_throw_from_callee_caught_by_ast_caller() {
    let out = same(
        r#"
        function thrower(x) { if (x > 1) throw new Error("big " + x); return x }
        function go(x) { return thrower(x) }
        for (let i = 0; i < 3; i++) {
            try { console.log(String(go(i))) } catch (e) { console.log(e.message) }
        }
        "#,
        1,
    );
    assert_eq!(out, lines(&["0", "1", "big 2"]));
}

#[test]
fn calls_arguments_user_stays_ast_and_callers_compile() {
    let mut it = Interp::with_compile_mode(CompileMode::On);
    it.compile_after = 1;
    let src = r#"
        function usesArgs(a) { return arguments.length + a }
        function f(x) { return usesArgs(x) + 1 }
        console.log(String(f(1)));
        console.log(String(f(2)));
    "#;
    assert!(it.run_sloppy(src).is_ok());
    assert_eq!(it.out, lines(&["3", "4"]));
    assert_eq!(it.cstats.rejected(FallbackReason::Arguments), 1);
    assert_eq!(it.cstats.rejected(FallbackReason::NotYet), 0);
    assert!(it.cstats.compiled() >= 1);
}

#[test]
fn calls_unsupported_callee_shapes_stay_ast_with_their_reasons() {
    let mut it = Interp::with_compile_mode(CompileMode::On);
    it.compile_after = 1;
    let src = r#"
        function spread(a) { return Math.max(...a) }
        function opt(f) { return f?.() }
        function ev() { return eval("1 + 1") }
        function ctor() { return Function("return 1")() }
        console.log(String(spread([1, 2])));
        console.log(String(opt(() => 4)));
        console.log(String(ev()));
        console.log(String(ctor()));
    "#;
    assert!(it.run_sloppy(src).is_ok());
    assert_eq!(it.out, lines(&["2", "4", "2", "1"]));
    assert_eq!(it.cstats.rejected(FallbackReason::Spread), 1);
    assert_eq!(it.cstats.rejected(FallbackReason::OptionalChain), 1);
    assert_eq!(it.cstats.rejected(FallbackReason::DirectEval), 1);
    assert_eq!(it.cstats.rejected(FallbackReason::FunctionCtor), 1);
    assert_eq!(it.cstats.rejected(FallbackReason::NotYet), 0);
}

#[test]
fn calls_tier_a_read_before_declaration_stays_rejected() {
    let mut it = Interp::with_compile_mode(CompileMode::On);
    it.compile_after = 1;
    let src = r#"
        function g(v) { return v }
        function f() { return g(x); let x = 1; }
        try { f() } catch (e) { console.log(e.message) }
    "#;
    assert!(it.run_sloppy(src).is_ok());
    assert_eq!(it.cstats.rejected(FallbackReason::ReadBeforeDecl), 1);
}

#[test]
fn calls_compiled_caller_reads_script_let_in_tdz_like_off() {
    let out = same(
        r#"
        function h() { return late + 1 }
        function caller() { return h() }
        try { console.log(String(caller())) } catch (e) { console.log(e.message) }
        let late = 5;
        console.log(String(caller()));
        "#,
        1,
    );
    assert_eq!(out.len(), 2);
    assert_eq!(out[1], "6");
}
