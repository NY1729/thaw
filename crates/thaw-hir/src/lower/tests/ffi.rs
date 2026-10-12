#[test]
fn lowers_ambient_declaration_call_to_ffi_call() {
    let program = lower(
        r#"declare function native_add(a: number, b: number): number;

        function main(): void {
            console.log(native_add(2, 3));
        }"#,
    );

    assert_eq!(
        program.functions.len(),
        1,
        "the ambient decl has no body to lower"
    );
    assert_eq!(
        program.extern_functions,
        vec![crate::FfiSignature {
            symbol: "native_add".into(),
            params: vec![HirType::F64, HirType::F64],
            variadic: None,
            variadic_abi: crate::FfiVariadicAbi::Native,
            ret: HirType::F64,
            error_abi: crate::FfiErrorAbi::Direct,
            return_ownership: crate::FfiOwnership::Borrowed,
            error_ownership: crate::FfiOwnership::Borrowed,
            param_string_abis: vec![crate::FfiStringAbi::NullTerminated; 2],
            return_string_abi: crate::FfiStringAbi::NullTerminated,
            calling_convention: crate::FfiCallingConvention::C,
            aggregate_return_abi: crate::FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
        }]
    );

    let main = &program.functions[0];
    assert_eq!(
        main.body[0],
        HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("console.log".into())),
            vec![HirExpr::FfiCall(
                Box::new(crate::FfiSignature {
                    symbol: "native_add".into(),
                    params: vec![HirType::F64, HirType::F64],
                    variadic: None,
                    variadic_abi: crate::FfiVariadicAbi::Native,
                    ret: HirType::F64,
                    error_abi: crate::FfiErrorAbi::Direct,
                    return_ownership: crate::FfiOwnership::Borrowed,
                    error_ownership: crate::FfiOwnership::Borrowed,
                    param_string_abis: vec![crate::FfiStringAbi::NullTerminated; 2],
                    return_string_abi: crate::FfiStringAbi::NullTerminated,
                    calling_convention: crate::FfiCallingConvention::C,
                    aggregate_return_abi: crate::FfiAggregateAbi::Internal,
                    aggregate_return_layout: None,
                }),
                vec![
                    HirExpr::Lit(HirLit::F64(2.0)),
                    HirExpr::Lit(HirLit::F64(3.0)),
                ],
            )],
        ))
    );
}

#[test]
fn specializes_generic_dynamic_ambient_arguments_per_call() {
    let program = lower(
        r#"declare function __thaw_typed_napi_7765616b<T extends object>(value: T): JsValue;
           function double(value: number): number { return value * 2; }
           function main(): void {
               const reference = __thaw_typed_napi_7765616b(double);
               console.log(reference);
           }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let lowered = format!("{:?}", main.body);
    assert!(lowered.contains("DynamicCall"), "{lowered}");
    assert!(lowered.contains("Function([F64], F64)"), "{lowered}");
}

#[test]
fn accepts_json_callbacks_for_jsvalue_callback_parameters() {
    let program = lower(
        r#"declare function nativeAll(callback: (error: JsValue, rows: JsValue) => void): void;
           function main(): void {
               nativeAll((error: Json, rows: Json): void => { console.log(rows); });
           }"#,
    );
    assert!(format!("{:?}", program.functions[0].body).contains("FfiCall"));
}

#[test]
fn resolves_namespace_scoped_types_used_by_flattened_ambient_functions() {
    let program = lower(
        r#"declare namespace Native {
               export type Backend = "inotify" | "windows";
               export interface Options { backend?: Backend; }
           }
           declare function subscribe(options: Options): void;
           function main(): void { subscribe({ backend: "inotify" }); }"#,
    );
    let body = format!("{:?}", program.functions[0].body);
    assert!(body.contains("Optional(Str)"), "{body}");
}

#[test]
fn generated_dynamic_wrappers_preserve_json_callback_boundaries() {
    let program = lower(
        r#"
        declare function __thaw_typed_js_66(value: Json): JsValue;
        function __thaw_typed_wrapper_js_66(value: Json): JsValue {
            return __thaw_typed_js_66(value);
        }
        function forward(value: Json): JsValue {
            return __thaw_typed_wrapper_js_66(value);
        }
        function main(): void {
            const callback = (value: Json): Json => value;
            forward(callback);
        }
        "#,
    );
    let wrapper = program
        .functions
        .iter()
        .find(|function| function.name == "__thaw_typed_wrapper_js_66")
        .unwrap();
    assert_eq!(wrapper.params[0].ty, HirType::Json);
}

#[test]
fn rejects_unsupported_ambient_variadic_element_types() {
    let module = thaw_parser::parse_typescript(
        r#"declare function native_merge(...values: (boolean | undefined)[][]): number;
           function main(): void { console.log(native_merge([true])); }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("unsupported rest element layout"), "{error}");
}

#[test]
fn variadic_ambient_calls_still_require_every_fixed_argument() {
    let module = thaw_parser::parse_typescript(
        r#"declare function native_sum(count: number, ...values: number[]): number;
           function main(): void { console.log(native_sum()); }"#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("expects at least 1 argument(s), got 0"),
        "{error}"
    );
}

/// `nanoid<Type extends string>(size?: number): Type` -- `Type` appears
/// solely in the return position, so nothing in `size`'s own type ever
/// mentions it. `infer_generic_type_tuple` has no argument to infer it
/// from, but a `let`/`const` declaration's own type annotation is
/// offered as a fallback via `lower_expr_with_expected_type`, so
/// `const id: string = nanoid()` still resolves `Type` to `string` (and,
/// separately, the actual `FfiCall`'s substituted `params`/`ret` -- not
/// just the outer `let`'s own declared type -- must reflect that: they
/// were collected with every type parameter placeholder'd to `Dynamic`,
/// re-derived per call site once `Type` is known).
#[test]
fn infers_a_return_only_generic_type_parameter_from_a_let_annotation() {
    let program = lower(
        r#"declare function nanoid<Type extends string>(size?: number): Type;
           function main(): void {
               const id: string = nanoid();
               console.log(id);
           }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert_eq!(
        main.body[0],
        HirStmt::Let(
            "id".into(),
            HirType::Str,
            HirExpr::FfiCall(
                Box::new(crate::FfiSignature {
                    symbol: "nanoid".into(),
                    params: vec![HirType::Optional(Box::new(HirType::F64))],
                    variadic: None,
                    variadic_abi: crate::FfiVariadicAbi::Native,
                    ret: HirType::Str,
                    error_abi: crate::FfiErrorAbi::Direct,
                    return_ownership: crate::FfiOwnership::Borrowed,
                    error_ownership: crate::FfiOwnership::Borrowed,
                    param_string_abis: vec![crate::FfiStringAbi::NullTerminated],
                    return_string_abi: crate::FfiStringAbi::NullTerminated,
                    calling_convention: crate::FfiCallingConvention::C,
                    aggregate_return_abi: crate::FfiAggregateAbi::Internal,
                    aggregate_return_layout: None,
                }),
                Vec::new(),
            ),
        )
    );
}

/// The same function called with its optional parameter actually
/// supplied: the argument must still be coerced to the *substituted*
/// declared type (`Optional(F64)`, not the pre-substitution `Dynamic`
/// placeholder coercion would previously have skipped entirely for any
/// generic call), wrapping the raw `5` into that Optional's own
/// representation.
#[test]
fn coerces_arguments_of_a_generic_ambient_call_to_the_substituted_type() {
    let program = lower(
        r#"declare function nanoid<Type extends string>(size?: number): Type;
           function main(): void {
               const short: string = nanoid(5);
               console.log(short);
           }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let HirStmt::Let(_, _, HirExpr::FfiCall(_, args)) = &main.body[0] else {
        panic!("expected a Let binding an FfiCall, got {:?}", main.body[0]);
    };
    assert_eq!(
        args.as_slice(),
        [HirExpr::OptionalSome(
            Box::new(HirExpr::Lit(HirLit::F64(5.0))),
            HirType::F64,
        )]
    );
}


#[test]
fn marks_fresh_json_arguments_for_ordinary_quickjs_returns_only() {
    let program = lower(
        r#"
        declare function __thaw_typed_wrapper_js_66(value: Json): JsValue;
        declare function __thaw_typed_js_6563686f(value: Json): JsValue;
        declare function __thaw_typed_js_67656e65726963<T>(value: Json, key: T): JsValue;
        declare function __thaw_typed_js_69735265616479(value: Json): boolean;
        declare function __thaw_typed_js_766f6964(value: Json): void;
        declare function __thaw_typed_js_70616972(value: Json, key: JsValue): JsValue;
        declare function __thaw_typed_wrapper_js_user_store(value: Json): Json;
        declare function consume_json(value: Json): void;
        function main(live: JsValue): void {
            __thaw_typed_js_6563686f(live);
        }
        function generated_wrapper(live: JsValue): void {
            __thaw_typed_wrapper_js_66(live);
        }
        function generic(live: JsValue): void {
            __thaw_typed_js_67656e65726963<string>(live, "key");
        }
        function boolean_result(live: JsValue): void {
            __thaw_typed_js_69735265616479(live);
        }
        function void_result(live: JsValue): void {
            __thaw_typed_js_766f6964(live);
        }
        function nested_unproven_store(live: JsValue, key: JsValue): void {
            __thaw_typed_js_70616972(__thaw_typed_wrapper_js_user_store(live), key);
        }
        function borrowed(value: Json): void {
            __thaw_typed_js_6563686f(value);
            consume_json(value);
        }
        "#,
    );
    let function_body = |name: &str| {
        format!(
            "{:?}",
            program.functions.iter().find(|function| function.name == name).unwrap().body
        )
    };
    for name in ["main", "generated_wrapper", "generic", "boolean_result", "void_result"] {
        let body = function_body(name);
        assert!(
            body.contains("__thaw_mark_typed_wrapper_json_argument"),
            "{name}: {body}"
        );
    }
    for name in ["borrowed"] {
        let body = function_body(name);
        assert!(
            !body.contains("__thaw_mark_typed_wrapper_json_argument"),
            "{name}: {body}"
        );
    }
    assert!(function_body("borrowed").contains("consume_json"));
    let nested = function_body("nested_unproven_store");
    assert!(nested.contains("backend: QuickJs"), "{nested}");
    assert!(nested.contains("symbol: \"pair\""), "{nested}");
    assert!(nested.contains("ret: JsValue"), "{nested}");
    assert!(nested.contains("__thaw_mark_typed_wrapper_json_argument"), "{nested}");
    assert!(nested.contains("Str(\"wrapper\")"), "{nested}");
    assert!(nested.contains("__thaw_typed_wrapper_js_user_store"), "{nested}");
}

#[test]
fn marks_only_live_json_coercions_to_napi_constructors() {
    let program = lower(
        r#"
        declare function __thaw_typed_napi_246e6577244f757465722e436c69656e7424617269747932(
            value: Json, key: JsValue,
        ): JsValue;
        declare function __thaw_typed_napi_246e6577244f757465722e47656e6572696324617269747932<T extends Json>(
            value: Json, key: JsValue,
        ): JsValue;
        declare function __thaw_typed_napi_666f6f(value: Json, key: JsValue): JsValue;
        function direct(live: JsValue): void {
            const ignored: JsValue = __thaw_typed_napi_246e6577244f757465722e436c69656e7424617269747932(live, "key");
        }
        function generic(live: JsValue): void {
            const ignored: JsValue = __thaw_typed_napi_246e6577244f757465722e47656e6572696324617269747932<Json>(live, live);
        }
        function non_constructor(live: JsValue): void {
            const ignored: JsValue = __thaw_typed_napi_666f6f(live, "key");
        }
        function borrowed(value: Json): void {
            const ignored: JsValue = __thaw_typed_napi_246e6577244f757465722e436c69656e7424617269747932(value, "key");
        }
        function shadowed(live: JsValue): void {
            const __thaw_typed_napi_246e6577244f757465722e436c69656e7424617269747932 =
                (value: Json, key: JsValue): JsValue => live;
            const ignored: JsValue = __thaw_typed_napi_246e6577244f757465722e436c69656e7424617269747932(live, "key");
        }
        "#,
    );
    let body = |name: &str| {
        format!(
            "{:?}",
            program.functions.iter().find(|function| function.name == name).unwrap().body,
        )
    };
    for (name, symbol) in [
        ("direct", "$new$Outer.Client$arity2"),
        ("generic", "$new$Outer.Generic$arity2"),
    ] {
        let body = body(name);
        assert!(body.contains("Str(\"napi-constructor\")"), "{name}: {body}");
        assert!(body.contains(&format!("Str(\"{symbol}\")")), "{name}: {body}");
        assert!(body.contains("JsValueAsJson"), "{name}: {body}");
    }
    for name in ["non_constructor", "borrowed", "shadowed"] {
        let body = body(name);
        assert!(!body.contains("__thaw_mark_typed_wrapper_json_argument"), "{name}: {body}");
    }
}

#[test]
fn source_cannot_forge_typed_wrapper_json_ownership_marker() {
    let module = thaw_parser::parse_typescript(
        r#"declare function __thaw_mark_typed_wrapper_json_argument(ownerKind: string, ownerKey: string, value: Json): Json;
           function main(value: Json): void { __thaw_mark_typed_wrapper_json_argument("direct-quickjs", "echo", value); }"#,
    ).unwrap();
    let error = crate::lower_module(&module).unwrap_err();
    assert!(error.contains("compiler-private"), "{error}");
}
