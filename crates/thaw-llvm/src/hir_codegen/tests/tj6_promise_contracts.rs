// thaw-js typed calls with a `Promise<T>` result: the thaw-js side awaits the settled value and the
// compiled side wraps it in a resolved native Promise (as the QuickJS path does).

fn tj6_hex(key: &str) -> String {
    key.bytes().map(|b| format!("{b:02x}")).collect()
}

fn tj6_compile_routed(source: &str, name: &str) -> (bool, bool, String) {
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
fn tj6_routed_promise_result_is_awaited_then_wrapped_in_a_native_promise() {
    let run = tj6_hex("k::run");
    let source = format!(
        "declare function __thaw_typed_tjs_{run}(x: number): Promise<number>;
         async function main(): Promise<void> {{ console.log(await __thaw_typed_tjs_{run}(2)); }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = tj6_compile_routed(&source, "tj6_promise_number");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("@thaw_tjs_call_typed_result("), "{ir}");
    assert!(ir.contains("@thaw_promise_new("), "{ir}");
    assert!(ir.contains("@thaw_promise_resolve("), "{ir}");
    assert!(!ir.contains("non-plain-data"), "{ir}");
}

#[test]
fn tj6_routed_promise_void_result_routes_through_the_typed_call() {
    let tick = tj6_hex("k::tick");
    let source = format!(
        "declare function __thaw_typed_tjs_{tick}(): Promise<void>;
         async function main(): Promise<void> {{ await __thaw_typed_tjs_{tick}(); console.log(\"done\"); }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = tj6_compile_routed(&source, "tj6_promise_void");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("@thaw_tjs_call_typed_result("), "{ir}");
    assert!(!ir.contains("non-plain-data"), "{ir}");
}

#[test]
fn tj6_routed_promise_of_a_non_plain_type_still_stubs() {
    let run = tj6_hex("k::run");
    let source = format!(
        "declare function __thaw_typed_tjs_{run}(): Promise<JsValue>;
         async function main(): Promise<void> {{ await __thaw_typed_tjs_{run}(); }}"
    );
    let (_, _, ir) = tj6_compile_routed(&source, "tj6_promise_non_plain");
    assert!(ir.contains("non-plain-data"), "{ir}");
}

#[test]
fn tj6_routed_class_method_promise_result_sets_the_await_bit() {
    let new = tj6_hex("$new$k::Svc$arity0");
    let run = tj6_hex("$method$k::Svc$run$overload0$arity1");
    let source = format!(
        "declare function __thaw_typed_tjs_{new}(): JsValue;
         declare function __thaw_typed_tjs_{run}(receiver: JsValue, n: number): Promise<number>;
         async function main(): Promise<void> {{
           const svc = __thaw_typed_tjs_{new}();
           console.log(await __thaw_typed_tjs_{run}(svc, 2));
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = tj6_compile_routed(&source, "tj6_class_promise");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("@thaw_tjs_class_call("), "{ir}");
    // flags: bit 1 (await) only; the result is not a handle
    assert!(ir.contains("i8 2)"), "{ir}");
    assert!(ir.contains("@thaw_promise_resolve("), "{ir}");
    assert!(!ir.contains("non-plain-data"), "{ir}");
}

#[test]
fn tj6_routed_promise_of_a_date_stays_unrouted() {
    let when = tj6_hex("k::when");
    let source = format!(
        "declare function __thaw_typed_tjs_{when}(): Promise<Date>;
         async function main(): Promise<void> {{ await __thaw_typed_tjs_{when}(); }}"
    );
    let (_, _, ir) = tj6_compile_routed(&source, "tj6_promise_date");
    assert!(ir.contains("non-plain-data"), "{ir}");
}
