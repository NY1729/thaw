// Routed thaw-js entry ordering (Node: process.nextTick callbacks run before promise and microtask
// jobs queued by the same main). The native drain must come after the thaw-js tick drain.

#[test]
fn routed_entry_drains_next_ticks_before_native_promise_jobs() {
    let (uses_thaw_js, _uses_quickjs, ir) = thaw_js_routed_ir(THAW_JS_HANDLE_PROGRAM, true);
    assert!(uses_thaw_js, "{ir}");
    let entry_start = ir.find("define i32 @main(").unwrap_or_else(|| panic!("no entry: {ir}"));
    let entry = &ir[entry_start..];
    let ticks = entry
        .find("@thaw_tjs_drain_next_ticks(")
        .unwrap_or_else(|| panic!("routed entry has no tick drain: {ir}"));
    let native = entry
        .find("@thaw_runtime_run_until_idle(")
        .unwrap_or_else(|| panic!("routed entry has no native drain: {ir}"));
    assert!(ticks < native, "tick drain must precede the native drain: {ir}");
}
