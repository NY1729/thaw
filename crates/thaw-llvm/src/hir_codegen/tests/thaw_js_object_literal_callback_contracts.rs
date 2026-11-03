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
    assert!(ir.contains("@thaw_tjs_register_native_callback_graph("), "{ir}");
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
    assert!(!ir.contains("@thaw_tjs_register_native_callback_graph("), "{ir}");
}
