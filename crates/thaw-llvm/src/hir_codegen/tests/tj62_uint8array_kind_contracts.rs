// tj62: a `Bytes` value coerced to `Json` brands its wrapper with the runtime kind, so a plain
// Uint8Array keeps its kind (`thaw_json_brand_bytes_kind`). Compile-only IR contract.

#[test]
fn tj62_bytes_to_json_coercion_brands_the_runtime_kind_after_wrapping() {
    let f = tj35_symbol("m::show");
    let (uses_thaw_js, _, ir) = compile_thaw_js_program(&format!(
        "declare function {f}(value: Json): Json;
         function main(): void {{ console.log({f}(new Uint8Array(3))); }}"
    ));
    assert!(uses_thaw_js, "{ir}");
    assert!(tj35_calls(&ir, "thaw_json_brand_wrapper"), "{ir}");
    assert!(tj35_calls(&ir, "thaw_bytes_console_info"), "{ir}");
    assert!(tj35_calls(&ir, "thaw_json_brand_bytes_kind"), "{ir}");
}
