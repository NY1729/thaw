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
        "declare function __thaw_typed_tjs_6d733a3a6d73(value: Map<string, number>): Json;
         function main(): void { const m = new Map<string, number>(); if (m.size > 5) { console.log(__thaw_typed_tjs_6d733a3a6d73(m)); } }",
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
    assert!(!thaw_js_plain_type(&JsValue));
    assert!(thaw_js_plain_type(&Bytes));
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

#[test]
fn typed_thaw_js_calls_carry_dates_on_the_date_wire_and_accept_live_value_params() {
    assert!(thaw_js_param_type(&thaw_hir::HirType::JsValue));
    // "dt::plus"
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_64743a3a706c7573(d: Date, n: number): Date;
         function main(): void { console.log(__thaw_typed_tjs_64743a3a706c7573(new Date(0), 5).getTime()); }",
    );
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    // arguments serialize through the Date-wire stringifier; the Date result is requested as a wire node
    assert!(ir.contains("@thaw_json_stringify_tjs("), "{ir}");
    assert!(ir.contains("\\03dt::plus"), "{ir}");
    assert!(ir.contains("__thaw_tjs_date__"), "{ir}");
}

#[test]
fn thaw_js_param_accepts_optional_dynamic() {
    use thaw_hir::HirType::*;
    assert!(thaw_js_param_type(&Optional(Box::new(Dynamic))));
    assert!(thaw_js_param_type(&Array(Box::new(Dynamic))));
    assert!(!thaw_js_plain_type(&Dynamic));
}

#[test]
fn thaw_js_result_type_accepts_tuple_and_union_with_resolved_promise() {
    use thaw_hir::HirType::*;
    assert!(thaw_js_result_type(&Tuple(vec![Json, Array(Box::new(Json)), Array(Box::new(Json))])));
    assert!(thaw_js_result_type(&Union(vec![Str, Promise(Box::new(Str))])));
    assert!(!thaw_js_result_type(&Union(vec![Str, Promise(Box::new(JsValue))])));
    assert!(!thaw_js_result_type(&Tuple(vec![JsValue])));
    assert!(!thaw_js_result_type(&JsValue));
    assert!(!thaw_js_result_type(&Array(Box::new(JsValue))));
    assert!(!thaw_js_result_type(&Dynamic));
}

#[test]
fn thaw_js_union_with_a_resolved_promise_member_is_a_routed_call_not_a_stub() {
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_6d733a3a6d73(): string | Promise<string>;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73()); }",
    );
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(!ir.contains("non-plain-data"), "{ir}");
}

#[test]
fn thaw_js_jsvalue_result_registers_host_operations_for_console_log() {
    // "x::after" returns a live handle; its only use is printing it, which needs the host operations.
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_783a3a6166746572(value: string): JsValue;
         function main(): void { const r = __thaw_typed_tjs_783a3a6166746572(\"hi\"); console.log(r); }",
    );
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("call void @thaw_json_register_host_operations("), "{ir}");
}

#[test]
fn thaw_js_plain_result_does_not_register_host_operations() {
    // Control for the test above: the same call returning a string holds no handle.
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_783a3a6166746572(value: string): string;
         function main(): void { console.log(__thaw_typed_tjs_783a3a6166746572(\"hi\")); }",
    );
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(!ir.contains("call void @thaw_json_register_host_operations("), "{ir}");
}

#[test]
fn a_void_callback_on_the_thaw_js_text_lane_returns_undefined_not_null() {
    let source = "declare function __thaw_typed_tjs_6d733a3a6d73(f: Json): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73((): void => {})); }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_void_callback_routed");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(compiler.uses_thaw_js() && !compiler.uses_quickjs(), "{ir}");
    assert!(ir.contains("@thaw_tjs_register_callback"), "{ir}");
    assert!(ir.contains("$__thaw_napi_undefined$"), "{ir}");
}

#[test]
fn discarded_call_with_borrowed_jsvalue_arg_guards_release() {
    // `mark(token)` may return `token` itself (decorator shape): the discarded result is released only if it differs from `token`
    let (_, _, ir) = compile_thaw_js_program(
        "function mark(value: JsValue): JsValue { return value; }
         function main(): void { const token: JsValue = {a: 1}; mark(token); }",
    );
    assert!(ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn discarded_call_with_fresh_call_arg_releases_unconditionally() {
    // `mark(make())`: the argument is a fresh +1 owned by nobody else and `mark` returns it unretained, so the release is correct
    let (_, _, ir) = compile_thaw_js_program(
        "function make(): JsValue { return {a: 1}; }
         function mark(value: JsValue): JsValue { return value; }
         function main(): void { mark(make()); }",
    );
    assert!(!ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn discarded_call_with_non_jsvalue_arg_releases_unconditionally() {
    let (_, _, ir) = compile_thaw_js_program(
        "function mark(n: number): JsValue { return {a: n}; }
         function main(): void { mark(1); }",
    );
    assert!(!ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn discarded_nested_call_arg_is_not_guarded_by_inner_record() {
    // `mark(mark2(token))`: the outer argument is a fresh call result, so the inner call's recorded borrowed argument must not guard the outer release
    let (_, _, ir) = compile_thaw_js_program(
        "function mark2(value: JsValue): JsValue { return value; }
         function mark(value: JsValue): JsValue { return value; }
         function main(): void { const token: JsValue = {a: 1}; mark(mark2(token)); }",
    );
    assert!(!ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn discarded_jsvalue_result_call_without_jsvalue_arguments_still_releases() {
    let (_, _, ir) = compile_thaw_js_program(
        "function make(): JsValue { return {a: 1}; }
         function main(): void { make(); }",
    );
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn object_literal_with_jsvalue_in_a_callback_registers_host_operations_before_encoding() {
    // The callback body may be the first dynamic operation on its thread; the literal's live JsValue
    // needs the host operations to be registered before the mixed-handle args are graph-encoded.
    let (_, _, ir) = compile_thaw_js_program(
        "function wrap(v: JsValue): JsValue { return { data: v }; }
         function main(): void { const r = wrap({ a: 1 }); console.log(r); }",
    );
    let body = ir
        .split("\ndefine ")
        .find(|function| function.contains("mixed_handle_args_json"))
        .unwrap_or_else(|| panic!("{ir}"));
    let register = body.find("call void @thaw_json_register_host_operations(").expect(body);
    let encode = body.find("@thaw_json_graph_encode(").expect(body);
    assert!(register < encode, "{body}");
}

#[test]
fn thaw_js_callback_adapter_registers_host_operations_before_graph_decode() {
    // The adapter can run on a thaw-js coroutine thread where the host-operation
    // TLS is unset; it must register before decoding `hdl` wire nodes.
    let source = "declare function __thaw_typed_tjs_6d733a3a6d73(options: JsValue): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73({ adapter: (config: any): string => String(config.url) })); }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_callback_adapter_register");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    let body = ir
        .split("\ndefine ")
        .find(|function| function.split('(').next().unwrap_or("").contains("@__thaw_napi_value_callback_"))
        .unwrap_or_else(|| panic!("{ir}"));
    let register = body.find("call void @thaw_json_register_host_operations(").expect(body);
    let decode = body.find("@thaw_json_graph_decode(").expect(body);
    assert!(register < decode, "{body}");
}

#[test]
fn async_throw_of_a_jsvalue_records_its_handle_id_before_the_tag_seven_store() {
    // The tag-7 packet carries the live handle id in its i64 slot, so a Promise hop on another
    // thread can still resolve the thrown object.
    let source = "declare function __thaw_typed_tjs_783a3a6166746572(value: string): JsValue;
         async function fail(): Promise<void> { const r = __thaw_typed_tjs_783a3a6166746572(\"hi\"); throw r; }
         function main(): void { fail(); }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_async_throw_handle_id");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    let lines: Vec<&str> = ir.lines().collect();
    let handle_call = lines
        .iter()
        .position(|line| line.contains("call i64 @thaw_json_borrowed_handle_id("))
        .unwrap_or_else(|| panic!("{ir}"));
    let handle_name = lines[handle_call].split(" = ").next().unwrap().trim();
    let id_store = lines
        .iter()
        .position(|line| line.contains(&format!("store i64 {handle_name}, ptr @__thaw_pending_exception_i64")))
        .unwrap_or_else(|| panic!("{ir}"));
    let tag_store = lines
        .iter()
        .position(|line| line.contains("store i64 7, ptr @__thaw_pending_exception_value_tag"))
        .unwrap_or_else(|| panic!("{ir}"));
    assert!(handle_call < id_store && id_store < tag_store, "{ir}");
}
