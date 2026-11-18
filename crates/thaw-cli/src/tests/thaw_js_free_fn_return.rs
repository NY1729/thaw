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

#[test]
fn thaw_js_free_function_unclassifiable_return_stays_a_handle() {
    // An interface return (`makeHolder(): Holder`) is `Unsupported`, not `JsValue`, but still lowers
    // to `JsValue`, so it must keep its handle the same way.
    let function = tjs_free_function(
        "makeHolder",
        thaw_bridge::DtsType::Unsupported("interface Holder is not supported".into()),
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

    // `any` and `unknown` returns still render as plain `Json`.
    for reason in ["`any` is not supported", "`unknown` is not supported"] {
        let function = tjs_free_function(
            "readAny",
            thaw_bridge::DtsType::Unsupported(reason.into()),
        );
        let (_, declaration) = typed_dynamic_declaration(
            "tjs!chain-kit",
            &function,
            false,
            &std::collections::BTreeSet::new(),
            None,
        )
        .unwrap();
        assert!(declaration.contains("): Json"), "{declaration}");
        assert!(!declaration.contains("JsValue"), "{declaration}");
    }

    // A `node:` builtin keeps its plain `Json` return even for an interface return.
    let spawn = tjs_free_function(
        "spawn",
        thaw_bridge::DtsType::Unsupported("interface Holder is not supported".into()),
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
fn thaw_js_free_function_promise_payload_return_is_not_a_bare_handle() {
    // An opaque promise payload is rewritten to a `Promise<...>`, never to a bare handle return.
    let function = tjs_free_function(
        "loadAll",
        thaw_bridge::DtsType::Unsupported(format!(
            "{}: `any` is not supported",
            thaw_bridge::PROMISE_PAYLOAD_UNSUPPORTED
        )),
    );
    let (_, declaration) = typed_dynamic_declaration(
        "tjs!chain-kit",
        &function,
        false,
        &std::collections::BTreeSet::new(),
        None,
    )
    .unwrap();
    assert!(declaration.contains("Promise<"), "{declaration}");
    assert!(!declaration.contains("): JsValue"), "{declaration}");
}

/// The ejs `compile` result: a union of two callables over an optional `Json` argument, one returning
/// `string`, the other `Promise<string>`.
fn tjs_ejs_compile() -> thaw_bridge::DtsFunction {
    use thaw_hir::{HirOptionalMask, HirType};
    let callable = |ret: HirType| {
        HirType::CallableFunction(
            vec![HirType::Optional(Box::new(HirType::Json))],
            HirOptionalMask::Inline(1),
            None,
            Box::new(ret),
        )
    };
    let mut function = tjs_free_function(
        "compile",
        thaw_bridge::DtsType::Native(HirType::Union(vec![
            callable(HirType::Str),
            callable(HirType::Promise(Box::new(HirType::Str))),
        ])),
    );
    function.params = vec![(
        "template".into(),
        thaw_bridge::DtsType::Native(HirType::Str),
    )];
    function.required_params = 1;
    function
}

#[test]
fn thaw_js_callable_union_result_is_a_handle_declared_once_for_every_wrapper() {
    let function = tjs_ejs_compile();
    // ejs.compile's union of plain callables crosses as a live JsValue handle, like a single callable
    let (symbol, declaration) = typed_dynamic_declaration(
        "tjs!tjs-ejs",
        &function,
        false,
        &std::collections::BTreeSet::new(),
        None,
    )
    .unwrap();
    assert!(symbol.starts_with("__thaw_typed_tjs_"), "{symbol}");
    assert!(declaration.contains("): JsValue"), "{declaration}");
    assert!(!declaration.contains("=>"), "{declaration}");
    // the bare and qualified aliases agree on the handle type (no `JsValue` -> `Json` retarget)
    let rewritten = thaw_js_callable_union_as_handle("tjs-ejs", &function);
    assert!(matches!(rewritten, std::borrow::Cow::Owned(_)));
    assert!(matches!(
        rewritten.ret,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::JsValue)
    ));
    // node: builtins keep their own return
    assert!(matches!(
        thaw_js_callable_union_as_handle("node:child_process", &function),
        std::borrow::Cow::Borrowed(_)
    ));
    // a union with a non-plain member (a `JsValue` return) is not a handle: it keeps its callable text
    let mut opaque = tjs_ejs_compile();
    opaque.ret = thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(vec![
        thaw_hir::HirType::CallableFunction(
            vec![thaw_hir::HirType::Json],
            thaw_hir::HirOptionalMask::Inline(0),
            None,
            Box::new(thaw_hir::HirType::JsValue),
        ),
        thaw_hir::HirType::Str,
    ]));
    let (_, declaration) = typed_dynamic_declaration(
        "tjs!tjs-ejs",
        &opaque,
        false,
        &std::collections::BTreeSet::new(),
        None,
    )
    .unwrap();
    assert!(!declaration.contains("): JsValue"), "{declaration}");
}
