// A live object literal with methods, passed to a thaw-js routed call, is built as plain Json by
// the call's argument compile: each method is a receiver callback passed as a `__thaw_tjs_handle__`
// marker, and no QuickJS builder is compiled for it. Getters keep a loud stub, and the QuickJS
// engine keeps the builder.

fn thaw_js_object_literal_program(members: &str) -> String {
    format!(
        "declare function __thaw_typed_tjs_{run}(opts: any): void;
         function main(): void {{
           __thaw_typed_tjs_{run}({members});
         }}",
        run = hex("nest::run"),
        members = members,
    )
}

#[test]
fn thaw_js_routed_object_literal_methods_are_receiver_handles() {
    let source = thaw_js_object_literal_program("{ cb(x: number): number { return x + 1; }, n: 1 }");
    let (uses_thaw_js, _, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("call { i64, ptr } @thaw_tjs_register_native_method_callback_graph("), "{ir}");
    assert!(!ir.contains("call { i64, ptr } @thaw_tjs_register_native_callback_graph("), "{ir}");
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    // the member is no longer built by the QuickJS builder
    assert!(!ir.contains("__thaw_object_from_operations"), "{ir}");
}

#[test]
fn thaw_js_routed_nested_object_literal_members_are_built_inline() {
    let source = thaw_js_object_literal_program(
        "{ inner: { f(x: number): number { return x; } }, n: 1 }",
    );
    let (uses_thaw_js, _, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(!ir.contains("__thaw_object_from_operations"), "{ir}");
    // the outer literal and the nested one each start a Json object
    assert!(ir.matches("@thaw_json_object_new(").count() >= 2, "{ir}");
}

#[test]
fn thaw_js_routed_object_literal_getters_stay_loud() {
    let source = thaw_js_object_literal_program("{ get x(): number { return 1; }, n: 1 }");
    let (_, _, ir) = thaw_js_routed_ir(&source, true);
    assert!(
        ir.contains("thaw-js: object literal getter members cannot cross the thaw-js boundary"),
        "{ir}"
    );
    assert!(!ir.contains("__thaw_object_from_operations"), "{ir}");
}

#[test]
fn quickjs_engine_object_literal_keeps_the_builder() {
    let source = thaw_js_object_literal_program("{ cb(x: number): number { return x + 1; }, n: 1 }");
    // The program's own `__thaw_typed_tjs_` call still reaches thaw-js (`uses_thaw_js` is set by the
    // call, whatever the engine), so only the object literal is checked: the QuickJS builder is
    // fetched through `thaw_js_get_global`, and no thaw-js callback marker or graph registration is used.
    let (_, _, ir) = thaw_js_routed_ir(&source, false);
    assert!(ir.contains("__thaw_object_from_operations"), "{ir}");
    assert!(ir.contains("@thaw_js_get_global("), "{ir}");
    assert!(!ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(!ir.contains("call { i64, ptr } @thaw_tjs_register_native_callback_graph("), "{ir}");
    assert!(!ir.contains("call { i64, ptr } @thaw_tjs_register_native_method_callback_graph("), "{ir}");
}

// A thaw-js routed call builds its method-bearing object literal argument from the members, so the
// QuickJS builder in the marker's first operand must not make the program need QuickJS. The QuickJS
// lane keeps the builder and the QuickJS requirement.

#[test]
fn thaw_js_routed_object_literal_argument_does_not_require_the_quickjs_builder() {
    let source = thaw_js_object_literal_program("{ cb(x: number): number { return x + 1; }, n: 1 }");
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js, "{ir}");
    assert!(!uses_quickjs, "{ir}");
    assert!(!ir.contains("__thaw_object_from_operations"), "{ir}");
}

#[test]
fn quickjs_lane_object_literal_argument_still_requires_quickjs() {
    let source = thaw_js_object_literal_program("{ cb(x: number): number { return x + 1; }, n: 1 }");
    let (_, uses_quickjs, ir) = thaw_js_routed_ir(&source, false);
    assert!(uses_quickjs, "{ir}");
    assert!(ir.contains("__thaw_object_from_operations"), "{ir}");
}

// A closure that is not a thaw-js call argument (held as `any`, or stored in a native Map) must stay a
// live Host lease, never the `{"__thaw_tjs_cb__": id}` / `{"__thaw_tjs_handle__": id}` marker object,
// which only thaw-js decodes when it parses a call's argument text. Call arguments keep the marker.

fn callback_gate_program(body: &str) -> String {
    format!(
        "declare function __thaw_typed_tjs_{run}(f: (x: number) => number): void;
         function main(): void {{
           {body}
         }}",
        run = hex("k::run"),
        body = body,
    )
}

#[test]
fn any_held_closure_is_a_live_lease_not_a_marker() {
    let source = callback_gate_program(
        "const g: any = (x: number): number => x + 1; console.log(g(1));",
    );
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(!ir.contains("__thaw_tjs_cb__"), "{ir}");
    assert!(!ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(ir.contains("@thaw_json_from_borrowed_handle("), "{ir}");
}

#[test]
fn map_stored_closure_is_a_live_lease_not_a_marker() {
    let source = callback_gate_program(
        "const m = new Map<string, any>(); m.set(\"q\", (x: number): number => x); console.log(m.size);",
    );
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(!ir.contains("__thaw_tjs_cb__"), "{ir}");
    assert!(!ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(ir.contains("@thaw_json_from_borrowed_handle("), "{ir}");
}

#[test]
fn closure_passed_to_a_thaw_js_call_keeps_the_callback_marker() {
    let source = callback_gate_program("__thaw_typed_tjs_KEY((x: number): number => x + 1);")
        .replace("KEY", &hex("k::run"));
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(&source, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("__thaw_tjs_cb__"), "{ir}");
}
