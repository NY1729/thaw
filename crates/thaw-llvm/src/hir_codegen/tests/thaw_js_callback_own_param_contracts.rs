// A graph callback returning its own `JsValue` parameter must publish a branded
// `__thaw_js_handle_id__` wrapper (the result takes its own lease in the encoder),
// and must release the parameter handle only after the result has been encoded.

fn callback_adapter_with_graph_encode(ir: &str) -> Option<Vec<String>> {
    let mut functions: Vec<Vec<String>> = Vec::new();
    for line in ir.lines() {
        if line.starts_with("define ") {
            functions.push(Vec::new());
        }
        if let Some(body) = functions.last_mut() {
            body.push(line.to_string());
        }
    }
    let mut matches = functions
        .into_iter()
        .filter(|body| body.iter().any(|line| line.contains("%napi_value_callback_result_string =")));
    matches.next()
}

#[test]
fn thaw_js_callback_returning_own_jsvalue_param_brands_and_orders_release_after_encode() {
    let make = "63626f776e3a3a6d616b65".to_string();
    let source = format!(
        "declare function __thaw_typed_tjs_{make}(): JsValue;
         function main(): void {{
           const o: any = __thaw_typed_tjs_{make}();
           const r: any = o.list.map((x: JsValue): JsValue => x);
         }}"
    );
    let (uses_thaw_js, uses_quickjs, ir) = compile_routed(&source, "thaw_js_callback_own_param");
    assert!(uses_thaw_js && !uses_quickjs, "{ir}");
    let adapter = callback_adapter_with_graph_encode(&ir)
        .unwrap_or_else(|| panic!("no callback result encoder in IR:\n{ir}"));
    let line_of = |needle: &str| adapter.iter().position(|line| line.contains(needle));
    let encode = line_of("@thaw_json_graph_encode(").expect("encode call");
    let brand = line_of("@thaw_json_brand_wrapper(")
        .unwrap_or_else(|| panic!("no brand call:\n{}", adapter.join("\n")));
    assert!(brand < encode, "result wrapper must be branded before encoding:\n{}", adapter.join("\n"));
    let early_release = adapter[..encode]
        .iter()
        .any(|line| line.contains("release_native_callback_argument"));
    assert!(!early_release, "parameter handle released before encode:\n{}", adapter.join("\n"));
    let late_release = adapter[encode..]
        .iter()
        .any(|line| line.contains("release_native_callback_argument"));
    assert!(late_release, "parameter handle must still be released after encode:\n{}", adapter.join("\n"));
}
