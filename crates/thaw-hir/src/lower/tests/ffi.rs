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

