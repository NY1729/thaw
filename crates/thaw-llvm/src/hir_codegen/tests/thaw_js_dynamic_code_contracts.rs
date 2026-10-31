// thaw-js: user `new Function(...)` and `eval(...)` resolve to `thaw_tjs_*` and link no QuickJS in a
// routed build; the unrouted build keeps the QuickJS symbols.

const THAW_JS_DYNAMIC_PROGRAM: &str = "function main(): void {
       const f: any = new Function('a', 'return a + 1');
       console.log(f(1), eval('1 + 2'));
     }";

fn thaw_js_dynamic_has_call(ir: &str, symbol: &str) -> bool {
    ir.lines().any(|line| line.contains(" call ") && line.contains(&format!("@{symbol}(")))
}

#[test]
fn thaw_js_routed_dynamic_code_names_only_thaw_tjs_symbols() {
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(THAW_JS_DYNAMIC_PROGRAM, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    for symbol in ["thaw_tjs_new_function", "thaw_tjs_call_graph_result"] {
        assert!(thaw_js_dynamic_has_call(&ir, symbol), "{symbol} missing: {ir}");
    }
    assert!(!thaw_js_dynamic_has_call(&ir, "thaw_js_new_function"), "{ir}");
    assert!(!thaw_js_dynamic_has_call(&ir, "thaw_js_call_graph_result"), "{ir}");
}

#[test]
fn thaw_js_unrouted_dynamic_code_keeps_the_quickjs_symbols() {
    let (_, uses_quickjs, ir) = thaw_js_routed_ir(THAW_JS_DYNAMIC_PROGRAM, false);
    assert!(uses_quickjs, "{ir}");
    assert!(thaw_js_dynamic_has_call(&ir, "thaw_js_new_function"), "{ir}");
    assert!(!thaw_js_dynamic_has_call(&ir, "thaw_tjs_new_function"), "{ir}");
}
