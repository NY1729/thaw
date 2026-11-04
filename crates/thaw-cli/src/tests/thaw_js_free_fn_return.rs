// thaw-js free functions: a `JsValue` return stays a live handle (`keep_ret_js_value`) while its
// parameters still flatten to `Json`. `node:*` builtins keep the plain `Json` return.

fn tjs_free_function(name: &str, ret: thaw_bridge::DtsType) -> thaw_bridge::DtsFunction {
    thaw_bridge::DtsFunction {
        param_field_constraints: vec![None, None],
        name: name.into(),
        generic: None,
        params: vec![
            (
                "name".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
            (
                "inner".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::JsValue),
            ),
        ],
        required_params: 2,
        rest_param: None,
        ret,
    }
}

#[test]
fn thaw_js_free_function_jsvalue_return_stays_a_handle_and_params_flatten() {
    let function = tjs_free_function(
        "makeThing",
        thaw_bridge::DtsType::Native(thaw_hir::HirType::JsValue),
    );
    let (_, declaration) = typed_dynamic_declaration(
        "tjs!chain-kit",
        &function,
        false,
        &std::collections::BTreeSet::new(),
        None,
    )
    .unwrap();
    assert!(declaration.contains("inner: Json"), "{declaration}");
    assert!(declaration.contains("): JsValue"), "{declaration}");
    assert!(!declaration.contains("JsValue, "), "{declaration}");

    // A `node:` builtin keeps its plain `Json` return.
    let spawn = tjs_free_function(
        "spawn",
        thaw_bridge::DtsType::Native(thaw_hir::HirType::JsValue),
    );
    let (_, declaration) = typed_dynamic_declaration(
        "tjs!node:child_process",
        &spawn,
        false,
        &std::collections::BTreeSet::new(),
        None,
    )
    .unwrap();
    assert!(declaration.contains("): Json"), "{declaration}");
    assert!(!declaration.contains("JsValue"), "{declaration}");
}

#[test]
fn thaw_js_retarget_keeps_only_the_return_of_a_multi_line_wrapper() {
    let wrapper = "declare function __thaw_typed_js_6d73(\n  inner: JsValue,\n  name: string\n): JsValue {\n";
    let (symbol, declaration) = thaw_js_retarget(
        "__thaw_typed_js_6d73".into(),
        wrapper.into(),
        false,
        true,
    );
    assert_eq!(symbol, "__thaw_typed_tjs_6d73");
    assert_eq!(
        declaration,
        "declare function __thaw_typed_tjs_6d73(\n  inner: Json,\n  name: string\n): JsValue {\n"
    );
}
