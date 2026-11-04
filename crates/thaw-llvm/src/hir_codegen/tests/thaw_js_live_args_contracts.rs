// Native arrays, Maps and Sets passed to a thaw-js instance method's `Json` slot are routed as live
// views (the view is built by `__thaw_*_with_native_view`) and the argument is passed through the
// handle conversion, instead of being copied. Literals and numbers keep the copy path.

fn live_arg_hex(key: &str) -> String {
    key.bytes().map(|b| format!("{b:02x}")).collect()
}

fn live_arg_ir(prelude: &str, argument: &str) -> String {
    let new = live_arg_hex("$new$k::Emitter$arity0");
    let emit = live_arg_hex("$method$k::Emitter$emit$overload0$arity3");
    let source = format!(
        "declare function __thaw_typed_tjs_{new}(): JsValue;
         declare function __thaw_typed_tjs_{emit}(receiver: JsValue, event: Json, value: Json): JsValue;
         function main(): void {{
           {prelude}
           const emitter = __thaw_typed_tjs_{new}();
           __thaw_typed_tjs_{emit}(emitter, \"e\", {argument});
         }}"
    );
    let module = thaw_parser::parse_typescript(&source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_live_args");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    let (uses_thaw_js, uses_quickjs) = (compiler.uses_thaw_js(), compiler.uses_quickjs());
    let ir = compiler.print_to_string();
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    ir
}

#[test]
fn thaw_js_method_json_slot_passes_a_native_array_variable_as_a_live_view() {
    let ir = live_arg_ir("const arr = [1, 2];", "arr");
    assert!(ir.contains("__thaw_array_with_native_view"), "{ir}");
    assert!(ir.contains("@thaw_json_from_borrowed_handle("), "{ir}");
    assert!(ir.contains("@thaw_tjs_release_handle("), "{ir}");
}

#[test]
fn thaw_js_method_json_slot_passes_a_native_map_variable_as_a_live_view() {
    let ir = live_arg_ir("const m = new Map<string, number>();", "m");
    assert!(ir.contains("__thaw_collection_with_native_view"), "{ir}");
    assert!(ir.contains("@thaw_json_from_borrowed_handle("), "{ir}");
}

#[test]
fn thaw_js_method_json_slot_keeps_a_fresh_array_literal_as_a_copy() {
    let ir = live_arg_ir("", "[1, 2]");
    assert!(!ir.contains("__thaw_array_with_native_view"), "{ir}");
}
