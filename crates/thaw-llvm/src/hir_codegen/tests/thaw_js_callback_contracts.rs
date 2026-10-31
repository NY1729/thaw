// thaw-js (`JsEngine::ThawJs`): a compiled callback that takes a live JS value is a graph callback
// registered with `thaw_tjs_register_native_callback_graph`, and the Promise-awaiting handle
// operations resolve to their `thaw_tjs_*` twins, so the build links no QuickJS.

const THAW_JS_CALLBACK_PROGRAM: &str = "function main(): void {
       const callback: (value: JsValue) => number = (value: JsValue): number => 1;
       const handle: JsValue = registerNativeCallback(callback);
       const resolved: JsValue = resolveDynamicValue(handle);
       const mixed: JsValue = callDynamicValueMixed(handle, JSON.parse(\"[]\"), [resolved]);
       console.log(String(mixed));
     }";

#[test]
fn thaw_js_routed_live_value_callbacks_use_the_graph_registration() {
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(THAW_JS_CALLBACK_PROGRAM, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    for symbol in [
        "thaw_tjs_register_native_callback_graph",
        "thaw_tjs_resolve_handle_handle_result",
        "thaw_tjs_call_handle_mixed_graph_result",
    ] {
        assert!(
            ir.contains(&format!("@{symbol}(")),
            "{symbol} missing: {ir}"
        );
    }
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
fn thaw_js_unrouted_live_value_callbacks_keep_the_quickjs_registration() {
    let (_, uses_quickjs, ir) = thaw_js_routed_ir(THAW_JS_CALLBACK_PROGRAM, false);
    assert!(uses_quickjs, "{ir}");
    assert!(ir.contains("@thaw_js_register_native_callback"), "{ir}");
    assert!(
        !ir.contains("@thaw_tjs_register_native_callback_graph("),
        "{ir}"
    );
}
