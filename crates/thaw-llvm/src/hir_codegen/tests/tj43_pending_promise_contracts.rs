// thaw-js pending `Promise<T>` results (`\u{4}` symbol, `__thaw_tjs_pending__` marker, generated settler).

#[test]
fn tj43_pending_ok_admits_only_plain_awaited_types() {
    use thaw_hir::HirType::*;
    assert!(thaw_js_pending_ok(&Array(Box::new(Json))));
    assert!(thaw_js_pending_ok(&F64) && thaw_js_pending_ok(&Void));
    assert!(!thaw_js_pending_ok(&JsValue));
    assert!(!thaw_js_pending_ok(&Function(vec![F64], Box::new(F64))));
    assert!(!thaw_js_pending_ok(&Object(vec![("timestamp".into(), F64)])));
}

fn tj43_compile_routed(source: &str, name: &str) -> (bool, bool, String) {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, name);
    compiler.set_thaw_js_routed(true);
    // compile_program verifies the module in debug builds, so cross-function block edges are checked here.
    compiler.compile_program(&program).unwrap();
    (compiler.uses_thaw_js(), compiler.uses_quickjs(), compiler.print_to_string())
}

#[test]
fn tj43_promise_array_call_emits_pending_branch_and_settler() {
    let once = tj6_hex("node:events::once");
    let source = format!(
        "declare function __thaw_typed_tjs_{once}(name: string): Promise<string[]>;
         async function main(): Promise<void> {{ console.log(await __thaw_typed_tjs_{once}(\"x\")); }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = tj43_compile_routed(&source, "tj43_pending");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("call void @thaw_tjs_watch_promise("), "{ir}");
    assert!(ir.contains("define internal void @__thaw_tjs_settle_body_"), "{ir}");
    assert!(ir.contains("define internal void @__thaw_tjs_settle_"), "{ir}");
    assert!(ir.contains("@thaw_promise_reject_text_copy("), "{ir}");
    assert!(ir.contains("@thaw_json_has_own("), "{ir}");
    assert!(ir.contains("__thaw_tjs_pending__"), "{ir}");
    assert!(ir.contains("\\04node:events::once") || ir.contains("\u{4}node:events::once"), "{ir}");
    assert!(!ir.contains("non-plain-data"), "{ir}");
}

#[test]
fn tj43_promise_void_call_settles_without_decoding_a_value() {
    let tick = tj6_hex("k::tick43");
    let source = format!(
        "declare function __thaw_typed_tjs_{tick}(): Promise<void>;
         async function main(): Promise<void> {{ await __thaw_typed_tjs_{tick}(); console.log(\"done\"); }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = tj43_compile_routed(&source, "tj43_void");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("call void @thaw_tjs_watch_promise("), "{ir}");
    assert!(ir.contains("define internal void @__thaw_tjs_settle_"), "{ir}");
    assert!(ir.contains("@thaw_promise_resolve("), "{ir}");
}

#[test]
fn tj43_non_promise_call_gets_no_pending_machinery() {
    let join = tj6_hex("path::join");
    let (_, _, ir) = tj43_compile_routed(
        &format!(
            "declare function __thaw_typed_tjs_{join}(a: string): string;
             function main(): void {{ console.log(__thaw_typed_tjs_{join}(\"a\")); }}"
        ),
        "tj43_plain",
    );
    assert!(!ir.contains("call void @thaw_tjs_watch_promise(") && !ir.contains("__thaw_tjs_settle_") && !ir.contains("__thaw_tjs_pending__"), "{ir}");
}

#[test]
fn tj43_promise_of_non_plain_element_gets_no_pending_machinery() {
    let each = tj6_hex("k::each43");
    let (_, _, ir) = tj43_compile_routed(
        &format!(
            "declare function __thaw_typed_tjs_{each}(): Promise<Map<string, number>>;
             async function main(): Promise<void> {{ const m = await __thaw_typed_tjs_{each}(); console.log(m.size); }}"
        ),
        "tj43_nonplain",
    );
    assert!(!ir.contains("call void @thaw_tjs_watch_promise(") && !ir.contains("__thaw_tjs_settle_"), "{ir}");
}
