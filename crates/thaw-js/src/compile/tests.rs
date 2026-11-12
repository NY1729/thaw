use super::*;

#[test]
fn env_mode_parsing() {
    assert_eq!(CompileMode::parse(None), CompileMode::Off);
    assert_eq!(CompileMode::parse(Some("off")), CompileMode::Off);
    assert_eq!(CompileMode::parse(Some("on")), CompileMode::On);
    assert_eq!(CompileMode::parse(Some(" check ")), CompileMode::Check);
    assert_eq!(CompileMode::parse(Some("ON")), CompileMode::Off);
    assert_eq!(CompileMode::parse(Some("")), CompileMode::Off);
    assert_eq!(CompileMode::parse(Some("yes")), CompileMode::Off);
}

#[test]
fn env_after_parsing() {
    assert_eq!(parse_after(None), DEFAULT_AFTER);
    assert_eq!(parse_after(Some("1")), 1);
    assert_eq!(parse_after(Some(" 5 ")), 5);
    assert_eq!(parse_after(Some("0")), DEFAULT_AFTER);
    assert_eq!(parse_after(Some("-3")), DEFAULT_AFTER);
    assert_eq!(parse_after(Some("many")), DEFAULT_AFTER);
}

#[test]
fn env_stats_parsing() {
    assert!(!stats_enabled(None));
    assert!(!stats_enabled(Some("")));
    assert!(!stats_enabled(Some("0")));
    assert!(stats_enabled(Some("1")));
    assert!(stats_enabled(Some("2")));
}

#[test]
fn stats_counters() {
    let mut s = Stats::default();
    assert_eq!(s.rejected(FallbackReason::NotYet), 0);
    s.record_rejected(FallbackReason::NotYet);
    s.record_rejected(FallbackReason::NotYet);
    assert_eq!(s.rejected(FallbackReason::NotYet), 2);
}

#[test]
fn cstate_cold_counts_to_threshold() {
    let mut st = CState::default();
    assert!(matches!(st, CState::Cold(0)));
    assert!(!st.on_call(3));
    assert!(!st.on_call(3));
    assert!(st.on_call(3));
    assert!(matches!(st, CState::Cold(3)));
}

#[test]
fn cstate_threshold_one_fires_on_first_call() {
    let mut st = CState::default();
    assert!(st.on_call(1));
}

#[test]
fn cstate_never_is_terminal() {
    let mut st = CState::default();
    st.reject(FallbackReason::NotYet);
    assert!(matches!(st, CState::Never(FallbackReason::NotYet)));
    assert!(!st.on_call(1));
    assert!(!st.on_call(1));
}

#[test]
fn check_mismatch_message_format() {
    assert_eq!(
        check::mismatch_message("f", "3:5", "x differs"),
        "thaw-js compile check mismatch in f (3:5): x differs"
    );
}

/// With every function rejected, `on` must behave exactly like `off`.
#[test]
fn compile_on_matches_off_when_everything_is_rejected() {
    let src = r#"
        function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
        const add = (a, b) => a + b;
        function thrower(x) { if (x > 2) throw new Error("big " + x); return x; }
        let acc = 0;
        for (let i = 0; i < 6; i++) {
            acc = add(acc, fib(i));
            try { acc += thrower(i); } catch (e) { console.log(e.message); }
        }
        console.log("acc", acc);
    "#;
    let mut off = Interp::with_compile_mode(CompileMode::Off);
    assert!(off.run(src).is_ok());
    let mut on = Interp::with_compile_mode(CompileMode::On);
    on.compile_after = 1;
    assert!(on.run(src).is_ok());
    assert_eq!(on.out, off.out);
    assert!(!on.out.is_empty());
    // C3: calls now compile, so no NotYet rejection remains
    assert_eq!(off.cstats.rejected(FallbackReason::NotYet), 0);
}

/// `Interp::runtime()` (the entry used by `Package::load`, the default realm, workers and thaw-js-node) turns the
/// compiler on unless `THAW_JS_COMPILE` is set; `Interp::new()` keeps `Off`. The env var is process-wide, so
/// this only checks the unset case when the test runner has it unset.
#[test]
fn runtime_realm_defaults_to_on_and_new_stays_off() {
    if std::env::var_os("THAW_JS_COMPILE").is_some() {
        return;
    }
    assert_eq!(Interp::runtime().compile_mode, CompileMode::On);
    assert_eq!(Interp::new().compile_mode, CompileMode::Off);
}

/// W1: a function whose body has a loop is compiled on its first call, not on the `compile_after`-th call.
#[test]
fn loop_function_compiles_on_first_call() {
    let src = "function sum(n) { let s = 0; for (let i = 0; i < n; i++) s += i; return s; } console.log(sum(10));";
    let mut it = Interp::with_compile_mode(CompileMode::On);
    assert!(it.run(src).is_ok());
    assert_eq!(it.out, vec!["45".to_string()]);
    assert_eq!(
        it.cstats.compiled, 1,
        "a loop function must compile on its first call"
    );
    assert_eq!(it.cstats.compiled_calls, 1);
}

/// W1: a loop-free function still waits for `compile_after` calls.
#[test]
fn loop_free_function_still_waits_for_threshold() {
    let src = "function add(a, b) { return a + b; } console.log(add(1, 2));";
    let mut it = Interp::with_compile_mode(CompileMode::On);
    assert!(it.run(src).is_ok());
    assert_eq!(it.out, vec!["3".to_string()]);
    assert_eq!(it.cstats.compiled, 0);
}

/// W1: loops inside a nested function or arrow do not make the outer function a loop function.
#[test]
fn loop_in_nested_function_does_not_count_for_outer() {
    let src = "function outer() { const f = () => { for (let i = 0; i < 2; i++) {} }; return typeof f; } console.log(outer());";
    let mut it = Interp::with_compile_mode(CompileMode::On);
    assert!(it.run(src).is_ok());
    assert_eq!(it.out, vec!["function".to_string()]);
    assert_eq!(it.cstats.compiled, 0);
}

/// W1: a loop function run once gives the same output compiled as on the AST (break, labels, closures
/// captured per iteration, return from a loop, throw out of a loop, a rejected for-of).
#[test]
fn loop_function_called_once_matches_ast() {
    let src = r#"
        function f(n) {
            const fns = [];
            let acc = 0;
            outer: for (let i = 0; i < n; i++) {
                for (let j = 0; j < n; j++) {
                    if (j === 2) continue;
                    if (i * j > 6) break outer;
                    acc += i * 10 + j;
                    fns.push(() => i + j);
                }
            }
            let k = 0;
            while (true) { if (++k > 3) break; }
            do { k += 10; } while (k < 30);
            return [acc, k, fns.map(q => q()).join(",")].join(";");
        }
        console.log(f(4));
        function r(x) { for (let i = 0; i < 5; i++) { if (i === x) return "hit" + i; } return "miss"; }
        console.log(r(2), r(9));
        function t() { let s = 0; for (let i = 0; i < 3; i++) { if (i == 1) throw new Error("boom " + i); s += i; } return s; }
        try { t(); } catch (e) { console.log(e.message); }
        function o(xs) { let c = 0; for (const x of xs) c += x; return c; }
        console.log(o([1, 2, 3]));
    "#;
    let mut off = Interp::with_compile_mode(CompileMode::Off);
    assert!(off.run(src).is_ok());
    let mut on = Interp::with_compile_mode(CompileMode::On);
    assert!(on.run(src).is_ok());
    assert_eq!(on.out, off.out);
    assert_eq!(on.out.len(), 4);
    assert!(
        on.cstats.compiled >= 1,
        "the loop functions must compile on their first call"
    );
}

/// W8: the glibc tuning succeeds where it applies (gnu targets), and `runtime()` still builds a realm.
#[test]
fn malloc_tuning_applies_on_gnu() {
    assert_eq!(
        tune_malloc(),
        cfg!(all(target_os = "linux", target_env = "gnu"))
    );
    let mut it = Interp::runtime();
    assert!(it.run("console.log(1 + 1)").is_ok());
    assert_eq!(it.out, vec!["2".to_string()]);
}
