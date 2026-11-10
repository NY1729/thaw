//! Differential tests of the compiled `switch` (TJ156): one discriminant evaluation, case tests in source
//! order up to the first strict match, the first `default` when nothing matches (also in the middle), fallthrough,
//! and `break` ending only the switch. Same contract as `diff_tests_ctl.rs`: `off` is the reference, `on` and
//! `check` must match it, and the compiled count must reach the minimum. Lexical declarations in a case stay refused.

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
fn switch_fallthrough_and_break_end_only_the_switch() {
    let out = same(
        r#"
        function ft(x) { var out = ''; switch (x) { case 1: out += 'a'; case 2: out += 'b'; break; default: out += 'd'; case 3: out += 'c'; } return out }
        console.log(ft(1), ft(2), ft(3), ft(9));
        console.log(ft(1), ft(2), ft(3), ft(9));
        "#,
        1,
    );
    assert_eq!(out, lines(&["ab b c dc", "ab b c dc"]));
}

#[test]
fn switch_default_first_and_in_the_middle() {
    let out = same(
        r#"
        function dfirst(x) { var out = ''; switch (x) { default: out += 'd'; case 1: out += '1'; break; case 2: out += '2' } return out }
        function dmid(x) { var o = ''; switch (x) { case 1: o += '1'; break; default: o += 'd'; case 2: o += '2'; break; case 3: o += '3' } return o }
        console.log(dfirst(1), dfirst(5), dfirst(2));
        console.log(dmid(1), dmid(2), dmid(7), dmid(3));
        "#,
        2,
    );
    assert_eq!(out, lines(&["1 d1 2", "1 2 d2 3"]));
}

#[test]
fn switch_no_match_empty_and_discriminant_once() {
    let out = same(
        r#"
        function nomatch(x) { var o = 'x'; switch (x) { case 1: o = 'one' } return o }
        function empty(x) { switch (x) {} switch (x) { default: } return 'ok' }
        var dn = 0;
        function disc() { dn++; return 2 }
        function once() { switch (disc()) { case 1: return 'a'; case 2: return 'b'; case 3: return 'c' } }
        console.log(nomatch(2), nomatch(1), empty(3));
        console.log(once(), once(), dn);
        "#,
        3,
    );
    assert_eq!(out, lines(&["x one ok", "b b 2"]));
}

#[test]
fn switch_case_tests_run_in_order_only_up_to_the_match() {
    let out = same(
        r#"
        var s = '';
        function t(v) { s += v; return v }
        function run(x) { s = ''; switch (x) { case t(1): return 'A'; case t(2): return 'B'; case t(3): return 'C'; default: s += 'D'; } return 'none' }
        console.log(run(2), s, run(9), s);
        console.log(run(2), s, run(9), s);
        "#,
        2,
    );
    assert_eq!(out, lines(&["B 12 none 123D", "B 12 none 123D"]));
}

#[test]
fn switch_strict_equality_corners() {
    let out = same(
        r#"
        function sw(x) { switch (x) { case 1: return 'num'; case '1': return 'str'; case null: return 'null'; case undefined: return 'undef'; case NaN: return 'nan'; case 0: return 'zero'; default: return 'other' } }
        console.log(sw(1), sw('1'), sw(null), sw(undefined), sw(NaN), sw(-0), sw(0), sw(true), sw(2), sw(''));
        console.log(sw(1), sw('1'), sw(null), sw(undefined), sw(NaN), sw(-0), sw(0), sw(true), sw(2), sw(''));
        "#,
        1,
    );
    assert_eq!(
        out,
        lines(&[
            "num str null undef other zero zero other other other",
            "num str null undef other zero zero other other other",
        ])
    );
}

#[test]
fn switch_object_and_bigint_discriminants() {
    let out = same(
        r#"
        var o1 = {}, o2 = {};
        function ob(x) { switch (x) { case o1: return 'o1'; case o2: return 'o2'; default: return 'none' } }
        function bi(x) { switch (x) { case 1n: return 'big'; case 'b': return 'str'; default: return 'other' } }
        console.log(ob(o2), ob(o1), ob({}));
        console.log(bi(1n), bi(1), bi('b'), bi(2n));
        "#,
        2,
    );
    assert_eq!(out, lines(&["o2 o1 none", "big other str other"]));
}

/// `break` inside a switch inside a loop ends the switch only; `continue` inside a switch reaches the loop.
#[test]
fn switch_inside_loop_break_and_continue() {
    let out = same(
        r#"
        function lp(n) { var out = ''; for (var i = 0; i < n; i++) { switch (i % 3) { case 0: out += 'z'; break; case 1: out += 'o'; continue; default: out += 't'; } out += '|'; } return out }
        console.log(lp(5), lp(0));
        console.log(lp(5), lp(0));
        "#,
        1,
    );
    assert_eq!(out, lines(&["z|ot|z|o ", "z|ot|z|o "]));
}

/// A loop inside a switch case: its `break` ends the loop, not the switch.
#[test]
fn loop_break_inside_switch_case_ends_the_loop() {
    let out = same(
        r#"
        function inner(x) { var c = 0; switch (x) { case 1: for (var j = 0; j < 10; j++) { if (j === 3) break; c++; } c += 100; break; case 2: c = -1; } return c }
        console.log(inner(1), inner(2), inner(4));
        console.log(inner(1), inner(2), inner(4));
        "#,
        1,
    );
    assert_eq!(out, lines(&["103 -1 0", "103 -1 0"]));
}

#[test]
fn switch_break_without_loop_and_nested_switches() {
    let out = same(
        r#"
        function bd(x) { switch (x) { case 1: break; } return 'after' }
        function ns(a, b) { var o = ''; switch (a) { case 1: switch (b) { case 1: o += 'a'; break; default: o += 'b'; } o += 'x'; break; default: o += 'y' } return o }
        console.log(bd(1), bd(2), ns(1, 1), ns(1, 2), ns(2, 0));
        console.log(bd(1), bd(2), ns(1, 1), ns(1, 2), ns(2, 0));
        "#,
        2,
    );
    assert_eq!(out, lines(&["after after ax bx y", "after after ax bx y"]));
}

#[test]
fn switch_case_body_throws_and_returns() {
    let out = same(
        r#"
        function th(x) { switch (x) { case 1: throw 'boom'; default: return 'ok' } }
        try { th(1) } catch (e) { console.log('caught', e) }
        console.log(th(2));
        try { th(1) } catch (e) { console.log('caught', e) }
        console.log(th(2));
        "#,
        1,
    );
    assert_eq!(out, lines(&["caught boom", "ok", "caught boom", "ok"]));
}

/// The analyzer still refuses case-scoped declarations (`SwitchLexical`), and the output is the AST's.
#[test]
fn switch_lexical_declarations_in_a_case_stay_refused() {
    let programs = [
        "function f(x) { switch (x) { case 1: let q = 1; return 5; default: return 0 } } console.log(f(1), f(2)); console.log(f(1), f(2));",
        "function g(x) { switch (x) { case 1: const k = 5; return 6; default: return 0 } } console.log(g(1), g(2)); console.log(g(1), g(2));",
    ];
    for src in programs {
        let (want, ok_want, _) = run_mode(src, CompileMode::Off);
        let (got, ok, it) = run_mode(src, CompileMode::On);
        assert_eq!((got, ok), (want, ok_want), "{src}");
        assert_eq!(it.cstats.compiled(), 0, "{src}");
        assert_eq!(
            it.cstats.rejected(FallbackReason::SwitchLexical),
            1,
            "{src}"
        );
    }
}
