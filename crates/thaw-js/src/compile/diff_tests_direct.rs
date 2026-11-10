//! Tests of the compiled-to-compiled call fast path, `Interp::call_direct` (plan TJ151 section 5, PR2). Programs run
//! under `off` (the reference), `on` and `check` (`compile_after = 1`); output and ok-ness must match. The
//! `direct_calls` counter proves the fast path was taken, so an equal output cannot come from the generic path alone.

use super::check::CHECK_FAILPOINT;
use super::*;
use crate::{Ctl, Interp, Value};
use std::time::{Duration, Instant};

fn run_mode(src: &str, mode: CompileMode) -> (Vec<String>, bool, Interp) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it)
}

/// Off, On and Check print the same; On and Check compile at least `min_compiled` functions.
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
fn direct_recursion_below_the_limit_equals_off() {
    let out = same(
        r#"
        function ra(n) { return n === 0 ? 0 : 1 + rb(n - 1) }
        function rb(n) { return n === 0 ? 0 : 1 + ra(n - 1) }
        console.log(ra(50));
        console.log(ra(50));
        "#,
        1,
    );
    assert_eq!(out, lines(&["50", "50"]));
    let it = run_mode(
        "function ra(n) { return n === 0 ? 0 : 1 + rb(n - 1) } function rb(n) { return n === 0 ? 0 : 1 + ra(n - 1) } console.log(ra(50));",
        CompileMode::On,
    )
    .2;
    assert!(
        // rb's first call is Cold (generic), so 49 of the 50 nested calls take the fast path
        it.cstats.direct_calls() >= 40,
        "{}",
        it.cstats.direct_calls()
    );
}

#[test]
fn direct_mutual_recursion_equals_off() {
    let out = same(
        r#"
        function isEven(n) { return n === 0 ? true : isOdd(n - 1) }
        function isOdd(n) { return n === 0 ? false : isEven(n - 1) }
        console.log(isEven(20), isOdd(7));
        console.log(isEven(20), isOdd(7));
        "#,
        2,
    );
    assert_eq!(out, lines(&["true true", "true true"]));
    let it = run_mode(
        "function isEven(n) { return n === 0 ? true : isOdd(n - 1) } function isOdd(n) { return n === 0 ? false : isEven(n - 1) } console.log(isEven(20));",
        CompileMode::On,
    )
    .2;
    assert!(it.cstats.direct_calls() > 0);
}

#[test]
fn direct_infinite_recursion_is_a_catchable_range_error() {
    let src = r#"
        function inf(n) { return inf2(n + 1) }
        function inf2(n) { return inf(n + 1) }
        try { inf(0) } catch (e) { console.log(e.name, e.message) }
        console.log("after");
    "#;
    let out = same(src, 1);
    assert_eq!(
        out,
        lines(&["RangeError Maximum call stack size exceeded", "after"])
    );
    for mode in [CompileMode::On, CompileMode::Check] {
        let (_, _, it) = run_mode(src, mode);
        assert_eq!(it.depth, 0, "{mode:?}: depth after the catch");
    }
}

/// `depth` returns to zero after a throw through 50 compiled frames, and the frames really were compiled calls.
#[test]
fn direct_depth_returns_to_zero_after_a_throw_through_frames() {
    let src = r#"
        function d(n) { if (n === 0) { return zz() } return e(n - 1) + 1 }
        function e(n) { return d(n) }
        try { d(20) } catch (e) { console.log(e.name) }
        try { d(20) } catch (e) { console.log(e.name) }
    "#;
    let out = same(src, 1);
    assert_eq!(out, lines(&["ReferenceError", "ReferenceError"]));
    let (_, _, it) = run_mode(src, CompileMode::On);
    assert_eq!(it.depth, 0);
    assert!(
        it.cstats.direct_calls() >= 60,
        "{}",
        it.cstats.direct_calls()
    );
}

/// The `run_id` swap is restored on the error path of a direct call.
#[test]
fn direct_run_id_restored_after_exception() {
    let mut it = Interp::with_compile_mode(CompileMode::On);
    it.compile_after = 1;
    assert!(it
        .run_sloppy("function bad() { return zz() } function bad2() { return bad() }")
        .is_ok());
    let g = it.global.clone();
    let f = it.lookup(&g, "bad2").expect("bad2 defined");
    for _ in 0..3 {
        it.run_id = 777;
        let r = it.call_direct(&f, &Value::Undefined, &[]);
        assert!(r.is_err());
        assert_eq!(it.run_id, 777, "run_id after a thrown direct call");
        assert_eq!(it.depth, 0);
    }
    assert!(
        it.cstats.direct_calls() >= 2,
        "{}",
        it.cstats.direct_calls()
    );
}

/// A deadline stops a compiled loop whose body makes calls, with the Node error code.
#[test]
fn direct_deadline_times_out_a_loop_with_calls() {
    let src = "function h(x) { return x + 1 } function spin() { var i = 0; while (true) { i = h(i) } } spin();";
    for mode in [CompileMode::Off, CompileMode::On] {
        let mut it = Interp::with_compile_mode(mode);
        it.compile_after = 1;
        it.deadline = Some(Instant::now() + Duration::from_millis(20));
        let t = Instant::now();
        let r = it.run_sloppy(src);
        assert!(t.elapsed() < Duration::from_secs(5), "{mode:?}");
        let Err(Ctl::Throw(v)) = r else {
            panic!("{mode:?}: expected a timeout");
        };
        let code = it.get(&v, "code").unwrap_or(Value::Undefined);
        assert_eq!(
            it.to_str(&code),
            "ERR_SCRIPT_EXECUTION_TIMEOUT".into(),
            "{mode:?}"
        );
        assert!(it.dl_hit, "{mode:?}");
        if mode == CompileMode::On {
            assert!(it.cstats.direct_calls() > 0);
        }
    }
}

/// Check mode: a pure-kernel callee reached from a compiled caller is still compared (the failpoint makes the
/// compiled result wrong, so the caught error shows the check ran), and it never takes the direct path.
#[test]
fn direct_check_mode_keeps_pure_kernels_checked() {
    let src = "function k(a) { return a * 2 + 1 } function c(x) { return k(x) } try { c(3) } catch (e) { console.log('caught') }";
    let mut it = Interp::with_compile_mode(CompileMode::Check);
    it.compile_after = 1;
    CHECK_FAILPOINT.with(|c| c.set(true));
    let ok = it.run_sloppy(src).is_ok();
    CHECK_FAILPOINT.with(|c| c.set(false));
    assert!(ok);
    assert_eq!(it.out, lines(&["caught"]));
    // The failpoint makes check mode build an error object, which runs prelude JS functions that compile
    // (and call each other directly) now that in/instanceof and regex literals compile. So the "no direct
    // call" assertion runs the same program in check mode without the failpoint.
    let mut quiet = Interp::with_compile_mode(CompileMode::Check);
    quiet.compile_after = 1;
    assert!(quiet.run_sloppy(src).is_ok());
    assert_eq!(
        quiet.cstats.direct_calls(),
        0,
        "pure kernel must not bypass the check"
    );
}

/// Natives, proxies, bound functions, classes, generators, async functions, and `arguments` callees, from a compiled
/// caller, print what `off` prints.
#[test]
fn direct_non_compiled_callees_equal_off() {
    let src = r#"
        class K { }
        function* gen() { yield 1 }
        async function af() { return 5 }
        function ar() { return arguments.length }
        const p = new Proxy(function (a) { return a + 1 }, {});
        const bd = function (a, b) { return a + b }.bind(null, 1);
        function viaNative() { return Math.max(1, 2) + String(3).length }
        function viaProxy() { return p(1) }
        function viaBound() { return bd(2) }
        function viaArgs() { return ar(1, 2, 3) }
        function viaClass() { return K() }
        function viaGen() { return gen().next().value }
        function viaAsync() { return typeof af() }
        for (const f of [viaNative, viaProxy, viaBound, viaArgs, viaGen, viaAsync, viaClass]) {
          try { console.log(f()) } catch (e) { console.log(e.message) }
        }
    "#;
    let out = same(src, 1);
    assert_eq!(
        out,
        lines(&[
            "3",
            "2",
            "3",
            "3",
            "1",
            "object",
            "Class constructor K cannot be invoked without 'new'",
        ])
    );
}

/// Plain and method calls take the fast path; `new` never does (it goes through `construct`).
#[test]
fn direct_method_and_plain_calls_take_the_fast_path_new_does_not() {
    let src = r#"
        const o = { n: 1, m(x) { return this.n + x } };
        function h(x) { return x + 1 }
        function go(k) { var s = 0; for (var i = 0; i < k; i++) { s = h(s) + o.m(0) } return s }
        console.log(go(10));
        console.log(go(10));
    "#;
    let out = same(src, 2);
    assert_eq!(out, lines(&["20", "20"]));
    let (_, _, it) = run_mode(src, CompileMode::On);
    // 10 plain calls to `h` in each `go`; `o.m` is an object-literal method and may stay on the AST
    assert!(
        it.cstats.direct_calls() >= 20,
        "{}",
        it.cstats.direct_calls()
    );
    let nw = r#"
        function K2(a) { this.v = a }
        function mk(k) { var t = 0; for (var i = 0; i < k; i++) { t = new K2(i).v } return t }
        console.log(mk(10));
        console.log(mk(10));
    "#;
    let out = same(nw, 2);
    assert_eq!(out, lines(&["9", "9"]));
    let (_, _, it) = run_mode(nw, CompileMode::On);
    assert_eq!(
        it.cstats.direct_calls(),
        0,
        "new must not take the direct path"
    );
}

/// Tier A (plan 2.1) is unchanged: a compiled caller reading a script-level `let` in its TDZ throws the same
/// ReferenceError as `off`, and a read-before-declaration function stays rejected.
#[test]
fn direct_tier_a_is_unchanged() {
    let src = r#"
        function readLater() { return lateLet }
        function caller() { return readLater() }
        try { caller() } catch (e) { console.log(e.name) }
        try { caller() } catch (e) { console.log(e.name) }
        let lateLet = 2;
        console.log(caller());
    "#;
    let out = same(src, 1);
    assert_eq!(out, lines(&["ReferenceError", "ReferenceError", "2"]));
    let (_, _, it) = run_mode(
        "function f() { return g(x); let x = 1; } try { f() } catch (e) { console.log(e.name) }",
        CompileMode::On,
    );
    assert_eq!(it.cstats.rejected(FallbackReason::ReadBeforeDecl), 1);
}

/// The deepest recursion under the default limit (100 frames) succeeds in every mode, with no native stack overflow.
#[test]
fn direct_deepest_default_recursion_succeeds() {
    let src = "function ra(n) { return n === 0 ? 0 : 1 + rb(n - 1) } function rb(n) { return n === 0 ? 0 : 1 + ra(n - 1) } console.log(ra(95));";
    let out = same(src, 1);
    assert_eq!(out, lines(&["95"]));
}

/// A `configure_stack` realm allows much deeper recursion. The thread's stack size equals the configured size (the
/// stack check only stops recursion when this thread's own stack is also short), so the limit is the real one.
/// The deep recursion must succeed in On exactly as in Off: no native stack overflow, no spurious RangeError.
#[test]
fn direct_deepest_configured_recursion_succeeds() {
    const STACK: usize = 128 << 20;
    let handle = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(|| {
            let src = "function ra(n) { return n === 0 ? 0 : 1 + rb(n - 1) } function rb(n) { return n === 0 ? 0 : 1 + ra(n - 1) } console.log(ra(2000));";
            let mut out = Vec::new();
            for mode in [CompileMode::Off, CompileMode::On] {
                let mut it = Interp::with_compile_mode(mode);
                it.compile_after = 1;
                it.configure_stack(STACK);
                let ok = it.run_sloppy(src).is_ok();
                assert!(ok, "{mode:?}: deep recursion failed: {:?}", it.out);
                out.push(it.out.clone());
            }
            assert_eq!(out[0], out[1]);
            assert_eq!(out[0], lines(&["2000"]));
        })
        .expect("spawn");
    handle.join().expect("deep recursion thread");
}
