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
