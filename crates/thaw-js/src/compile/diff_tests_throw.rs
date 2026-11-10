//! Tests of the compiled `throw` statement (plan TJ156). Programs run under `off` (the reference), `on` and
//! `check` (`compile_after = 1`); output and ok-ness must match, and the compiled counters prove the throwing
//! functions really were compiled, so an equal output cannot come from the AST alone.

use super::*;
use crate::Interp;

fn run_mode(src: &str, mode: CompileMode) -> (Vec<String>, bool, Interp) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it)
}

/// Off, On and Check print the same; On and Check compile at least `min_compiled` functions.
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

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Thrown values of every kind reach the AST caller unchanged.
#[test]
fn throw_values_of_every_kind_equal_off() {
    let out = same(
        r#"
        function tn() { throw 42 }
        function ts() { throw 'boom' }
        function tu() { throw undefined }
        function tz() { throw null }
        function tb(x) { throw x > 1 }
        function to() { throw { code: 7 } }
        function tc(x) { if (x) throw new TypeError('bad ' + x); return 0 }
        try { tn() } catch (e) { console.log(typeof e, e) }
        try { ts() } catch (e) { console.log(typeof e, e) }
        try { tu() } catch (e) { console.log(typeof e, e) }
        try { tz() } catch (e) { console.log(typeof e, e) }
        try { tb(2) } catch (e) { console.log(typeof e, e) }
        try { to() } catch (e) { console.log(typeof e, e.code) }
        try { tc(3) } catch (e) { console.log(e.name, e.message) }
        console.log(tc(0));
        "#,
        4,
    );
    assert_eq!(
        out,
        lines(&[
            "number 42",
            "string boom",
            "undefined undefined",
            "object null",
            "boolean true",
            "object 7",
            "TypeError bad 3",
            "0",
        ])
    );
}

/// The thrown expression is evaluated once, left to right, and a `throw` inside a loop and a branch stops the loop.
#[test]
fn throw_in_loop_and_branch_evaluates_in_order() {
    let out = same(
        r#"
        function side(x) { console.log('side ' + x); return x }
        function pick(a, b) { return side(a) + side(b) }
        function walk(n) {
          var i = 0;
          while (i < n) {
            i++;
            if (i === 3) { throw 'stop at ' + pick(i, 10) }
          }
          return i
        }
        try { walk(10) } catch (e) { console.log(e) }
        console.log(walk(2));
        "#,
        1,
    );
    assert_eq!(out, lines(&["side 3", "side 10", "stop at 13", "2",]));
}

/// An uncaught throw from a compiled function ends the program the same way (ok-ness is compared by `same`).
#[test]
fn uncaught_throw_from_compiled_function_equals_off() {
    let out = same(
        r#"
        function boom(x) { throw new RangeError('uncaught ' + x) }
        console.log('before');
        boom(5);
        console.log('after');
        "#,
        1,
    );
    assert_eq!(out, lines(&["before"]));
}

/// A rethrow through several compiled frames is caught by an AST `try` in the caller.
#[test]
fn rethrow_through_compiled_frames_is_caught_by_ast_caller() {
    let src = r#"
        function a3(x) { if (x > 1) throw new Error('big ' + x); return x }
        function a2(x) { return a3(x) * 2 }
        function a1(x) { try { return a2(x) + 1 } catch (e) { throw 'rethrown: ' + e.message } }
        function top(x) { return a1(x) }
        try { console.log(top(1)) } catch (e) { console.log('no', e) }
        try { console.log(top(2)) } catch (e) { console.log('outer', e) }
        "#;
    let out = same(src, 2);
    assert_eq!(out, lines(&["3", "outer rethrown: big 2"]));
}

/// A compiled caller of an AST function that throws: the error passes through the compiled frame to the caller.
#[test]
fn compiled_caller_of_throwing_ast_function() {
    let src = r#"
        function thrower(x) { try { throw x } catch (e) { if (e > 5) throw e + 1; return -1 } }
        function go(x) { return thrower(x) + 100 }
        console.log(go(1));
        try { go(9) } catch (e) { console.log('got', e) }
        "#;
    let out = same(src, 1);
    assert_eq!(out, lines(&["99", "got 10"]));
    let (_, _, it) = run_mode(src, CompileMode::On);
    assert!(it.cstats.rejected(FallbackReason::Try) >= 1);
}

/// Negative control: a `try` inside the function still keeps it on the AST, and the output is unchanged.
#[test]
fn try_inside_the_function_still_blocks_it() {
    let src = "function t() { try { throw 1 } catch (e) { return 'c' } } console.log(t()); console.log(t());";
    let out = same(src, 0);
    assert_eq!(out, lines(&["c", "c"]));
    let (_, _, it) = run_mode(src, CompileMode::On);
    assert_eq!(it.cstats.compiled(), 0);
    assert_eq!(it.cstats.rejected(FallbackReason::Try), 1);
}

/// A rethrow from an AST `catch` into a compiled callee and back: `finally` in the AST caller still runs.
#[test]
fn finally_in_ast_caller_runs_around_a_compiled_throw() {
    let src = r#"
        function t(x) { throw x }
        function run(x) {
          try { t(x) } catch (e) { console.log('caught', e) } finally { console.log('finally', x) }
          return 'done'
        }
        console.log(run('a'));
        console.log(run('b'));
        "#;
    let out = same(src, 1);
    assert_eq!(
        out,
        lines(&[
            "caught a",
            "finally a",
            "done",
            "caught b",
            "finally b",
            "done"
        ])
    );
}

/// `depth` returns to zero after a throw through 50 compiled frames (one self-recursive frame per level, under MAX_DEPTH), and the frames really were compiled calls.
#[test]
fn throw_through_fifty_compiled_frames_restores_depth() {
    let src = r#"
        function d(n) { if (n === 0) { throw new RangeError('bottom') } return d(n - 1) + 1 }
        try { d(50) } catch (x) { console.log(x.name, x.message) }
        try { d(50) } catch (x) { console.log(x.name) }
    "#;
    let out = same(src, 1);
    assert_eq!(out, lines(&["RangeError bottom", "RangeError"]));
    let (_, _, it) = run_mode(src, CompileMode::On);
    assert_eq!(it.depth, 0);
    assert!(
        it.cstats.direct_calls() >= 50,
        "{}",
        it.cstats.direct_calls()
    );
}

/// Check mode on a pure kernel that throws on one path: both runs throw, and the outcome agrees.
#[test]
fn check_mode_pure_kernel_that_throws() {
    let src = r#"
        function pk(x) { if (x > 1) throw x; return x + 1 }
        console.log(pk(0), pk(1));
        try { pk(5) } catch (e) { console.log('pk', e) }
        try { pk(7) } catch (e) { console.log('pk', e) }
        "#;
    let out = same(src, 1);
    assert_eq!(out, lines(&["1 2", "pk 5", "pk 7"]));
    let (_, _, it) = run_mode(src, CompileMode::Check);
    assert_eq!(it.cstats.compiled(), 1);
}
