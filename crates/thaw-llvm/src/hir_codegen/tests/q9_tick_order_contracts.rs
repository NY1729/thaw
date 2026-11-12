// Routed thaw-js entry ordering. Node: a CommonJS main runs its process.nextTick callbacks before the promise
// jobs it queued; an ES module body runs inside a microtask, so its promise jobs run before its ticks.

fn routed_entry_ir(source: &str, commonjs: bool) -> (bool, String) {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "q9_tick_order");
    compiler.set_commonjs_entry(commonjs);
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    (compiler.uses_thaw_js(), compiler.print_to_string())
}

#[test]
fn routed_commonjs_entry_drains_next_ticks_before_native_promise_jobs() {
    let (uses_thaw_js, ir) = routed_entry_ir(THAW_JS_HANDLE_PROGRAM, true);
    assert!(uses_thaw_js, "{ir}");
    let entry_start = ir.find("define i32 @main(").unwrap_or_else(|| panic!("no entry: {ir}"));
    let entry = &ir[entry_start..];
    let ticks = entry
        .find("@thaw_tjs_drain_next_ticks(")
        .unwrap_or_else(|| panic!("routed CommonJS entry has no tick drain: {ir}"));
    let native = entry
        .find("@thaw_runtime_run_until_idle(")
        .unwrap_or_else(|| panic!("routed entry has no native drain: {ir}"));
    assert!(ticks < native, "tick drain must precede the native drain: {ir}");
}

#[test]
fn routed_es_module_entry_keeps_promise_jobs_before_next_ticks() {
    let (uses_thaw_js, ir) = routed_entry_ir(THAW_JS_HANDLE_PROGRAM, false);
    assert!(uses_thaw_js, "{ir}");
    let entry_start = ir.find("define i32 @main(").unwrap_or_else(|| panic!("no entry: {ir}"));
    assert!(
        !ir[entry_start..].contains("@thaw_tjs_drain_next_ticks("),
        "an ES module entry must not drain ticks before its promises: {ir}"
    );
}
