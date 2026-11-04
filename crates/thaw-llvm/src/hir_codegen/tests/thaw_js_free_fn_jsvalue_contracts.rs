// thaw-js (`JsEngine::ThawJs`): a routed free function declared `: JsValue` (the thaw-cli retarget
// keeps that return) is called with the `\x02` want-handle prefix and its result is stored as an i64
// handle, not decoded as plain data.

#[test]
fn thaw_js_routed_free_function_jsvalue_result_is_an_i64_handle() {
    let source = "declare function __thaw_typed_tjs_636861696e2d6b69743a3a6d616b655468696e67(name: string): JsValue;
         function main(): void {
           const thing: JsValue = __thaw_typed_tjs_636861696e2d6b69743a3a6d616b655468696e67(\"x\");
           console.log(thing);
         }";
    let (uses_thaw_js, uses_quickjs, ir) = thaw_js_routed_ir(source, true);
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    assert!(ir.contains("\\02chain-kit::makeThing"), "{ir}");
    assert!(!ir.contains("c\"chain-kit::makeThing\\00\""), "{ir}");
    assert!(ir.contains("@thaw_tjs_call_typed_result("), "{ir}");
    assert!(ir.contains("fptosi double"), "{ir}");
}
