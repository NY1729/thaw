// Q9 batch 2 Stage 1: a thaw-js constructor whose option object carries callbacks (`new Writable({ write })`)
// receives the object as a live `__thaw_dynamic_object_literal` call, so its callbacks become receiver handles.
// Constructor-only scope: plain-data option objects and `$method$` symbols keep their typed lowering.

fn stream_ctor_hex(key: &str) -> String {
    key.bytes().map(|b| format!("{b:02x}")).collect()
}

fn stream_ctor_lowered_dump(source: &str) -> String {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    format!("{program:?}")
}

fn stream_ctor_routed_build(source: &str) -> Result<(bool, bool, String), String> {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "thaw_js_stream_ctor");
    compiler.set_thaw_js_routed(true);
    compiler.compile_program(&program)?;
    Ok((
        compiler.uses_thaw_js(),
        compiler.uses_quickjs(),
        compiler.print_to_string(),
    ))
}

const STREAM_WRITABLE_CTOR: &str = "$new$node:stream::Writable$arity1";

fn stream_writable_source(argument: &str) -> String {
    format!(
        "declare function __thaw_typed_tjs_{run}(opts: {{ write: ((arg0: JsValue, arg1: string, arg2: () => void) => void) | undefined }}): JsValue;
         function main(): void {{
           __thaw_typed_tjs_{run}({argument});
         }}",
        run = stream_ctor_hex(STREAM_WRITABLE_CTOR),
    )
}

#[test]
fn writable_ctor_object_literal_is_dynamic_object_literal_call() {
    let dump = stream_ctor_lowered_dump(&stream_writable_source("{ write(c, e, cb) { cb(); } }"));
    assert!(
        dump.contains("Var(\"__thaw_dynamic_object_literal\")"),
        "the argument is not a __thaw_dynamic_object_literal call; lowered program: {dump}"
    );
    assert!(
        !dump.contains("JsonAsNative"),
        "the marker is wrapped in a Json->Object decode: {dump}"
    );
}

#[test]
fn plain_data_ctor_object_literal_stays_typed() {
    let source = format!(
        "declare function __thaw_typed_tjs_{run}(opts: {{ n: number }}): JsValue;
         function main(): void {{
           __thaw_typed_tjs_{run}({{ n: 1 }});
         }}",
        run = stream_ctor_hex(STREAM_WRITABLE_CTOR),
    );
    let dump = stream_ctor_lowered_dump(&source);
    assert!(
        !dump.contains("__thaw_dynamic_object_literal"),
        "a plain-data option object must keep its typed lowering: {dump}"
    );
}

#[test]
fn method_symbol_callback_object_literal_stays_typed() {
    let method = stream_ctor_hex("$method$node:stream::Writable$write$arity1");
    let source = format!(
        "declare function __thaw_typed_tjs_{method}(opts: {{ write: ((arg0: JsValue, arg1: string, arg2: () => void) => void) | undefined }}): JsValue;
         function main(): void {{
           __thaw_typed_tjs_{method}({{ write(c, e, cb) {{ cb(); }} }});
         }}",
    );
    let dump = stream_ctor_lowered_dump(&source);
    assert!(
        !dump.contains("__thaw_dynamic_object_literal"),
        "a $method$ symbol is outside the constructor scope: {dump}"
    );
}

#[test]
fn thaw_js_routed_ctor_callback_object_literal_is_json_with_handles() {
    let (uses_thaw_js, uses_quickjs, ir) =
        stream_ctor_routed_build(&stream_writable_source("{ write(c, e, cb) { cb(); } }"))
            .expect("the Writable option object builds on thaw-js");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(
        ir.contains("call { i64, ptr } @thaw_tjs_register_native_method_callback_graph("),
        "{ir}"
    );
    assert!(!ir.contains("call { i64, ptr } @thaw_tjs_register_native_callback_graph("), "{ir}");
    assert!(!ir.contains("__thaw_object_from_operations"), "{ir}");
    assert!(!ir.contains("cannot cross the thaw-js boundary"), "{ir}");
}

#[test]
fn thaw_js_routed_ctor_object_variable_is_a_loud_error() {
    let source = format!(
        "declare function __thaw_typed_tjs_{run}(opts: {{ write: ((arg0: JsValue, arg1: string, arg2: () => void) => void) | undefined }}): JsValue;
         function main(): void {{
           const options: {{ write: ((arg0: JsValue, arg1: string, arg2: () => void) => void) | undefined }} = {{ write: undefined }};
           __thaw_typed_tjs_{run}(options);
         }}",
        run = stream_ctor_hex(STREAM_WRITABLE_CTOR),
    );
    let error = stream_ctor_routed_build(&source).expect_err("a variable option object is rejected");
    assert!(error.contains("must be an object literal"), "{error}");
}

#[test]
fn thaw_js_routed_ctor_arrow_callback_object_literal_is_json_with_handles() {
    let (uses_thaw_js, uses_quickjs, ir) = stream_ctor_routed_build(&stream_writable_source(
        "{ write: (c, e, cb) => { cb(); } }",
    ))
    .expect("the arrow-valued Writable option object builds on thaw-js");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    // an arrow property is a data member holding a closure: plain graph ABI, not a method handle
    assert!(ir.contains("call { i64, ptr } @thaw_tjs_register_native_callback_graph("), "{ir}");
    assert!(!ir.contains("call { i64, ptr } @thaw_tjs_register_native_method_callback_graph("), "{ir}");
    assert!(!ir.contains("cannot cross the thaw-js boundary"), "{ir}");
}

// Transform's option callbacks take a nested `callback: (error?: any, data?: Json) => void`, which lowers to an
// optional-parameter CallableFunction: it must cross as a nested callback like Writable's `cb`.
const STREAM_TRANSFORM_CTOR: &str = "$new$node:stream::Transform$arity1$overload1";
const TRANSFORM_FN: &str = "((arg0: JsValue, arg1: string, arg2: (error?: any, data?: Json) => void) => void) | undefined";
const FLUSH_FN: &str = "((callback: (error?: any, data?: Json) => void) => void) | undefined";

fn stream_transform_source(decl_symbol: &str, members: &str, decl_members: &str) -> String {
    format!(
        "declare function __thaw_typed_tjs_{run}(opts: {{ {decl_members} }}): JsValue;
         function main(): void {{
           __thaw_typed_tjs_{run}({members});
         }}",
        run = stream_ctor_hex(decl_symbol),
    )
}

#[test]
fn thaw_js_routed_transform_ctor_with_a_callable_cb_builds() {
    let source = stream_transform_source(
        STREAM_TRANSFORM_CTOR,
        "{ transform(c, e, cb) { cb(null, 'X'); } }",
        &format!("transform: {TRANSFORM_FN}"),
    );
    let (uses_thaw_js, uses_quickjs, ir) =
        stream_ctor_routed_build(&source).expect("the Transform option object builds on thaw-js");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("__thaw_tjs_handle__"), "{ir}");
    assert!(
        ir.contains("call { i64, ptr } @thaw_tjs_register_native_method_callback_graph("),
        "{ir}"
    );
    assert!(!ir.contains("cannot cross the thaw-js boundary"), "{ir}");
}

#[test]
fn thaw_js_routed_transform_ctor_with_flush_builds() {
    let source = stream_transform_source(
        "$new$node:stream::Transform$arity1$overload2",
        "{ transform(c, e, cb) { cb(null, 'X'); }, flush(cb) { cb(null, 'z'); } }",
        &format!("transform: {TRANSFORM_FN}, flush: {FLUSH_FN}"),
    );
    let (uses_thaw_js, uses_quickjs, ir) =
        stream_ctor_routed_build(&source).expect("the Transform flush option builds on thaw-js");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(!ir.contains("cannot cross the thaw-js boundary"), "{ir}");
}

#[test]
fn thaw_js_routed_transform_ctor_with_a_rest_nested_callback_is_still_rejected() {
    let source = stream_transform_source(
        STREAM_TRANSFORM_CTOR,
        "{ transform(c, e, cb) { cb(null, 'X'); } }",
        "transform: ((arg0: JsValue, arg1: string, arg2: (...a: Json[]) => void) => void) | undefined",
    );
    let error = stream_ctor_routed_build(&source).expect_err("a rest nested callback stays on QuickJS");
    assert!(error.contains("cannot cross the thaw-js boundary"), "{error}");
}
