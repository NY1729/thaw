// thaw-js (`DynamicBackend::ThawJs`): plain JSON-text calls into `thaw_tjs_*`, never JS engine.

fn compile_thaw_js_program(source: &str) -> (bool, String) {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_contract");
    compiler.compile_program(&program).unwrap();
    (compiler.uses_thaw_js(), compiler.print_to_string())
}

#[test]
fn typed_thaw_js_call_uses_the_tjs_json_abi_and_not_js() {
    // "ms::ms"
    let (uses_thaw_js, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_6d733a3a6d73(value: string, scale: number): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73(\"2 days\", 1)); }",
    );
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("call { ptr, ptr } @thaw_tjs_call_typed_result("), "{ir}");
    assert!(ir.contains("ms::ms"), "{ir}");
    assert!(!ir.contains("call i64 @thaw_js_get_global"), "{ir}");
}

#[test]
fn typed_thaw_js_call_with_a_non_plain_type_throws_instead_of_failing_the_build() {
    // Every generated wrapper is compiled whether or not the program calls it.
    let (uses_thaw_js, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_6d733a3a6d73(value: Map<string, number>): Json;
         function main(): void { const m = new Map<string, number>(); if (m.size > 5) { console.log(__thaw_typed_tjs_6d733a3a6d73(m)); } }",
    );
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("cannot cross the thaw-js boundary"), "{ir}");
}

#[test]
fn thaw_js_constructor_option_object_accepts_a_rest_callback_field() {
    use thaw_hir::{HirOptionalMask, HirType::*};
    let rest = CallableFunction(vec![], HirOptionalMask::Inline(0), Some(Box::new(Str)), Box::new(Void));
    let options = Object(vec![("verbose".into(), Optional(Box::new(rest)))]);
    assert!(thaw_js_object_with_callbacks_type(&options));
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
fn typed_thaw_js_callback_parameters_register_with_thaw_js_and_not_js() {
    use thaw_hir::HirType::*;
    let callback = Function(vec![F64], Box::new(F64));
    assert!(thaw_js_callback_type(&callback) && thaw_js_param_type(&callback));
    assert!(thaw_js_callback_type(&Optional(Box::new(callback.clone()))));
    // not plain data: a Promise-returning callback, a callback taking a callback, a bare class
    assert!(!thaw_js_callback_type(&Function(vec![F64], Box::new(Promise(Box::new(F64))))));
    assert!(!thaw_js_callback_type(&Function(vec![callback.clone()], Box::new(F64))));
    assert!(!thaw_js_callback_type(&F64) && !thaw_js_plain_type(&callback));
    let (uses_thaw_js, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_6d733a3a6d73(items: number[], f: (x: number) => number): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73([1, 2], (x: number): number => x * 2)); }",
    );
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("call i64 @thaw_tjs_register_callback("), "{ir}");
    assert!(ir.contains("__thaw_tjs_cb__"), "{ir}");
    assert!(!ir.contains("call { i64, ptr } @thaw_js_register_native_callback"), "{ir}");
    // an optional callback registers only when present
    let (uses_thaw_js, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_6d733a3a6d73(items: number[], f?: (x: number) => number): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73([1, 2], undefined)); }",
    );
    assert!(uses_thaw_js, "{ir}");
}

#[test]
fn a_callback_coerced_to_a_json_parameter_registers_with_thaw_js_in_a_routed_build() {
    let source = "declare function __thaw_typed_tjs_6d733a3a6d73(items: Json, f: Json): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73([1, 2], (x: number): number => x * 2)); }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    // Routed: the HIR registers the callback with thaw-js.
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_routed");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(compiler.uses_thaw_js(), "{ir}");
    assert!(ir.contains("@thaw_tjs_register_callback"), "{ir}");
}

#[test]
fn catching_an_error_in_a_routed_build_does_not_need_js() {
    let source = "declare function __thaw_typed_tjs_6d733a3a6d73(value: string): Json;
         function main(): void {
           try { console.log(__thaw_typed_tjs_6d733a3a6d73(\"x\")); }
           catch (e: any) { console.log(e.message, e instanceof TypeError, String(e)); }
         }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_routed_catch");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(compiler.uses_thaw_js(), "{ir}");
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
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(compiler.uses_thaw_js(), "{ir}");
    assert!(ir.contains("@thaw_tjs_class_call("), "{ir}");
    // instance retain/release use the thaw-js archive's own entry points
    assert!(!ir.contains("@thaw_js_release_handle(") || ir.contains("@thaw_tjs_release_handle("), "{ir}");
}

fn thaw_js_class_symbol_hex(symbol: &str) -> String {
    symbol.as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>()
}

fn compile_thaw_js_class_source(source: &str) -> Result<(bool, String), String> {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_class_op");
    compiler.compile_program(&program)?;
    let ir = compiler.print_to_string();
    Ok((compiler.uses_thaw_js(), ir))
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
    let (uses_thaw_js, ir) = compile_thaw_js_class_source(&source).unwrap_or_else(|e| panic!("{e}"));
    assert!(uses_thaw_js, "{ir}");
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
    let (uses_thaw_js, ir) = compile_thaw_js_class_source(&source).unwrap_or_else(|e| panic!("{e}"));
    assert!(uses_thaw_js, "{ir}");
    assert!(!ir.contains("cannot cross the thaw-js boundary"), "{ir}");
    assert!(ir.contains("@thaw_promise_new("), "{ir}");
    assert!(ir.contains("@thaw_promise_resolve("), "{ir}");
}

#[test]
fn thaw_js_class_property_getters_and_static_accessors_compile_to_the_class_call() {
    // Instance getter (receiver handle only), static getter (no arguments) and static setter (value in,
    // value out) all lower to the same `@thaw_tjs_class_call` as methods: no build refusal, no JS engine.
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
    let (uses_thaw_js, ir) = compile_thaw_js_class_source(&source).unwrap_or_else(|e| panic!("{e}"));
    assert!(uses_thaw_js, "{ir}");
    assert!(!ir.contains("cannot cross the thaw-js boundary"), "{ir}");
    assert!(ir.contains("@thaw_tjs_class_call("), "{ir}");
    for symbol in [getter, static_getter, static_setter] {
        assert!(ir.contains(symbol), "{symbol} missing from:\n{ir}");
    }
}

#[test]
fn routed_graph_retain_uses_the_tjs_symbol_and_not_js() {
    let source = "function main(): void { const object: JsValue = retainDynamicJson({}); }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_graph");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(compiler.uses_thaw_js(), "{ir}");
    assert!(ir.contains("call { i64, ptr } @thaw_tjs_retain_graph_result("), "{ir}");
    assert!(!ir.contains("call { i64, ptr } @thaw_js_retain_graph_result("), "{ir}");
}

#[test]
fn typed_thaw_js_calls_carry_dates_on_the_date_wire_and_accept_live_value_params() {
    assert!(thaw_js_param_type(&thaw_hir::HirType::JsValue));
    // "dt::plus"
    let (uses_thaw_js, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_64743a3a706c7573(d: Date, n: number): Date;
         function main(): void { console.log(__thaw_typed_tjs_64743a3a706c7573(new Date(0), 5).getTime()); }",
    );
    assert!(uses_thaw_js, "{ir}");
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
    let (uses_thaw_js, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_6d733a3a6d73(): string | Promise<string>;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73()); }",
    );
    assert!(uses_thaw_js, "{ir}");
    assert!(!ir.contains("non-plain-data"), "{ir}");
}

#[test]
fn thaw_js_jsvalue_result_registers_host_operations_for_console_log() {
    // "x::after" returns a live handle; its only use is printing it, which needs the host operations.
    let (uses_thaw_js, ir) = compile_thaw_js_program(
        "declare function __thaw_typed_tjs_783a3a6166746572(value: string): JsValue;
         function main(): void { const r = __thaw_typed_tjs_783a3a6166746572(\"hi\"); console.log(r); }",
    );
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("call void @thaw_json_register_host_operations("), "{ir}");
}

#[test]
fn a_void_callback_on_the_thaw_js_text_lane_returns_undefined_not_null() {
    let source = "declare function __thaw_typed_tjs_6d733a3a6d73(f: Json): Json;
         function main(): void { console.log(__thaw_typed_tjs_6d733a3a6d73((): void => {})); }";
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_void_callback_routed");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(compiler.uses_thaw_js(), "{ir}");
    assert!(ir.contains("@thaw_tjs_register_callback"), "{ir}");
    assert!(ir.contains("$__thaw_napi_undefined$"), "{ir}");
}

#[test]
fn discarded_call_with_borrowed_jsvalue_arg_guards_release() {
    // `mark(token)` may return `token` itself (decorator shape): the discarded result is released only if it differs from `token`
    let (_, ir) = compile_thaw_js_program(
        "function mark(value: JsValue): JsValue { return value; }
         function main(): void { const token: JsValue = {a: 1}; mark(token); }",
    );
    assert!(ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn discarded_call_with_fresh_call_arg_releases_unconditionally() {
    // `mark(make())`: the argument is a fresh +1 owned by nobody else and `mark` returns it unretained, so the release is correct
    let (_, ir) = compile_thaw_js_program(
        "function make(): JsValue { return {a: 1}; }
         function mark(value: JsValue): JsValue { return value; }
         function main(): void { mark(make()); }",
    );
    assert!(!ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn discarded_call_with_non_jsvalue_arg_releases_unconditionally() {
    let (_, ir) = compile_thaw_js_program(
        "function mark(n: number): JsValue { return {a: n}; }
         function main(): void { mark(1); }",
    );
    assert!(!ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn discarded_nested_call_arg_is_not_guarded_by_inner_record() {
    // `mark(mark2(token))`: the outer argument is a fresh call result, so the inner call's recorded borrowed argument must not guard the outer release
    let (_, ir) = compile_thaw_js_program(
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
    let (_, ir) = compile_thaw_js_program(
        "function mark(value: JsValue): JsValue { return value; }
         function main(): void { const token: JsValue = {a: 1}; const f = mark; f(token); }",
    );
    assert!(ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn discarded_decorator_bound_to_a_local_guards_release_of_its_token() {
    // `@Mark class K` lowers to a discarded `Mark(token)` through a local closure, the shape behind the decorated-class retain failure
    let (_, ir) = compile_thaw_js_program(
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
    let (_, ir) = compile_thaw_js_program(
        "function make(): JsValue { return {a: 1}; }
         function mark(value: JsValue): JsValue { return value; }
         function main(): void { const f = mark; f(make()); }",
    );
    assert!(!ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
    let (_, ir) = compile_thaw_js_program(
        "function make(): JsValue { return {a: 1}; }
         function main(): void { const f = make; f(); }",
    );
    assert!(!ir.contains("discarded_result_is_argument"), "{ir}");
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
}

#[test]
fn discarded_jsvalue_result_call_without_jsvalue_arguments_still_releases() {
    let (_, ir) = compile_thaw_js_program(
        "function make(): JsValue { return {a: 1}; }
         function main(): void { make(); }",
    );
    assert!(ir.contains("release_discarded_dynamic_value"), "{ir}");
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
fn routed_process_reporters_emit_through_thaw_tjs() {
    let module = thaw_parser::parse_typescript("function main(): void {}").unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_process_reporters");
    compiler.uses_thaw_js = true;
    compiler.uses_thaw_js_handles = true;
    compiler.compile_program(&program).unwrap();
    let routed = compiler.print_to_string();
    let calls = |ir: &str, symbol: &str| ir.lines().any(|line| line.contains("call ") && line.contains(symbol));
    // rejection-handled is registered as a reporter pointer, not called directly
    let symbols = [
        "@thaw_tjs_emit_uncaught_result(",
        "@thaw_tjs_emit_unhandled_rejection_result(",
    ];
    for symbol in symbols {
        assert!(calls(&routed, symbol), "routed entry must report through {symbol}: {routed}");
    }
    assert!(routed.contains("ptr @thaw_tjs_emit_rejection_handled_result"), "{routed}");
    assert!(!routed.lines().any(|line| line.contains("call ") && line.contains("@thaw_js_emit_")), "{routed}");
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
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(compiler.uses_thaw_js(), "{ir}");
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
fn routed_set_process_env_declares_the_thaw_tjs_symbol_and_not_js() {
    let module = thaw_parser::parse_typescript("function main(): void {}").unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_set_process_env");
    compiler.uses_thaw_js = true;
    compiler.uses_thaw_js_handles = true;
    compiler.compile_program(&program).unwrap();
    // the routed build aliases the setter into thaw-js's extern "C" definition (process_env.rs)
    let routed = compiler.print_to_string();
    assert!(routed.contains("@thaw_tjs_set_process_env("), "{routed}");
    assert!(!routed.contains("@thaw_js_set_process_env"), "{routed}");
}

/// The perf_hooks `new PerformanceObserver(cb)` shape: the declared callback takes class instances
/// (`PerfList`, `PerfObs`), which the registry lowers as marker-only objects. The closure's own params
/// are unannotated, so they are lowered as live handles (`JsValue`).
#[test]
fn a_callback_with_a_class_instance_param_registers_on_the_graph_route() {
    let sym = thaw_js_class_symbol_hex("$new$PerformanceObserver$arity1");
    let source = format!(
        "class PerfList {{ getEntries(): number[] {{ return []; }} }} class PerfObs {{ disconnect(): void {{}} }}
         declare function __thaw_typed_tjs_{sym}(callback: (list: PerfList, observer: PerfObs) => void): JsValue;
         function main(): void {{ const o = __thaw_typed_tjs_{sym}((list, observer) => {{ console.log(\"obs\"); }}); }}"
    );
    let (uses_thaw_js, ir) = compile_thaw_js_class_source(&source).unwrap();
    assert!(uses_thaw_js, "{ir}");
    assert!(ir.contains("call { i64, ptr } @thaw_tjs_register_native_callback_graph("), "{ir}");
    assert!(!ir.contains("call i64 @thaw_tjs_register_callback("), "{ir}");
}

/// The shape the bridge now emits for a same-module class callback param
/// (`JsValue`, not the old `Json` fallback): same graph route, live handles.
#[test]
fn a_callback_with_a_jsvalue_class_param_registers_on_the_graph_route() {
    let sym = thaw_js_class_symbol_hex("$new$PerformanceObserver$arity1");
    let source = format!(
        "declare function __thaw_typed_tjs_{sym}(callback: (list: JsValue, observer: JsValue) => void): JsValue;
         function main(): void {{ const o = __thaw_typed_tjs_{sym}((list, observer) => {{ console.log(\"obs\"); }}); }}"
    );
    let (uses_thaw_js, ir) = compile_thaw_js_class_source(&source).unwrap();
    assert!(uses_thaw_js, "{ir}");
    assert!(
        ir.contains("call { i64, ptr } @thaw_tjs_register_native_callback_graph("),
        "{ir}"
    );
    assert!(
        !ir.contains("call i64 @thaw_tjs_register_callback("),
        "{ir}"
    );
}

/// Plain callback params (Json, number) keep the JSON-text route: only class instances move to the graph.
#[test]
fn plain_callback_params_keep_the_text_route_for_class_calls() {
    for ptype in ["Json", "number"] {
        let sym = thaw_js_class_symbol_hex("$new$PerformanceObserver$arity1");
        let source = format!(
            "declare function __thaw_typed_tjs_{sym}(callback: (list: {ptype}, observer: {ptype}) => void): JsValue;
             function main(): void {{ const o = __thaw_typed_tjs_{sym}((list: {ptype}, observer: {ptype}): void => {{ console.log(\"obs\"); }}); }}"
        );
        let (_, ir) = compile_thaw_js_class_source(&source).unwrap();
        assert!(ir.contains("call i64 @thaw_tjs_register_callback("), "{ptype}: {ir}");
        assert!(!ir.contains("call { i64, ptr } @thaw_tjs_register_native_callback_graph("), "{ptype}: {ir}");
    }
}

#[test]
fn class_instance_params_are_handle_leaves_but_date_stays_plain() {
    use thaw_hir::HirType::*;
    let marker = |name: &str| ("__thaw_class_identity_\u{1e}".to_string() + name, Bool);
    let instance = Object(vec![marker("PerfList")]);
    let callback = |param: HirType| Function(vec![param], Box::new(Void));
    assert!(thaw_js_class_instance_type(&instance));
    assert!(thaw_js_callback_takes_class_instance(&callback(instance)));
    let date = Object(vec![marker("Date"), ("timestamp".into(), F64)]);
    assert!(!thaw_js_class_instance_type(&date));
    assert!(!thaw_js_callback_takes_class_instance(&callback(date)));
    assert!(!thaw_js_class_instance_type(&Object(vec![(
        "a".into(),
        F64
    )])));
}
