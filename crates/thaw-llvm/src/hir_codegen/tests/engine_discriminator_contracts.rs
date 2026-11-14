// The JS engine discriminator: compiled code tells thaw-std which engine backs its JS handles by
// calling `thaw_js_engine_id()`, which the rename pass turns into `thaw_tjs_engine_id()` only
// when the build really links thaw-js (a routed build that falls back to QuickJS keeps `thaw_js_`).

const ENGINE_DISCRIMINATOR_PROGRAM: &str = "function main(): void {
       const callback: (value: JsValue) => number = (value: JsValue): number => 1;
       const handle: JsValue = registerNativeCallback(callback);
       console.log(String(handle));
     }";

// Same callback registration, plus a typed call that only the QuickJS backend can serve, so a
// routed build must fall back to QuickJS.
const ENGINE_DISCRIMINATOR_FALLBACK_PROGRAM: &str =
    "declare function __thaw_typed_js_6d733a3a6d73(value: string, scale: number): Json;
     function main(): void {
       const callback: (value: JsValue) => number = (value: JsValue): number => 1;
       const handle: JsValue = registerNativeCallback(callback);
       console.log(String(handle), __thaw_typed_js_6d733a3a6d73(\"2 days\", 1));
     }";

fn engine_discriminator_ir(source: &str, routed: bool) -> (bool, bool, String) {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "engine_discriminator");
    compiler.set_thaw_js_routed(routed);
    compiler.compile_program(&program).unwrap();
    (
        compiler.uses_thaw_js(),
        compiler.uses_quickjs(),
        compiler.print_to_string(),
    )
}

fn calls(ir: &str, symbol: &str) -> bool {
    ir.contains(&format!("@{symbol}("))
}

#[test]
fn quickjs_build_reports_the_quickjs_engine_id_and_sets_the_json_engine() {
    let (uses_thaw_js, uses_quickjs, ir) =
        engine_discriminator_ir(ENGINE_DISCRIMINATOR_PROGRAM, false);
    assert!(uses_quickjs && !uses_thaw_js, "{ir}");
    assert!(calls(&ir, "thaw_json_set_js_engine"), "{ir}");
    assert!(calls(&ir, "thaw_js_engine_id"), "{ir}");
    assert!(!calls(&ir, "thaw_tjs_engine_id"), "{ir}");
}

#[test]
fn routed_thaw_js_build_reports_the_thaw_js_engine_id() {
    let (uses_thaw_js, uses_quickjs, ir) =
        engine_discriminator_ir(ENGINE_DISCRIMINATOR_PROGRAM, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(calls(&ir, "thaw_json_set_js_engine"), "{ir}");
    assert!(calls(&ir, "thaw_tjs_engine_id"), "{ir}");
    assert!(!calls(&ir, "thaw_js_engine_id"), "{ir}");
}

#[test]
fn routed_build_that_falls_back_to_quickjs_reports_the_quickjs_engine_id() {
    let (_, uses_quickjs, ir) =
        engine_discriminator_ir(ENGINE_DISCRIMINATOR_FALLBACK_PROGRAM, true);
    assert!(uses_quickjs, "{ir}");
    assert!(calls(&ir, "thaw_json_set_js_engine"), "{ir}");
    assert!(calls(&ir, "thaw_js_engine_id"), "{ir}");
    assert!(!calls(&ir, "thaw_tjs_engine_id"), "{ir}");
}
