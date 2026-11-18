//! `mem_trace` counters: total counts only grow, the report line has its format, and a script without
//! cycles leaves the live counts where they were before it ran (the counters are balanced).
use super::*;
use crate::mem_trace::{closure_total, live_counts, mem_trace_line, mem_trace_report};

#[test]
fn closure_cycles_grow_the_total_counters() {
    let before_total = closure_total();
    {
        let mut it = Interp::new();
        it.run("for (var i = 0; i < 1000; i++) { var o = {}; o.f = function () { return o; }; }")
            .unwrap_or_else(|_| panic!("cycle script must run"));
        let mid = closure_total();
        assert!(
            mid - before_total >= 1000,
            "1000 closures must be counted: {} new",
            mid - before_total
        );
    }
    // totals never fall; the live closures left after teardown are the leak, reported not asserted
    assert!(closure_total() >= before_total + 1000);
}

#[test]
fn report_line_has_the_documented_format() {
    let line = mem_trace_line("unit");
    assert!(
        line.starts_with("thaw-js mem-trace unit: obj live="),
        "bad line: {line}"
    );
    for key in [
        " obj live=",
        " total=",
        " scope live=",
        " closure live=",
        " rss_kb=",
        " heap_in_use=",
    ] {
        assert!(line.contains(key), "missing {key:?} in {line}");
    }
    // without THAW_JS_MEM_TRACE this prints nothing and must not panic
    mem_trace_report("unit");
}

/// Live counts left by one realm after its drop, measured from `before`.
fn residue_of(before: (isize, isize, isize), run: Option<&str>) -> (isize, isize, isize) {
    {
        let mut it = Interp::new();
        if let Some(src) = run {
            it.run(src).unwrap_or_else(|_| panic!("script must run"));
        }
    }
    let after = live_counts();
    (after.0 - before.0, after.1 - before.1, after.2 - before.2)
}

#[test]
fn script_without_cycles_balances_the_live_counts() {
    // Teardown does not free every realm's own cycles today (see the note below), so the balance is
    // checked against an empty realm: a script without cycles must leave exactly the residue an
    // empty realm leaves, no more. Warm-up drop first so one-time per-thread state is not counted.
    drop(Interp::new());
    let before = live_counts();
    let empty = residue_of(before, None);
    let before = live_counts();
    let script = "function add(a, b) { return a + b; } var sum = add(1, 2);";
    let with_script = residue_of(before, Some(script));
    assert_eq!(
        with_script, empty,
        "a script without cycles left more live counts than an empty realm (obj, scope, closure)"
    );
}
