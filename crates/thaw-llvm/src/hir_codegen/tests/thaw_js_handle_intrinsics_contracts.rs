// thaw-js (`JsEngine::ThawJs`): the live-handle intrinsics (property reads, selected method calls,
// exception capture, console probes) resolve to `thaw_tjs_*` and the build links no QuickJS.

fn thaw_js_routed_ir(source: &str, routed: bool) -> (bool, bool, String) {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_handles");
    compiler.set_thaw_js_routed(routed);
    compiler.compile_program(&program).unwrap();
    (
        compiler.uses_thaw_js(),
        compiler.uses_quickjs(),
        compiler.print_to_string(),
    )
}

const THAW_JS_HANDLE_PROGRAM: &str =
    "declare function __thaw_typed_tjs_246e6577246b3a3a426f7824617269747931(v: number): JsValue;
     function main(): void {
       const box = __thaw_typed_tjs_246e6577246b3a3a426f7824617269747931(1);
       const sum: JsValue = box.add(2);
       const value: JsValue = box.value;
       console.log(sum, value);
       console.log(box);
     }";

#[test]
fn thaw_js_routed_handle_intrinsics_name_only_thaw_tjs_symbols() {
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(THAW_JS_HANDLE_PROGRAM, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    for symbol in [
        "thaw_tjs_get_property_result",
        "thaw_tjs_retain_handle",
        "thaw_tjs_release_handle",
    ] {
        assert!(
            ir.contains(&format!("@{symbol}(")),
            "{symbol} missing: {ir}"
        );
    }
    // Only declarations of unused QuickJS intrinsics may remain, never a call.
    assert!(
        !ir.lines().any(|line| {
            line.contains(" call ")
                && line
                    .split('(')
                    .next()
                    .is_some_and(|callee| callee.contains("@thaw_js_"))
        }),
        "{ir}"
    );
}

#[test]
fn thaw_js_unrouted_handle_intrinsics_keep_the_quickjs_symbols() {
    let (_, uses_quickjs, ir) = thaw_js_routed_ir(THAW_JS_HANDLE_PROGRAM, false);
    assert!(uses_quickjs, "{ir}");
    assert!(ir.contains("@thaw_js_get_property_result("), "{ir}");
    assert!(!ir.contains("@thaw_tjs_get_property_result("), "{ir}");
}
