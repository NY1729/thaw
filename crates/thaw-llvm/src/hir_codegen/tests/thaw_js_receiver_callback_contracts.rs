// A callback that reads the call-time receiver (`function (this: T) {}`) passed to a thaw-js typed
// call is registered through the graph path, so its `this` arrives with each call, and it keeps the
// closure identity so `off(listener)` finds it. No QuickJS fallback.

#[test]
fn routed_function_with_this_callback_is_a_graph_handle_with_identity() {
    let run = hex("k::run");
    let source = format!(
        "declare function __thaw_typed_tjs_{run}(listener: (v: number) => void): void;
         function main(): void {{
           __thaw_typed_tjs_{run}(function (this: Json, v: number): void {{ console.log(typeof this, v); }});
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_receiver_callback");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    let calls = |name: &str| ir.lines().any(|line| line.contains("call ") && line.contains(name));
    assert!(calls("@thaw_tjs_register_native_callback_graph("), "{ir}");
    assert!(calls("@thaw_tjs_callback_identity("), "{ir}");
    assert!(!calls("@thaw_tjs_register_callback("), "{ir}");
    assert!(!ir.contains("__thaw_tjs_cb__"), "{ir}");
}
