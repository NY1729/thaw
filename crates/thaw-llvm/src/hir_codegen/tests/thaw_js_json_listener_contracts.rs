// A listener declared with a rest signature whose own closure takes a bare `Json` parameter is
// registered through the graph route (live handle arguments), by the closure's own type. Closures
// with rest, number or fixed-arity signatures keep the text route.

fn listener_ir(declared: &str, closure: &str) -> String {
    let on = hex("$method$k::Emitter$on$overload0$arity2");
    let new = hex("$new$k::Emitter$arity0");
    let source = format!(
        "declare function __thaw_typed_tjs_{new}(): JsValue;
         declare function __thaw_typed_tjs_{on}(receiver: JsValue, event: Json, listener: {declared}): JsValue;
         function main(): void {{
           const emitter = __thaw_typed_tjs_{new}();
           __thaw_typed_tjs_{on}(emitter, \"x\", {closure});
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_json_listener");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    ir
}

#[test]
fn rest_declared_listener_with_a_json_closure_takes_the_graph_route() {
    let ir = listener_ir(
        "(...args: Json[]) => void",
        "(o: Json): void => { console.log(o); }",
    );
    let calls = |name: &str| ir.lines().any(|line| line.contains("call ") && line.contains(name));
    assert!(calls("@thaw_tjs_register_native_callback_graph("), "{ir}");
    assert!(calls("@thaw_tjs_callback_identity("), "{ir}");
    assert!(!calls("@thaw_tjs_register_callback("), "{ir}");
    assert!(!ir.contains("__thaw_tjs_cb__"), "{ir}");
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
}

#[test]
fn rest_declared_listener_with_a_rest_closure_keeps_the_text_route() {
    let ir = listener_ir(
        "(...args: Json[]) => void",
        "(...args: Json[]): void => { console.log(args.length); }",
    );
    assert!(ir.contains("call i64 @thaw_tjs_register_callback("), "{ir}");
}

#[test]
fn rest_declared_listener_with_a_number_closure_keeps_the_text_route() {
    let ir = listener_ir(
        "(...args: Json[]) => void",
        "(x: number): void => { console.log(x); }",
    );
    assert!(ir.contains("call i64 @thaw_tjs_register_callback("), "{ir}");
}

#[test]
fn fixed_declared_listener_with_a_json_closure_keeps_the_text_route() {
    let ir = listener_ir("(o: Json) => void", "(o: Json): void => { console.log(o); }");
    assert!(ir.contains("call i64 @thaw_tjs_register_callback("), "{ir}");
}

#[test]
fn real_node_events_shape_listener_takes_the_graph_route() {
    let new = hex("$new$node:events::EventEmitter$arity0");
    let on = hex("$method$node:events::EventEmitter$on$overload0$arity2");
    let source = format!(
        "declare function __thaw_typed_tjs_{new}(): JsValue;
         declare function __thaw_typed_tjs_{on}(receiver: JsValue, event: Json, listener: (...args: Json[]) => void): JsValue;
         function main(): void {{
           const ee = __thaw_typed_tjs_{new}();
           __thaw_typed_tjs_{on}(ee, \"e\", (o: any): void => {{ console.log(o); }});
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_real_node_events_listener");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    let calls = |name: &str| ir.lines().any(|line| line.contains("call ") && line.contains(name));
    assert!(calls("@thaw_tjs_register_native_callback_graph("), "{ir}");
    assert!(!calls("@thaw_tjs_register_callback("), "{ir}");
}

#[test]
fn real_node_events_shape_capturing_closure_takes_the_graph_route() {
    let new = hex("$new$node:events::EventEmitter$arity0");
    let on = hex("$method$node:events::EventEmitter$on$overload0$arity2");
    let source = format!(
        "declare function __thaw_typed_tjs_{new}(): JsValue;
         declare function __thaw_typed_tjs_{on}(receiver: JsValue, event: Json, listener: (...args: Json[]) => void): JsValue;
         function main(): void {{
           const arr = [1, 2];
           const ee = __thaw_typed_tjs_{new}();
           __thaw_typed_tjs_{on}(ee, \"e\", (o: any): void => {{ o.push(3); console.log(o === arr, arr.length); }});
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_real_node_events_capture");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    let calls = |name: &str| ir.lines().any(|line| line.contains("call ") && line.contains(name));
    assert!(calls("@thaw_tjs_register_native_callback_graph("), "{ir}");
    assert!(!calls("@thaw_tjs_register_callback("), "{ir}");
}
