// Node builtin shims on thaw-js (`THAW_JS_RUNTIME=1`): callbacks with rest parameters on class
// operations, plain-data value exports and the routed handle-to-Json conversion.

fn hex(key: &str) -> String {
    key.bytes().map(|b| format!("{b:02x}")).collect()
}

fn compile_routed(source: &str, name: &str) -> (bool, bool, String) {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, name);
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    (
        compiler.uses_thaw_js(),
        compiler.uses_quickjs(),
        compiler.print_to_string(),
    )
}

#[test]
fn thaw_js_callback_types_include_rest_parameter_callables() {
    use thaw_hir::HirType::*;
    let rest = CallableFunction(
        vec![],
        Default::default(),
        Some(Box::new(Json)),
        Box::new(Void),
    );
    assert!(thaw_js_callback_type(&rest) && thaw_js_param_type(&rest));
    assert!(thaw_js_callback_type(&Optional(Box::new(rest.clone()))));
    let fixed = CallableFunction(vec![F64], Default::default(), None, Box::new(Void));
    assert!(thaw_js_callback_type(&fixed));
    // a rest of non-plain elements, or a non-plain result, still stays on QuickJS
    assert!(!thaw_js_callback_type(&CallableFunction(
        vec![],
        Default::default(),
        Some(Box::new(JsValue)),
        Box::new(Void)
    )));
    assert!(!thaw_js_callback_type(&CallableFunction(
        vec![],
        Default::default(),
        None,
        Box::new(Promise(Box::new(F64)))
    )));
}

#[test]
fn routed_class_method_takes_a_rest_callback_and_registers_a_closure_identity() {
    let new = hex("$new$k::Emitter$arity0");
    let on = hex("$method$k::Emitter$on$overload0$arity2");
    let source = format!(
        "declare function __thaw_typed_tjs_{new}(): JsValue;
         declare function __thaw_typed_tjs_{on}(receiver: JsValue, event: Json, listener: (...args: Json[]) => void): JsValue;
         function main(): void {{
           const emitter = __thaw_typed_tjs_{new}();
           __thaw_typed_tjs_{on}(emitter, \"x\", (...args: Json[]): void => {{ console.log(args.length); }});
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_class_rest_callback");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("@thaw_tjs_class_call("), "{ir}");
    assert!(ir.contains("__thaw_tjs_cb__"), "{ir}");
    // the same closure registered for `on` and `off` is one callback
    assert!(ir.contains("@thaw_tjs_callback_identity("), "{ir}");
    assert!(ir.contains("call i64 @thaw_tjs_register_callback("), "{ir}");
}

#[test]
fn routed_plain_value_export_is_a_class_call_returning_plain_data() {
    let eol = hex("$value$k::EOL$arity0");
    let source = format!(
        "declare function __thaw_typed_tjs_{eol}(): string;
         let eol: string = __thaw_typed_tjs_{eol}();
         function main(): void {{ console.log(eol.length); }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_plain_value");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("@thaw_tjs_class_call("), "{ir}");
}

#[test]
fn routed_live_handle_to_json_conversion_captures_the_graph_instead_of_an_empty_object() {
    let source = "function main(): void {
           const handle: JsValue = retainDynamicJson({ a: 1 });
           const copy: Json = __thaw_json_host_from_dynamic(handle);
           console.log(copy);
         }";
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(source, "thaw_js_host_json");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(
        ir.contains("@thaw_tjs_capture_handle_graph_result("),
        "{ir}"
    );
    assert!(!ir.contains("routed_no_host_json"), "{ir}");
}
