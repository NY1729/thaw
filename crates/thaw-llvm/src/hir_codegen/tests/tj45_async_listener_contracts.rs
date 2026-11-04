// thaw-js listeners whose own closure returns a Promise (`async () => {...}`) take the graph route
// (`finish` registers the Promise), while a sync listener keeps the JSON marker route.

fn tj45_count_calls(ir: &str, name: &str) -> usize {
    ir.lines().filter(|line| line.contains("call ") && line.contains(name)).count()
}

#[test]
fn tj45_async_listener_is_a_graph_callback_with_a_promise_finish() {
    let listen = tj6_hex("k::tj45listen");
    let source = format!(
        "declare function __thaw_typed_tjs_{listen}(listener: (value: string) => void): void;
         function main(): void {{
           __thaw_typed_tjs_{listen}(async (value: string): Promise<void> => {{ throw new Error(\"b\"); }});
           __thaw_typed_tjs_{listen}((value: string): void => {{ console.log(value); }});
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    // one graph registration (the async listener) and one marker registration (the sync listener)
    assert_eq!(tj45_count_calls(&ir, "@thaw_tjs_register_native_callback_graph("), 1, "{ir}");
    assert_eq!(tj45_count_calls(&ir, "@thaw_tjs_register_callback("), 1, "{ir}");
    assert!(ir.contains("__thaw_native_promise_finish_"), "{ir}");
    assert!(ir.contains("__thaw_tjs_cb__"), "{ir}");
}

#[test]
fn tj45_sync_listener_keeps_the_marker_route() {
    let listen = tj6_hex("k::tj45sync");
    let source = format!(
        "declare function __thaw_typed_tjs_{listen}(listener: (value: string) => void): void;
         function main(): void {{
           __thaw_typed_tjs_{listen}((value: string): void => {{ console.log(value); }});
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert_eq!(tj45_count_calls(&ir, "@thaw_tjs_register_native_callback_graph("), 0, "{ir}");
    assert_eq!(tj45_count_calls(&ir, "@thaw_tjs_register_callback("), 1, "{ir}");
    assert!(!ir.contains("__thaw_native_promise_finish_"), "{ir}");
}
