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

fn thaw_js_class_symbol_hex(symbol: &str) -> String {
    symbol.as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>()
}

fn compile_thaw_js_class_source(source: &str) -> Result<(bool, bool, String), String> {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_class_op");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program)?;
    let ir = compiler.print_to_string();
    Ok((compiler.uses_thaw_js(), compiler.uses_quickjs(), ir))
}

#[test]
fn thaw_js_class_setters_are_class_operations_and_not_build_errors() {
    // `receiver[name] = value` (`[JsValue, Str] -> Str`): the same class-call ABI as a method
    let setter = thaw_js_class_symbol_hex("$setter$k::Box$v$overload0$arity1");
    let new = thaw_js_class_symbol_hex("$new$k::Box$arity0");
    let source = format!(
        "declare function __thaw_typed_tjs_{setter}(receiver: JsValue, value: string): string;
         declare function __thaw_typed_tjs_{new}(): JsValue;
         function main(): void {{
           const box = __thaw_typed_tjs_{new}();
           console.log(__thaw_typed_tjs_{setter}(box, \"b\"));
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_class_source(&source).unwrap_or_else(|e| panic!("{e}"));
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(!ir.contains("cannot cross the thaw-js boundary"), "{ir}");
    assert!(ir.contains("@thaw_tjs_class_call("), "{ir}");
    assert!(ir.contains("$setter$k::Box$v$overload0$arity1"), "{ir}");
}

#[test]
fn thaw_js_class_promise_of_a_handle_wraps_the_handle_in_a_resolved_promise() {
    // `later(n): Promise<JsValue>` on an instance: the host awaits it and returns the value's handle
    let later = thaw_js_class_symbol_hex("$method$k::Box$later$overload0$arity1");
    let new = thaw_js_class_symbol_hex("$new$k::Box$arity0");
    let source = format!(
        "declare function __thaw_typed_tjs_{later}(receiver: JsValue, n: number): Promise<JsValue>;
         declare function __thaw_typed_tjs_{new}(): JsValue;
         function main(): void {{
           const box = __thaw_typed_tjs_{new}();
           __thaw_typed_tjs_{later}(box, 1);
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_class_source(&source).unwrap_or_else(|e| panic!("{e}"));
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(!ir.contains("cannot cross the thaw-js boundary"), "{ir}");
    assert!(ir.contains("@thaw_promise_new("), "{ir}");
    assert!(ir.contains("@thaw_promise_resolve("), "{ir}");
}

#[test]
fn thaw_js_class_property_getters_and_static_accessors_compile_to_the_class_call() {
    // Instance getter (receiver handle only), static getter (no arguments) and static setter (value in,
    // value out) all lower to the same `@thaw_tjs_class_call` as methods: no build refusal, no QuickJS.
    let getter = "$getter$k::Box$v";
    let static_getter = "$staticgetter$k::Box$w";
    let static_setter = "$staticsetter$k::Box$w";
    let new = thaw_js_class_symbol_hex("$new$k::Box$arity0");
    let source = format!(
        "declare function __thaw_typed_tjs_{getter_hex}(receiver: JsValue): number;
         declare function __thaw_typed_tjs_{static_getter_hex}(): number;
         declare function __thaw_typed_tjs_{static_setter_hex}(value: number): number;
         declare function __thaw_typed_tjs_{new}(): JsValue;
         function main(): void {{
           const box = __thaw_typed_tjs_{new}();
           console.log(__thaw_typed_tjs_{getter_hex}(box));
           console.log(__thaw_typed_tjs_{static_setter_hex}(3));
           console.log(__thaw_typed_tjs_{static_getter_hex}());
         }}",
        getter_hex = thaw_js_class_symbol_hex(getter),
        static_getter_hex = thaw_js_class_symbol_hex(static_getter),
        static_setter_hex = thaw_js_class_symbol_hex(static_setter),
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_thaw_js_class_source(&source).unwrap_or_else(|e| panic!("{e}"));
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(!ir.contains("cannot cross the thaw-js boundary"), "{ir}");
    assert!(ir.contains("@thaw_tjs_class_call("), "{ir}");
    for symbol in [getter, static_getter, static_setter] {
        assert!(ir.contains(symbol), "{symbol} missing from:\n{ir}");
    }
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
fn discarded_closure_variable_call_with_borrowed_jsvalue_arg_guards_release() {
    // `const f = mark; f(token)` goes through `compile_closure_call`, which must record the borrowed argument like a direct call
    let (_, _, ir) = compile_thaw_js_program(
        "function mark(value: JsValue): JsValue { return value; }
         function main(): void { const token: JsValue = {a: 1}; const f = mark; f(token); }",
    );
    assert!(ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn discarded_decorator_bound_to_a_local_guards_release_of_its_token() {
    // `@Mark class K` lowers to a discarded `Mark(token)` through a local closure, the shape behind the decorated-class retain failure
    let (_, _, ir) = compile_thaw_js_program(
        "function Mark(value: JsValue): JsValue { return value; }
         @Mark class K {}
         function main(): void { console.log(new K()); }",
    );
    assert!(ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn discarded_closure_variable_call_with_fresh_or_no_jsvalue_args_releases_unconditionally() {
    // Negative control: a fresh call-result argument and a call with no JsValue argument are not borrowed, so no guard
    let (_, _, ir) = compile_thaw_js_program(
        "function make(): JsValue { return {a: 1}; }
         function mark(value: JsValue): JsValue { return value; }
         function main(): void { const f = mark; f(make()); }",
    );
    assert!(!ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
    let (_, _, ir) = compile_thaw_js_program(
        "function make(): JsValue { return {a: 1}; }
         function main(): void { const f = make; f(); }",
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
fn thaw_js_graph_callback_falls_back_to_the_plain_entry_when_this_entry_is_null() {
    // Rust-built closures (thaw-std NativeClosure) have a null this-entry at offset 8;
    // the adapter must call their plain entry at offset 0 instead of jumping to null.
    let source = "declare function __thaw_typed_tjs_6d733a3a6d73(options: JsValue): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73({ adapter: (config: any): string => String(config.url) })); }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_callback_null_this_entry");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    let body = ir
        .split("\ndefine ")
        .find(|function| function.split('(').next().unwrap_or("").contains("@__thaw_napi_value_callback_") && function.contains("callback_holder_word"))
        .unwrap_or_else(|| panic!("{ir}"));
    assert!(body.contains("%napi_value_callback_this_entry_missing = icmp eq ptr"), "{body}");
    assert!(body.contains("%invoke_napi_value_callback_plain = call"), "{body}");
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

#[test]
fn routed_process_reporters_emit_through_thaw_tjs_and_quickjs_keeps_its_symbols() {
    let compile = |quickjs: bool| {
        let module = thaw_parser::parse_typescript("function main(): void {}").unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "thaw_js_process_reporters");
        if quickjs {
            compiler.uses_quickjs = true;
            compiler.uses_quickjs_handles = true;
        } else {
            compiler.uses_thaw_js = true;
            compiler.uses_thaw_js_handles = true;
            compiler.set_thaw_js_routed(true);
        }
        compiler.compile_program(&program).unwrap();
        compiler.print_to_string()
    };
    let calls = |ir: &str, symbol: &str| ir.lines().any(|line| line.contains("call ") && line.contains(symbol));
    // rejection-handled is registered as a reporter pointer, not called directly
    let symbols = [
        "@thaw_tjs_emit_uncaught_result(",
        "@thaw_tjs_emit_unhandled_rejection_result(",
    ];

    let routed = compile(false);
    for symbol in symbols {
        assert!(calls(&routed, symbol), "routed entry must report through {symbol}: {routed}");
    }
    assert!(routed.contains("ptr @thaw_tjs_emit_rejection_handled_result"), "{routed}");
    assert!(!routed.lines().any(|line| line.contains("call ") && line.contains("@thaw_js_emit_")), "{routed}");

    let quickjs = compile(true);
    assert!(calls(&quickjs, "@thaw_js_emit_uncaught_result("), "{quickjs}");
    assert!(calls(&quickjs, "@thaw_js_emit_unhandled_rejection_result("), "{quickjs}");
    assert!(quickjs.contains("ptr @thaw_js_emit_rejection_handled_result"), "{quickjs}");
    assert!(!quickjs.lines().any(|line| line.contains("call ") && line.contains("@thaw_tjs_emit_")), "{quickjs}");
}

#[test]
fn every_unported_thaw_js_symbol_in_use_is_listed_as_a_quickjs_blocker() {
    // Two thaw-js-named functions; only one is called, so only that one may be listed.
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_unported_blockers");
    let void_fn = context.void_type().fn_type(&[], false);
    let used = compiler.module.add_function("thaw_js_call_result", void_fn, None);
    compiler.module.add_function("thaw_js_fatal_exit", void_fn, None);
    let probe = compiler.module.add_function("q9_probe", void_fn, None);
    let entry = context.append_basic_block(probe, "entry");
    compiler.builder.position_at_end(entry);
    compiler.builder.build_call(used, &[], "").unwrap();
    compiler.builder.build_return(None).unwrap();
    compiler.js_unported_symbols_used();
    assert!(compiler.uses_quickjs());
    assert_eq!(compiler.quickjs_blockers(), ["unported:thaw_js_call_result".to_string()]);
}

#[test]
fn quickjs_site_is_recorded_as_a_blocker_once() {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_site_blockers");
    compiler.note_quickjs("probe:site");
    compiler.note_quickjs("probe:site");
    assert!(compiler.uses_quickjs());
    assert_eq!(compiler.quickjs_blockers(), ["site:probe:site".to_string()]);
}

#[test]
fn routed_typed_quickjs_call_names_its_site_in_the_blockers() {
    let source = "declare function __thaw_typed_js_6d733a3a6d73(value: string, scale: number): Json;
         function main(): void { console.log(__thaw_typed_js_6d733a3a6d73(\"2 days\", 1)); }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_typed_site");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    assert!(compiler.uses_quickjs());
    assert_eq!(
        compiler.quickjs_blockers(),
        ["site:typed_calls:quickjs_backend".to_string()]
    );
}

#[test]
fn routed_async_main_drives_thaw_js_and_releases_its_completion_only_once_settled() {
    let module = thaw_parser::parse_typescript(
        "declare function __thaw_typed_tjs_6d733a3a6d73(value: string, scale: number): Json;
         async function main(): Promise<void> { console.log(__thaw_typed_tjs_6d733a3a6d73(\"2 days\", 1)); }",
    )
    .unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_async_main_release");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(compiler.uses_thaw_js() && !compiler.uses_quickjs(), "{ir}");
    let entry = &ir[ir.find("define i32 @main(").unwrap_or_else(|| panic!("no entry: {ir}"))..];
    let destroy = entry
        .find("@thaw_promise_destroy(")
        .unwrap_or_else(|| panic!("async main never releases its completion: {ir}"));
    let event_loop = entry
        .find("@thaw_tjs_run_event_loop(")
        .unwrap_or_else(|| panic!("routed async main has no thaw-js event loop call: {ir}"));
    assert!(event_loop < destroy, "thaw-js must run before the completion is released: {ir}");
    let guard = entry[..destroy]
        .rfind("@thaw_promise_state(")
        .unwrap_or_else(|| panic!("completion is released without a settled-state check: {ir}"));
    assert!(entry[guard..destroy].contains("br i1"), "the release must be conditional on the state: {ir}");
}

#[test]
fn routed_set_process_env_declares_the_thaw_tjs_symbol_and_not_quickjs() {
    let compile = |routed: bool| {
        let module = thaw_parser::parse_typescript("function main(): void {}").unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "thaw_js_set_process_env");
        if routed {
            compiler.uses_thaw_js = true;
            compiler.uses_thaw_js_handles = true;
            compiler.set_thaw_js_routed(true);
        } else {
            compiler.uses_quickjs = true;
            compiler.uses_quickjs_handles = true;
        }
        compiler.compile_program(&program).unwrap();
        compiler.print_to_string()
    };
    // the routed build aliases the setter into thaw-js's extern "C" definition (process_env.rs)
    let routed = compile(true);
    assert!(routed.contains("@thaw_tjs_set_process_env("), "{routed}");
    assert!(!routed.contains("@thaw_js_set_process_env"), "{routed}");
    let quickjs = compile(false);
    assert!(quickjs.contains("@thaw_js_set_process_env("), "{quickjs}");
}
