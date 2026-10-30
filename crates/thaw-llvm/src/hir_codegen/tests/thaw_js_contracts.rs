// thaw-js (`DynamicBackend::ThawJs`): plain JSON-text calls into `thaw_tjs_*`, never QuickJS.

fn compile_thaw_js_program(source: &str) -> (bool, bool, String) {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_contract");
    compiler.compile_program(&program).unwrap();
    (compiler.uses_thaw_js(), compiler.uses_quickjs(), compiler.print_to_string())
}

#[test]
fn typed_thaw_js_call_uses_the_tjs_json_abi_and_not_quickjs() {
    // "ms::ms"
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_6d733a3a6d73(value: string, scale: number): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73(\"2 days\", 1)); }",
    );
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("call { ptr, ptr } @thaw_tjs_call_typed_result("), "{ir}");
    assert!(ir.contains("ms::ms"), "{ir}");
    assert!(!ir.contains("call i64 @thaw_js_get_global"), "{ir}");
}

#[test]
fn typed_thaw_js_call_with_a_non_plain_type_throws_instead_of_failing_the_build() {
    // Every generated wrapper is compiled whether or not the program calls it.
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_6d733a3a6d73(value: Buffer): Json;
         function main(): void { const b = Buffer.from([1]); if (b.length > 5) { console.log(__thaw_typed_tjs_6d733a3a6d73(b)); } }",
    );
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("cannot cross the thaw-js boundary"), "{ir}");
}

#[test]
fn typed_thaw_js_plain_types_are_numbers_strings_booleans_json_and_their_shapes() {
    use thaw_hir::HirType::*;
    assert!(thaw_js_plain_type(&F64) && thaw_js_plain_type(&Str) && thaw_js_plain_type(&Bool));
    assert!(thaw_js_plain_type(&Json) && thaw_js_plain_type(&Null) && thaw_js_plain_type(&Undefined));
    assert!(thaw_js_plain_type(&Array(Box::new(Str))));
    assert!(thaw_js_plain_type(&Optional(Box::new(Array(Box::new(F64))))));
    assert!(thaw_js_plain_type(&Union(vec![Str, Bool])));
    assert!(thaw_js_plain_type(&Object(vec![("a".into(), F64)])));
    assert!(!thaw_js_plain_type(&JsValue) && !thaw_js_plain_type(&Bytes));
    assert!(!thaw_js_plain_type(&Function(vec![F64], Box::new(F64))));
    assert!(!thaw_js_plain_type(&Array(Box::new(JsValue))));
}

#[test]
fn typed_thaw_js_callback_parameters_register_with_thaw_js_and_not_quickjs() {
    use thaw_hir::HirType::*;
    let callback = Function(vec![F64], Box::new(F64));
    assert!(thaw_js_callback_type(&callback) && thaw_js_param_type(&callback));
    assert!(thaw_js_callback_type(&Optional(Box::new(callback.clone()))));
    // not plain data: a Promise-returning callback, a callback taking a callback, a bare class
    assert!(!thaw_js_callback_type(&Function(vec![F64], Box::new(Promise(Box::new(F64))))));
    assert!(!thaw_js_callback_type(&Function(vec![callback.clone()], Box::new(F64))));
    assert!(!thaw_js_callback_type(&F64) && !thaw_js_plain_type(&callback));
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_6d733a3a6d73(items: number[], f: (x: number) => number): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73([1, 2], (x: number): number => x * 2)); }",
    );
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("call i64 @thaw_tjs_register_callback("), "{ir}");
    assert!(ir.contains("__thaw_tjs_cb__"), "{ir}");
    assert!(!ir.contains("call { i64, ptr } @thaw_js_register_native_callback"), "{ir}");
    // an optional callback registers only when present
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_6d733a3a6d73(items: number[], f?: (x: number) => number): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73([1, 2], undefined)); }",
    );
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
}

#[test]
fn a_callback_coerced_to_a_json_parameter_registers_with_thaw_js_in_a_routed_build() {
    let source = "declare function __thaw_typed_tjs_6d733a3a6d73(items: Json, f: Json): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73([1, 2], (x: number): number => x * 2)); }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    // Unrouted: the callback is a QuickJS value.
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_unrouted");
    compiler.compile_program(&program).unwrap();
    assert!(compiler.uses_quickjs());
    // Routed: the same HIR registers it with thaw-js.
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_routed");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(compiler.uses_thaw_js() && !compiler.uses_quickjs(), "{ir}");
    assert!(ir.contains("@thaw_tjs_register_callback"), "{ir}");
}

#[test]
fn catching_an_error_in_a_routed_build_does_not_need_quickjs() {
    let source = "declare function __thaw_typed_tjs_6d733a3a6d73(value: string): Json;
         function main(): void {
           try { console.log(__thaw_typed_tjs_6d733a3a6d73(\"x\")); }
           catch (e: any) { console.log(e.message, e instanceof TypeError, String(e)); }
         }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_unrouted_catch");
    compiler.compile_program(&program).unwrap();
    assert!(compiler.uses_quickjs(), "the unrouted catch adapter keeps the QuickJS projections");
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_routed_catch");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(compiler.uses_thaw_js() && !compiler.uses_quickjs(), "{ir}");
}

#[test]
fn thaw_js_class_operations_call_the_class_abi_and_keep_instances_as_handles() {
    // "$new$k::Box$arity1", "$method$k::Box$get$overload0$arity0"
    let source = "declare function __thaw_typed_tjs_246e6577246b3a3a426f7824617269747931(v: number): JsValue;
         declare function __thaw_typed_tjs_246d6574686f64246b3a3a426f78246765742e6f7665726c6f61643024617269747930(receiver: JsValue): number;
         function main(): void {
           const box = __thaw_typed_tjs_246e6577246b3a3a426f7824617269747931(1);
           console.log(__thaw_typed_tjs_246d6574686f64246b3a3a426f78246765742e6f7665726c6f61643024617269747930(box));
         }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_class");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(compiler.uses_thaw_js() && !compiler.uses_quickjs(), "{ir}");
    assert!(ir.contains("@thaw_tjs_class_call("), "{ir}");
    // instance retain/release use the thaw-js archive's own entry points
    assert!(!ir.contains("@thaw_js_release_handle(") || ir.contains("@thaw_tjs_release_handle("), "{ir}");
}

#[test]
fn thaw_js_class_properties_are_not_class_operations() {
    // A property write has no thaw-js operation: compiling one is an error the CLI answers by
    // rebuilding the program on QuickJS.
    let source = format!(
        "declare function __thaw_typed_tjs_{}(receiver: JsValue, value: number): number;
         declare function __thaw_typed_tjs_{}(v: number): JsValue;
         function main(): void {{
           const box = __thaw_typed_tjs_{}(1);
           console.log(__thaw_typed_tjs_{}(box, 2));
         }}",
        "$setter$k::Box$v".as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "$new$k::Box$arity1".as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "$new$k::Box$arity1".as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "$setter$k::Box$v".as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>(),
    );
    let module = thaw_parser::parse_typescript(&source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_class_setter");
    compiler.set_thaw_js_routed(true);
    let error = compiler.compile_program(&program).unwrap_err();
    assert!(error.contains("cannot cross the thaw-js boundary"), "{error}");
}

#[test]
fn routed_graph_retain_uses_the_tjs_symbol_and_not_quickjs() {
    let source = "function main(): void { const object: JsValue = retainDynamicJson({}); }";
    let compile = |routed: bool| {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "thaw_js_graph");
        compiler.set_thaw_js_routed(routed);
        compiler.compile_program(&program).unwrap();
        (compiler.uses_thaw_js(), compiler.uses_quickjs(), compiler.print_to_string())
    };
    let (uses_thaw_js, _, ir) = compile(true);
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("call { i64, ptr } @thaw_tjs_retain_graph_result("), "{ir}");
    assert!(!ir.contains("call { i64, ptr } @thaw_js_retain_graph_result("), "{ir}");
    let (_, uses_quickjs, ir) = compile(false);
    assert!(uses_quickjs, "{ir}");
    assert!(ir.contains("call { i64, ptr } @thaw_js_retain_graph_result("), "{ir}");
}
