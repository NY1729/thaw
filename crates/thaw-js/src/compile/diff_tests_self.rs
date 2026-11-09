//! Differential tests of compiled functions that reference their own name (plan TJ152 section 4 PR1, and the
//! section 5 risk R1). Each program runs under `off` (the reference), `on` and `check`; output and ok-ness must
//! match, and the compiled count of `on` must reach the minimum, so a program cannot pass by never compiling.

use super::*;
use crate::Interp;

fn run_mode(src: &str, mode: CompileMode) -> (Vec<String>, bool, Interp) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it)
}

/// Runs `src` under all three modes, asserts identical output and ok-ness, and asserts the `on` run compiled at
/// least `min_compiled` functions. Returns the reference output.
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

/// Runs `src` in `on` mode only and returns the interpreter (for the rejection counters).
fn on_only(src: &str) -> Interp {
    run_mode(src, CompileMode::On).2
}

#[test]
fn self_name_shadowed_by_var_matches_off() {
    // Plan R1: the interpreter keeps the self binding across `var f` (declare_if_absent), so `typeof f` is
    // "function" under Off. The compiled path must agree.
    let out = same(
        r#"
        function f() { var f; return typeof f }
        console.log(f());
        console.log(f());
        "#,
        1,
    );
    assert_eq!(out, vec!["function".to_string(), "function".to_string()]);
}

#[test]
fn self_name_shadowed_by_var_value_matches_off() {
    let out = same(
        r#"
        function v() { var v; return v === undefined ? "undef" : "fn" }
        console.log(v());
        console.log(v());
        "#,
        1,
    );
    assert_eq!(out, vec!["fn".to_string(), "fn".to_string()]);
}

#[test]
fn self_name_shadowed_by_param_matches_off() {
    let out = same(
        r#"
        function g(g) { return typeof g }
        console.log(g(1));
        console.log(g(2));
        "#,
        1,
    );
    assert_eq!(out, vec!["number".to_string(), "number".to_string()]);
}

#[test]
fn self_name_shadowed_by_var_init_matches_off() {
    let out = same(
        r#"
        function h() { var h = 1; return typeof h }
        console.log(h());
        console.log(h());
        "#,
        1,
    );
    assert_eq!(out, vec!["number".to_string(), "number".to_string()]);
}

#[test]
fn self_name_shadowed_by_inner_fn_decl_matches_off() {
    // an inner declaration blocks (InnerFunction), so this one stays on the AST; the output must still agree
    let out = same(
        r#"
        function k() { function k() { return 2 } return typeof k() }
        console.log(k());
        console.log(k());
        "#,
        0,
    );
    assert_eq!(out, vec!["number".to_string(), "number".to_string()]);
}

#[test]
fn self_recursion_fib() {
    let out = same(
        r#"
        function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2) }
        console.log(fib(15));
        console.log(fib(15));
        "#,
        1,
    );
    assert_eq!(out, vec!["610".to_string(), "610".to_string()]);
}

#[test]
fn self_recursion_accumulator() {
    let out = same(
        r#"
        function sum(n, acc) { if (n === 0) return acc; return sum(n - 1, acc + n) }
        console.log(sum(40, 0));
        console.log(sum(40, 0));
        "#,
        1,
    );
    assert_eq!(out, vec!["820".to_string(), "820".to_string()]);
}

#[test]
fn self_recursion_depth_is_a_range_error() {
    let out = same(
        r#"
        function r(n) { return r(n + 1) }
        try { r(0) } catch (e) { console.log(e instanceof RangeError, e.message) }
        try { r(0) } catch (e) { console.log(e instanceof RangeError, e.message) }
        "#,
        1,
    );
    assert_eq!(
        out,
        vec![
            "true Maximum call stack size exceeded".to_string(),
            "true Maximum call stack size exceeded".to_string(),
        ]
    );
}

#[test]
fn self_named_function_expression_recurses() {
    let out = same(
        r#"
        var x = function g(n) { return n ? g(n - 1) + 1 : 0 };
        console.log(x(5));
        console.log(x(5));
        "#,
        1,
    );
    assert_eq!(out, vec!["5".to_string(), "5".to_string()]);
}

#[test]
fn self_name_assignment_matches_off() {
    // the interpreter writes the call-local binding only; the outer name is not changed (pinned as Off behaviour)
    same(
        r#"
        function f(n) { f = 5; return typeof f }
        console.log(f(1));
        console.log(f(2));
        console.log(typeof f);
        "#,
        1,
    );
    same(
        r#"
        function fs(n) { "use strict"; fs = 5; return typeof fs }
        console.log(fs(1));
        console.log(fs(2));
        "#,
        1,
    );
    same(
        r#"
        var x = function g() { g = 1; return typeof g };
        console.log(x());
        console.log(x());
        "#,
        1,
    );
    same(
        r#"
        var y = function gs() { "use strict"; gs = 1; return typeof gs };
        console.log(y());
        console.log(y());
        "#,
        1,
    );
}

#[test]
fn self_name_shadowed_in_block_matches_off() {
    let out = same(
        r#"
        function b(n) { { let b = n; return b } }
        console.log(b(1));
        console.log(b(2));
        "#,
        1,
    );
    assert_eq!(out, vec!["1".to_string(), "2".to_string()]);
}

#[test]
fn self_name_properties_unaffected() {
    same(
        r#"
        function nm(a, b) { return nm.name + nm.length + ":" + (nm(1, 2) ? "y" : "n") }
        console.log(nm(0, 0));
        console.log(nm(0, 0));
        console.log(nm.name, nm.length);
        "#,
        1,
    );
}

#[test]
fn self_name_call_apply_and_new() {
    same(
        r#"
        function cs(n) { return n ? cs.call(null, n - 1) + 1 : 0 }
        function ap(n) { return n ? ap.apply(null, [n - 1]) + 1 : 0 }
        console.log(cs(4), ap(4));
        console.log(cs(4), ap(4));
        function K(n) { this.n = n; this.kid = n > 0 ? new K(n - 1) : null }
        const k = new K(2);
        console.log(k instanceof K, k.kid.n, k.kid.kid.n, k.kid.kid.kid);
        console.log(new K(1) instanceof K);
        "#,
        2,
    );
}

#[test]
fn self_name_alias_after_reassignment_matches_off() {
    // the call-local slot still holds the callee after the outer name is reassigned
    same(
        r#"
        function al(n) { return n ? al(n - 1) + 1 : 0 }
        const h = al;
        al = null;
        console.log(h(3));
        console.log(h(3));
        "#,
        1,
    );
}

#[test]
fn self_name_top_level_lexical_stays_on_ast() {
    // a top-level `let` of the same name replaces the self binding in the interpreter's frame: blocked
    let out = same(
        r#"
        function q() { let q = 2; return q }
        console.log(q());
        console.log(q());
        "#,
        0,
    );
    assert_eq!(out, vec!["2".to_string(), "2".to_string()]);
    let it = on_only(
        r#"
        function q() { let q = 2; return q }
        q(); q();
        "#,
    );
    assert!(it.cstats.rejected(FallbackReason::SelfBinding) >= 1);
}

#[test]
fn self_name_captured_by_closure_stays_on_ast() {
    let out = same(
        r#"
        function cf(n) { return n ? (() => cf)() : 0 }
        console.log(typeof cf(1));
        console.log(typeof cf(1));
        "#,
        0,
    );
    assert_eq!(out, vec!["function".to_string(), "function".to_string()]);
    let it = on_only(
        r#"
        function cf(n) { return n ? (() => cf)() : 0 }
        cf(1); cf(1);
        "#,
    );
    // the arrow inside is an inner function (a higher-priority reason); the self name alone would be SelfBinding
    assert!(
        it.cstats.rejected(FallbackReason::InnerFunction)
            + it.cstats.rejected(FallbackReason::SelfBinding)
            >= 1
    );
}
