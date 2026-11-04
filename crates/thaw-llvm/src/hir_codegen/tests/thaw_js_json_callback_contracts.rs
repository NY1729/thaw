// A callback with a `Json` (`any`) parameter passed to a thaw-js typed call is a graph callback
// with live argument handles, so the listener receives the live value rather than a JSON copy.
// Callbacks without a `Json` parameter and rest callbacks keep the JSON-text protocol.

#[test]
fn any_parameter_callback_is_a_graph_handle_with_identity() {
    let listen = hex("k::listen");
    let source = format!(
        "declare function __thaw_typed_tjs_{listen}(listener: (o: any) => void): void;
         function main(): void {{
           __thaw_typed_tjs_{listen}((o: any): void => {{ console.log(typeof o); }});
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_json_callback");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    let calls = |name: &str| ir.lines().any(|line| line.contains("call ") && line.contains(name));
    assert!(calls("@thaw_tjs_register_native_callback_graph("), "{ir}");
    assert!(calls("@thaw_tjs_callback_identity("), "{ir}");
    assert!(!calls("@thaw_tjs_register_callback("), "{ir}");
    assert!(!ir.contains("__thaw_tjs_cb__"), "{ir}");
}

#[test]
fn any_parameter_callback_coerced_to_a_json_parameter_is_a_graph_handle() {
    let listen = hex("k::listen_json");
    let source = format!(
        "declare function __thaw_typed_tjs_{listen}(listener: Json): void;
         function main(): void {{
           __thaw_typed_tjs_{listen}((o: any): void => {{ console.log(typeof o); }});
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_json_callback_coerced");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    let calls = |name: &str| ir.lines().any(|line| line.contains("call ") && line.contains(name));
    assert!(calls("@thaw_tjs_register_native_callback_graph("), "{ir}");
    assert!(!calls("@thaw_tjs_register_callback("), "{ir}");
}

#[test]
fn number_parameter_callback_keeps_the_json_text_protocol() {
    let listen = hex("k::listen_number");
    let source = format!(
        "declare function __thaw_typed_tjs_{listen}(listener: (x: number) => number): void;
         function main(): void {{
           __thaw_typed_tjs_{listen}((x: number): number => x * 2);
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_number_callback");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("call i64 @thaw_tjs_register_callback("), "{ir}");
    assert!(ir.contains("__thaw_tjs_cb__"), "{ir}");
}

#[test]
fn rest_callback_keeps_the_json_text_protocol_with_any_element() {
    let listen = hex("k::listen_rest");
    let source = format!(
        "declare function __thaw_typed_tjs_{listen}(listener: (...args: Json[]) => void): void;
         function main(): void {{
           __thaw_typed_tjs_{listen}((...args: Json[]): void => {{ console.log(args.length); }});
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_rest_any_callback");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("call i64 @thaw_tjs_register_callback("), "{ir}");
    assert!(ir.contains("__thaw_tjs_cb__"), "{ir}");
}
