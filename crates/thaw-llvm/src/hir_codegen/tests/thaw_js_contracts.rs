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
