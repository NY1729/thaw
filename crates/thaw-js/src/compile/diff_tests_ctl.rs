//! Differential tests of the C2 PR3 control-flow subset (plan TJ150 section 7): `if`, `while`, `do-while`,
//! `for`, unlabeled `break`/`continue`, and the deadline check of loops. Same contract as `diff_tests.rs`:
//! `off` is the reference, `on` and `check` must match it, and the compiled count (functions compiled, not
//! calls) must reach the minimum. Bodies avoid calls and member reads, which are later phases.

use std::time::{Duration, Instant};

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
fn pr3_sum_nested_and_while_loops_with_break_and_continue() {
    let out = same(
        r#"
        function sum(n) { var s = 0; for (var i = 0; i < n; i++) { if (i % 2) continue; s += i; } return s }
        function nest(n) { var out = 0; for (let i = 0; i < n; i++) { for (let j = 0; j < n; j++) { if (j > i) break; if (j === 1) continue; out += i * 10 + j; } } return out }
        function wh(n) { var k = 0, c = 0; while (true) { k++; if (k > n) break; if (k % 3 === 0) continue; c += k; } return c }
        console.log(sum(10), nest(4), wh(10));
        console.log(sum(10), nest(4), wh(10));
        "#,
        3,
    );
    assert_eq!(out, lines(&["20 147 37", "20 147 37"]));
}

#[test]
fn pr3_continue_in_for_runs_the_update() {
    let out = same(
        r#"
        function c() { var out = 0; for (var i = 0; i < 5; i++) { if (i % 2) continue; out = out * 10 + i } return out }
        console.log(c()); console.log(c());
        "#,
        1,
    );
    assert_eq!(out, lines(&["24", "24"]));
}

#[test]
fn pr3_for_without_parts_and_do_while_semantics() {
    let out = same(
        r#"
        function g() { var i = 0; for (;;) { i++; if (i === 3) break } return i }
        function d() { var n = 0; do { n++ } while (false); var m = 0; do { m++; if (m < 3) continue; } while (m < 5); return n * 10 + m }
        console.log(g(), d());
        console.log(g(), d());
        "#,
        2,
    );
    assert_eq!(out, lines(&["3 15", "3 15"]));
}

#[test]
fn pr3_loop_variable_after_for_var_and_let_head() {
    let out = same(
        r#"
        function lv() { for (var i = 0; i < 4; i++) {} return i }
        function lt() { let last = -1; for (let i = 0; i < 4; i++) { last = i } return last }
        console.log(lv(), lt());
        console.log(lv(), lt());
        "#,
        2,
    );
    assert_eq!(out, lines(&["4 3", "4 3"]));
}

#[test]
fn pr3_let_in_loop_body_is_reinitialised_each_iteration() {
    // `let x;` is undefined again on each pass: 5, then 7 (undefined), then 7
    let out = same(
        r#"
        function r() { var out = 0; for (var i = 0; i < 3; i++) { let x; if (i === 0) x = 5; out = out * 10 + (x === undefined ? 7 : x) } return out }
        console.log(r()); console.log(r());
        "#,
        1,
    );
    assert_eq!(out, lines(&["577", "577"]));
}

#[test]
fn pr3_return_from_nested_loops_and_blocks() {
    let out = same(
        r#"
        function find(t) { for (let i = 0; i < 3; i++) { { let j = i; while (true) { if (j === t) return j * 100 + i; break } } } return -1 }
        function first() { var k = 0; do { k++; if (k > 2) { return k * 10 } } while (true) }
        console.log(find(2), find(9), first());
        console.log(find(2), find(9), first());
        "#,
        2,
    );
    assert_eq!(out, lines(&["202 -1 30", "202 -1 30"]));
}

#[test]
fn pr3_empty_bodies_and_comma_update() {
    let out = same(
        r#"
        function e() { var i = 0, j = 0; for (i = 0, j = 10; i < j; i++, j--) ; while (false) ; do ; while (false); return i * 100 + j }
        console.log(e()); console.log(e());
        "#,
        1,
    );
    assert_eq!(out, lines(&["505", "505"]));
}

#[test]
fn pr3_while_with_postfix_test_and_if_else_chains() {
    let out = same(
        r#"
        function w(n) { var c = 0; while (n--) { c++ } return c * 1000 + n }
        function cls(x) { if (x < 0) return "neg"; else if (x === 0) return "zero"; else if (x > 100) { return "big" } return "pos" }
        console.log(w(3), cls(-1), cls(0), cls(5), cls(500));
        console.log(w(3), cls(-1), cls(0), cls(5), cls(500));
        "#,
        2,
    );
    assert_eq!(
        out,
        lines(&["2999 neg zero pos big", "2999 neg zero pos big"])
    );
}

#[test]
fn pr3_shadowed_names_in_sibling_blocks_and_loops() {
    // digits 0,2 from the first loop, 7 from the sibling block, 5 from the last loop
    let out = same(
        r#"
        function s() { var out = 0; for (let k = 0; k < 2; k++) { let v = k * 2; out = out * 10 + v } { let v = 7; out = out * 10 + v } for (let k = 5; k < 6; k++) { let v = k; out = out * 10 + v } return out }
        console.log(s()); console.log(s());
        "#,
        1,
    );
    assert_eq!(out, lines(&["275", "275"]));
}

#[test]
fn pr3_break_and_continue_leave_only_the_innermost_loop() {
    let out = same(
        r#"
        function nb() { var hits = 0; for (var i = 0; i < 3; i++) { for (var j = 0; j < 3; j++) { if (j === 1) break; hits++ } hits += 100 } return hits }
        function nc() { var hits = 0; for (var i = 0; i < 3; i++) { var j = 0; while (j < 3) { j++; if (j === 2) continue; hits++ } } return hits }
        console.log(nb(), nc());
        console.log(nb(), nc());
        "#,
        2,
    );
    assert_eq!(out, lines(&["303 6", "303 6"]));
}

/// A deadline set by the embedder stops a compiled loop exactly as it stops the interpreted one.
#[test]
fn pr3_deadline_stops_compiled_loops() {
    for src in [
        "function spin() { var i = 0; while (true) { i++ } } spin();",
        "function spin() { do { } while (true) } spin();",
        "function spin() { for (;;) { } } spin();",
    ] {
        for mode in [CompileMode::Off, CompileMode::On] {
            let mut it = Interp::with_compile_mode(mode);
            it.compile_after = 1;
            it.deadline = Some(Instant::now() + Duration::from_millis(20));
            let t = Instant::now();
            let ok = it.run_sloppy(src).is_ok();
            assert!(!ok, "{mode:?}: {src}");
            assert!(it.dl_hit, "{mode:?}: {src}");
            assert!(t.elapsed() < Duration::from_secs(5), "{mode:?}: {src}");
            if mode == CompileMode::On {
                assert!(it.cstats.compiled() >= 1, "{src}");
            }
        }
    }
}

/// Tier A (read-before-declaration) loop shapes stay on the AST and behave as `off`.
#[test]
fn pr3_tier_a_loop_programs_stay_on_the_ast() {
    let programs = [
        "function f() { let n = 0; while (n < 2) { if (n === 1) { y; } let y = n; n++ } } try { f() } catch (e) { console.log(e.name) }",
        "function f() { for (let i = 0; i < 2; i++) { if (i === 1) { z; } let z = i } } try { f() } catch (e) { console.log(e.name) }",
    ];
    for src in programs {
        let (want, ok_want, _) = run_mode(src, CompileMode::Off);
        let (got, ok, it) = run_mode(src, CompileMode::On);
        assert_eq!((got, ok), (want, ok_want), "{src}");
        // C3: calls now compile (a JS prelude function with a call compiles); the tier A function itself is
        // still rejected, asserted by the ReadBeforeDecl count below
        assert_eq!(
            it.cstats.rejected(FallbackReason::ReadBeforeDecl),
            1,
            "{src}"
        );
    }
}

/// Loop-adjacent statements outside the subset stay on the AST, with the same output.
#[test]
fn pr3_switch_try_and_labels_stay_on_the_ast() {
    // switch compiles: same output as the AST, and the function is compiled
    let sw = "function s(a) { switch (a) { case 1: return 'one'; default: return 'other' } } console.log(s(1), s(2)); console.log(s(1), s(2));";
    let (want, ok_want, _) = run_mode(sw, CompileMode::Off);
    let (got, ok, it) = run_mode(sw, CompileMode::On);
    assert_eq!((got, ok), (want, ok_want), "{sw}");
    assert!(it.cstats.compiled() >= 1, "{sw}");
    let cases: [(&str, &[FallbackReason]); 2] = [
        (
            "function t() { try { throw 1 } catch (e) { return 'c' } } console.log(t()); console.log(t());",
            &[FallbackReason::Try, FallbackReason::Throw],
        ),
        (
            "function lb() { var i = 0; outer: for (i = 0; i < 2; i++) { for (var j = 0; j < 2; j++) { if (j) continue outer; } } return i } console.log(lb()); console.log(lb());",
            &[FallbackReason::Labels],
        ),
    ];
    for (src, reasons) in cases {
        let (want, ok_want, _) = run_mode(src, CompileMode::Off);
        let (got, ok, it) = run_mode(src, CompileMode::On);
        assert_eq!((got, ok), (want, ok_want), "{src}");
        assert_eq!(it.cstats.compiled(), 0, "{src}");
        let hit: u64 = reasons.iter().map(|r| it.cstats.rejected(*r)).sum();
        assert_eq!(hit, 1, "{src}");
    }
}
