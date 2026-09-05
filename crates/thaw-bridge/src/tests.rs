use super::*;

#[test]
fn fallback_shim_retains_callable_return_values() {
    let functions =
        parse_dts("export declare function make(factor: number): (value: number) => number;")
            .unwrap();
    let shim = generate_shim(&functions, false, &[], &Default::default());
    assert!(
        shim.contains("function make(argsArray: Json): JsValue"),
        "{shim}"
    );
    assert!(
        shim.contains("callDynamicValueHandle(callable, argsArray)"),
        "{shim}"
    );
}

#[test]
fn callable_interface_return_is_preserved_as_javascript_value() {
    let functions = parse_dts(
        r#"export interface Limit {
                <T>(task: () => T): Promise<T>;
                readonly activeCount: number;
                clearQueue(): void;
            }
            export default function pLimit(concurrency: number): Limit;"#,
    )
    .unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0].name, "pLimit");
    assert_eq!(functions[0].ret, DtsType::Native(HirType::JsValue));
    let shim = generate_shim(&functions, false, &[], &Default::default());
    assert!(
        shim.contains("function pLimit(argsArray: Json): JsValue"),
        "{shim}"
    );
}

#[test]
fn classifies_simple_primitive_signature_as_fast_path() {
    let funcs = parse_dts("export declare function add(a: number, b: number): number;").unwrap();
    assert_eq!(funcs.len(), 1);
    assert_eq!(
        classify(&funcs[0]),
        Classification::FastPath(Box::new(FfiSignature {
            symbol: "add".into(),
            params: vec![HirType::F64, HirType::F64],
            variadic: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
            ret: HirType::F64,
            error_abi: FfiErrorAbi::Direct,
            return_ownership: FfiOwnership::Borrowed,
            error_ownership: FfiOwnership::Borrowed,
            param_string_abis: vec![FfiStringAbi::NullTerminated; 2],
            return_string_abi: FfiStringAbi::NullTerminated,
            calling_convention: FfiCallingConvention::C,
            aggregate_return_abi: FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
        }))
    );
}

#[test]
fn classifies_number_rest_signature_as_variadic_fast_path() {
    let funcs =
        parse_dts("export declare function sum(count: number, ...values: number[]): number;")
            .unwrap();
    assert_eq!(funcs[0].params.len(), 1);
    assert_eq!(
        funcs[0].rest_param,
        Some(("values".into(), DtsType::Native(HirType::F64)))
    );
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("number rest signature should classify as FastPath");
    };
    assert_eq!(signature.params, vec![HirType::F64]);
    assert_eq!(signature.variadic, Some(HirType::F64));
    assert_eq!(
        generate_shim(&funcs, true, &[], &Default::default()),
        "declare function sum(count: number, ...values: number[]): number;\n"
    );
}

#[test]
fn boolean_string_and_handle_rest_signatures_classify_as_variadic_fast_paths() {
    for (source, expected) in [
        (
            "export declare function all(...values: boolean[]): boolean;",
            HirType::Bool,
        ),
        (
            "export declare function join(...values: string[]): string;",
            HirType::Str,
        ),
        (
            "export declare function handles(...values: JsValue[]): number;",
            HirType::JsValue,
        ),
    ] {
        let funcs = parse_dts(source).unwrap();
        let classification = classify(&funcs[0]);
        let Classification::FastPath(signature) = classification else {
            panic!("supported rest signature should classify as FastPath: {classification:?}");
        };
        assert_eq!(signature.variadic, Some(expected));
    }
}

#[test]
fn tagged_object_rest_signature_uses_the_native_variadic_abi() {
    let funcs = parse_dts(
        "export declare function merge(...values: { value: number | undefined }[]): number;",
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("tagged object rest values should use the native variadic ABI");
    };
    assert_eq!(
        signature.variadic,
        Some(HirType::Object(vec![(
            "value".into(),
            HirType::Optional(Box::new(HirType::F64)),
        )]))
    );
}

#[test]
fn aggregate_rest_signatures_classify_as_variadic_fast_paths() {
    for (source, expected) in [
            (
                "export declare function arrays(...values: number[][]): number;",
                HirType::Array(Box::new(HirType::F64)),
            ),
            (
                "export declare function strings(...values: string[][]): number;",
                HirType::Array(Box::new(HirType::Str)),
            ),
            (
                "export declare function booleans(...values: boolean[][]): number;",
                HirType::Array(Box::new(HirType::Bool)),
            ),
            (
                "export declare function handles(...values: JsValue[][]): number;",
                HirType::Array(Box::new(HirType::JsValue)),
            ),
            (
                "export declare function objects(...values: { value: number; meta: { enabled: boolean; label: string }; samples: number[] }[]): number;",
                HirType::Object(vec![
                    ("value".into(), HirType::F64),
                    (
                        "meta".into(),
                        HirType::Object(vec![
                            ("enabled".into(), HirType::Bool),
                            ("label".into(), HirType::Str),
                        ]),
                    ),
                    (
                        "samples".into(),
                        HirType::Array(Box::new(HirType::F64)),
                    ),
                ]),
            ),
        ] {
            let functions = parse_dts(source).unwrap();
            let Classification::FastPath(signature) = classify(&functions[0]) else {
                panic!("aggregate rest signature should classify as FastPath");
            };
            assert_eq!(signature.variadic, Some(expected));
        }
}

#[test]
fn tagged_rest_signatures_classify_as_variadic_fast_paths() {
    for (source, expected) in [
            (
                "export declare function optional(...values: (number | undefined)[]): number;",
                HirType::Optional(Box::new(HirType::F64)),
            ),
            (
                "export declare function nullable(...values: (number | null)[]): number;",
                HirType::Nullable(Box::new(HirType::F64)),
            ),
            (
                "export declare function nullish(...values: (number | null | undefined)[]): number;",
                HirType::Nullish(Box::new(HirType::F64)),
            ),
            (
                "export declare function aggregate(...values: ({ value: number } | undefined)[]): number;",
                HirType::Optional(Box::new(HirType::Object(vec![(
                    "value".into(),
                    HirType::F64,
                )]))),
            ),
        ] {
            let functions = parse_dts(source).unwrap();
            let classification = classify(&functions[0]);
            let Classification::FastPath(signature) = classification else {
                panic!("tagged rest signature should classify as FastPath: {classification:?}");
            };
            assert_eq!(signature.variadic, Some(expected));
        }
}

#[test]
fn tagged_regular_signatures_classify_as_fast_paths() {
    let funcs = parse_dts(
        r#"export declare function inspect(
                count: number | undefined,
                label: string | null
            ): boolean | null | undefined;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("regular nullable unions should use the tagged native ABI");
    };
    assert_eq!(
        signature.params,
        vec![
            HirType::Optional(Box::new(HirType::F64)),
            HirType::Nullable(Box::new(HirType::Str)),
        ]
    );
    assert_eq!(signature.ret, HirType::Nullish(Box::new(HirType::Bool)));

    let shim = generate_shim(&funcs, true, &[], &Default::default());
    let module = thaw_parser::parse_typescript(&shim)
        .unwrap_or_else(|error| panic!("tagged bridge shim did not parse: {error}\n{shim}"));
    let program = thaw_hir::lower_module(&module)
        .unwrap_or_else(|error| panic!("tagged bridge shim did not lower: {error}\n{shim}"));
    assert_eq!(program.extern_functions.len(), 1);
    assert_eq!(
        program.extern_functions[0].params,
        vec![
            HirType::Optional(Box::new(HirType::F64)),
            HirType::Nullable(Box::new(HirType::Str)),
        ]
    );
    assert_eq!(
        program.extern_functions[0].ret,
        HirType::Nullish(Box::new(HirType::Bool))
    );
}

#[test]
fn resolves_tagged_unions_after_generic_substitution() {
    let funcs = parse_dts(
        r#"export interface Maybe<T> { value: T | null; }
            export declare function inspect(value: Maybe<number>): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("nullable generic fields should resolve after substitution");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![(
            "value".into(),
            HirType::Nullable(Box::new(HirType::F64)),
        )])]
    );
}

#[test]
fn resolves_forward_non_generic_type_aliases() {
    let funcs = parse_dts(
        r#"export type Request = Later;
            export type Later = {
                id: number;
                label: Label;
            };
            export type Label = string;
            export declare function inspect(value: Request): Label;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("forward non-generic aliases should resolve");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![
            ("id".into(), HirType::F64),
            ("label".into(), HirType::Str),
        ])]
    );
    assert_eq!(signature.ret, HirType::Str);
}

#[test]
fn resolves_utility_types_inside_non_generic_aliases() {
    let funcs = parse_dts(
        r#"export interface Config { host: string; port: number; }
            export type HostOnly = Pick<Config, "host">;
            export declare function inspect(value: HostOnly): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("utility types inside aliases should resolve");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![("host".into(), HirType::Str)])]
    );
}

#[test]
fn resolves_interfaces_and_aliases_across_forward_references() {
    let funcs = parse_dts(
        r#"export type Request = Envelope;
            export interface Envelope { payload: Payload; }
            export type Payload = { value: number };
            export declare function inspect(value: Request): number;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("interfaces and aliases should resolve each other's forward references");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![(
            "payload".into(),
            HirType::Object(vec![("value".into(), HirType::F64)]),
        )])]
    );
}

#[test]
fn cyclic_non_generic_aliases_fall_back() {
    let funcs = parse_dts(
        r#"export type Left = Right;
            export type Right = Left;
            export declare function inspect(value: Left): string;"#,
    )
    .unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn resolves_generic_type_aliases_and_defaults() {
    let funcs = parse_dts(
        r#"export type Box<T> = { value: T };
            export type Pair<T, U = T> = { left: T; right: U };
            export declare function inspect(
                value: Box<number>,
                pair: Pair<string>
            ): Box<boolean>;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("generic aliases and default arguments should resolve");
    };
    assert_eq!(
        signature.params,
        vec![
            HirType::Object(vec![("value".into(), HirType::F64)]),
            HirType::Object(vec![
                ("left".into(), HirType::Str),
                ("right".into(), HirType::Str),
            ]),
        ]
    );
    assert_eq!(
        signature.ret,
        HirType::Object(vec![("value".into(), HirType::Bool)])
    );
}

#[test]
fn resolves_generic_aliases_inside_generic_interfaces() {
    let funcs = parse_dts(
        r#"export type Selection<T> = Pick<T, keyof T>;
            export interface Envelope<T> { payload: Selection<T>; }
            export interface Config { host: string; port: number; }
            export declare function inspect(value: Envelope<Config>): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("generic aliases should resolve through outer substitutions");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![(
            "payload".into(),
            HirType::Object(vec![
                ("host".into(), HirType::Str),
                ("port".into(), HirType::F64),
            ]),
        )])]
    );
}

#[test]
fn resolves_generic_aliases_inside_non_generic_interfaces() {
    let funcs = parse_dts(
        r#"export type Box<T> = { value: T };
            export interface Request { payload: Box<number>; }
            export declare function inspect(value: Request): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("non-generic interfaces should resolve concrete generic aliases");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![(
            "payload".into(),
            HirType::Object(vec![("value".into(), HirType::F64)]),
        )])]
    );
}

#[test]
fn enforces_generic_alias_constraints() {
    let funcs = parse_dts(
        r#"export type Identified<T extends { id: number }> = { value: T };
            export interface Good { id: number; label: string; }
            export interface Bad { label: string; }
            export declare function good(value: Identified<Good>): string;
            export declare function bad(value: Identified<Bad>): string;"#,
    )
    .unwrap();
    assert!(matches!(classify(&funcs[0]), Classification::FastPath(_)));
    assert!(matches!(
        classify(&funcs[1]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn cyclic_generic_aliases_fall_back() {
    let funcs = parse_dts(
        r#"export type Loop<T> = Loop<T>;
            export declare function inspect(value: Loop<number>): string;"#,
    )
    .unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn classifies_finite_mapped_types() {
    let funcs = parse_dts(
        r#"export declare function inspect(
                value: { [K in "left" | "right"]?: number }
            ): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("finite mapped types should expand to fixed object fields");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![
            ("left".into(), HirType::Optional(Box::new(HirType::F64)),),
            ("right".into(), HirType::Optional(Box::new(HirType::F64)),),
        ])]
    );
}

#[test]
fn resolves_keyed_mapped_types_after_generic_substitution() {
    let funcs = parse_dts(
        r#"export type Snapshot<T> = { [K in keyof T]: T[K] };
            export type Complete<T> = { [K in keyof T]-?: T[K] };
            export interface Config { host?: string; port: number; }
            export declare function snapshot(value: Snapshot<Config>): Complete<Config>;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("keyed mapped types should resolve each substituted property");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![
            ("host".into(), HirType::Optional(Box::new(HirType::Str)),),
            ("port".into(), HirType::F64),
        ])]
    );
    assert_eq!(
        signature.ret,
        HirType::Object(vec![
            ("host".into(), HirType::Str),
            ("port".into(), HirType::F64),
        ])
    );
}

#[test]
fn resolves_static_mapped_key_remapping() {
    let funcs = parse_dts(
        r#"export type Renamed = {
                [K in "userName" | "value" as `get${Capitalize<string & K>}`]: number
            };
            export declare function inspect(value: Renamed): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("static mapped template keys should resolve");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![
            ("getUserName".into(), HirType::F64),
            ("getValue".into(), HirType::F64),
        ])]
    );
}

#[test]
fn resolves_intrinsic_mapped_key_transforms() {
    let funcs = parse_dts(
        r#"export type Names = {
                [K in "Hello" as `${Uppercase<K>}_${Lowercase<K>}_${Uncapitalize<K>}`]: boolean
            };
            export declare function inspect(value: Names): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("intrinsic mapped key transforms should resolve");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![(
            "HELLO_hello_hello".into(),
            HirType::Bool,
        )])]
    );
}

#[test]
fn rejects_dynamic_or_duplicate_mapped_key_remapping() {
    let funcs = parse_dts(
        r#"export type Dynamic = { [K in "name" as `get${Custom<K>}`]: number };
            export type Duplicate = { [K in "left" | "right" as "value"]: number };
            export declare function dynamic(value: Dynamic): string;
            export declare function duplicate(value: Duplicate): string;"#,
    )
    .unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
    assert!(matches!(
        classify(&funcs[1]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn resolves_concrete_conditional_types_lazily() {
    let funcs = parse_dts(
        r#"export type Selected = "value" extends string ? number : Date;
            export type Rejected = number extends string ? Date : boolean;
            export declare function inspect(value: Selected): Rejected;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("concrete conditional types should select one branch");
    };
    assert_eq!(signature.params, vec![HirType::F64]);
    assert_eq!(signature.ret, HirType::Bool);
}

#[test]
fn resolves_conditional_types_after_generic_substitution() {
    let funcs = parse_dts(
        r#"export type Result<T> = T extends { id: number }
                ? Pick<T, "id">
                : { value: string };
            export interface Good { id: number; label: string; }
            export interface Bad { label: string; }
            export declare function inspect(
                good: Result<Good>,
                bad: Result<Bad>
            ): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("conditional aliases should resolve after generic substitution");
    };
    assert_eq!(
        signature.params,
        vec![
            HirType::Object(vec![("id".into(), HirType::F64)]),
            HirType::Object(vec![("value".into(), HirType::Str)]),
        ]
    );
}

#[test]
fn unresolved_conditional_tests_fall_back() {
    let funcs = parse_dts(
        r#"export type Unknown = unknown extends string ? number : boolean;
            export declare function inspect(value: Unknown): string;"#,
    )
    .unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn classifies_quoted_and_numeric_object_property_keys() {
    let funcs = parse_dts(
        r#"export interface Headers {
                "content-type": string;
                200: boolean;
            }
            export declare function inspect(
                headers: Headers,
                inline: { "x-request-id": string; 404: number }
            ): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("quoted and numeric object keys should preserve fixed layouts");
    };
    assert_eq!(
        signature.params,
        vec![
            HirType::Object(vec![
                ("content-type".into(), HirType::Str),
                ("200".into(), HirType::Bool),
            ]),
            HirType::Object(vec![
                ("x-request-id".into(), HirType::Str),
                ("404".into(), HirType::F64),
            ]),
        ]
    );
}

#[test]
fn classifies_quoted_keys_inside_generic_interfaces() {
    let funcs = parse_dts(
        r#"export interface Entry<T> { "current-value": T; }
            export declare function inspect(value: Entry<number>): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("generic interfaces should preserve quoted keys");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![(
            "current-value".into(),
            HirType::F64,
        )])]
    );
}

#[test]
fn generates_native_addon_wrapper_and_module_initializer() {
    let funcs = parse_dts("export declare function add(args: any): any;").unwrap();
    let shim = generate_native_addon_shim(&funcs, &[], &Default::default());
    assert!(shim.contains(r#"return callNativeAddon("add", argsArray);"#));
    let init = generate_native_addon_init(&[NativeAddon {
        package_name: "native-add",
        bytes: &[0xde, 0xad, 0xbe, 0xef],
        root_export: None,
    }]);
    assert!(init.contains("function __thaw_native_module_init(): void"));
    assert!(init.contains(r#"loadNativeAddonEmbedded("deadbeef", "");"#));
}

#[test]
fn classifies_number_array_and_flat_object_as_fast_path() {
    let funcs = parse_dts(
        "export declare function sum(xs: number[]): number;\n\
             export declare function dist(p: { x: number; y: number }): number;",
    )
    .unwrap();

    assert_eq!(
        classify(&funcs[0]),
        Classification::FastPath(Box::new(FfiSignature {
            symbol: "sum".into(),
            params: vec![HirType::Array(Box::new(HirType::F64))],
            variadic: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
            ret: HirType::F64,
            error_abi: FfiErrorAbi::Direct,
            return_ownership: FfiOwnership::Borrowed,
            error_ownership: FfiOwnership::Borrowed,
            param_string_abis: vec![FfiStringAbi::NullTerminated; 1],
            return_string_abi: FfiStringAbi::NullTerminated,
            calling_convention: FfiCallingConvention::C,
            aggregate_return_abi: FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
        }))
    );
    assert_eq!(
        classify(&funcs[1]),
        Classification::FastPath(Box::new(FfiSignature {
            symbol: "dist".into(),
            params: vec![HirType::Object(vec![
                ("x".into(), HirType::F64),
                ("y".into(), HirType::F64),
            ])],
            variadic: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
            ret: HirType::F64,
            error_abi: FfiErrorAbi::Direct,
            return_ownership: FfiOwnership::Borrowed,
            error_ownership: FfiOwnership::Borrowed,
            param_string_abis: vec![FfiStringAbi::NullTerminated; 1],
            return_string_abi: FfiStringAbi::NullTerminated,
            calling_convention: FfiCallingConvention::C,
            aggregate_return_abi: FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
        }))
    );
}

#[test]
fn classifies_generic_function_as_fallback() {
    let funcs = parse_dts("export declare function identity<T>(x: T): T;").unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn preserves_generic_function_syntax_for_call_site_specialization() {
    let functions =
        parse_dts("declare function weak<T extends object>(object: T, callback?: () => void): T;")
            .unwrap();
    let generic = functions[0].generic.as_ref().unwrap();
    assert_eq!(
        generic.type_params,
        vec![("T".to_string(), Some("object".to_string()))]
    );
    assert_eq!(generic.param_types, vec!["T", "() => void"]);
}

#[test]
fn classifies_primitive_constrained_generic_function_as_fast_path() {
    let funcs =
        parse_dts("export declare function nanoid<Type extends string>(size?: number): Type;")
            .unwrap();
    assert_eq!(funcs[0].required_params, 0);
    assert!(matches!(
        classify(&funcs[0]),
        Classification::FastPath(signature)
            if signature.params == vec![HirType::F64] && signature.ret == HirType::Str
    ));
}

#[test]
fn substitutes_constrained_generic_inside_returned_callable() {
    let funcs = parse_dts(
        "export declare function custom<Type extends string>(size?: number): (length?: number) => Type;",
    )
    .unwrap();
    assert_eq!(funcs[0].required_params, 0);
    assert!(matches!(
        &funcs[0].ret,
        DtsType::Native(HirType::CallableFunction(params, optional, None, ret))
            if params == &vec![HirType::Optional(Box::new(HirType::F64))]
                && optional.contains(0)
                && ret.as_ref() == &HirType::Str
    ));
}

#[test]
fn classifies_union_parameter_as_fallback() {
    let funcs = parse_dts("export declare function f(x: string | number): void;").unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn classifies_same_layout_literal_unions_as_fast_path() {
    let functions = parse_dts(
        r#"export declare function mode(
                value: "read" | "write",
                count: 1 | 2 | number,
                enabled: true | false
            ): "ok" | "done";"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&functions[0]) else {
        panic!("same-layout literal union should use FastPath");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Str, HirType::F64, HirType::Bool]
    );
    assert_eq!(signature.ret, HirType::Str);
}

#[test]
fn classifies_compatible_object_intersections_as_fast_path() {
    let functions = parse_dts(
        r#"export declare function inspect(
                value: { name: string } & { count: number } & { enabled: boolean }
            ): number;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&functions[0]) else {
        panic!("compatible object intersection should use FastPath");
    };
    assert_eq!(
        signature.params[0],
        HirType::Object(vec![
            ("name".into(), HirType::Str),
            ("count".into(), HirType::F64),
            ("enabled".into(), HirType::Bool),
        ])
    );
}

#[test]
fn classifies_string_array_as_fast_path() {
    let funcs = parse_dts("export declare function f(xs: string[]): void;").unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("string arrays should use the native pointer-length ABI");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Array(Box::new(HirType::Str))]
    );
}

#[test]
fn classifies_boolean_array_as_fast_path() {
    let funcs = parse_dts("export declare function f(xs: boolean[]): boolean[];").unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("boolean arrays should use the native pointer-length ABI");
    };
    let expected = HirType::Array(Box::new(HirType::Bool));
    assert_eq!(signature.params, vec![expected.clone()]);
    assert_eq!(signature.ret, expected);
}

#[test]
fn classifies_js_value_array_as_fast_path() {
    let funcs = parse_dts("export declare function f(xs: JsValue[]): JsValue[];").unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("JsValue arrays should use the native pointer-length ABI");
    };
    let expected = HirType::Array(Box::new(HirType::JsValue));
    assert_eq!(signature.params, vec![expected.clone()]);
    assert_eq!(signature.ret, expected);
}

#[test]
fn retains_recursive_arrays_for_napi_but_not_direct_ffi() {
    let functions =
        parse_dts("export declare function f(values: string[][]): { name: string }[];").unwrap();
    assert_eq!(
        functions[0].params[0].1,
        DtsType::Native(HirType::Array(Box::new(HirType::Array(Box::new(
            HirType::Str,
        )))))
    );
    assert!(matches!(
        classify(&functions[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn classifies_tuples_as_fast_path() {
    let funcs =
        parse_dts("export declare function f(value: [number, string, boolean]): [string, number];")
            .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("tuples should use the native fixed-element ABI");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Tuple(vec![
            HirType::F64,
            HirType::Str,
            HirType::Bool,
        ])]
    );
    assert_eq!(
        signature.ret,
        HirType::Tuple(vec![HirType::Str, HirType::F64])
    );
}

#[test]
fn classifies_readonly_and_parenthesized_native_collections() {
    let funcs = parse_dts(
        "export declare function f(values: readonly string[], pair: readonly [number, string], flags: (boolean[])): readonly boolean[];",
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("readonly native collections should preserve their ABI layout");
    };
    assert_eq!(
        signature.params,
        vec![
            HirType::Array(Box::new(HirType::Str)),
            HirType::Tuple(vec![HirType::F64, HirType::Str]),
            HirType::Array(Box::new(HirType::Bool)),
        ]
    );
    assert_eq!(signature.ret, HirType::Array(Box::new(HirType::Bool)));
}

#[test]
fn classifies_readonly_utility_types_without_changing_the_abi() {
    let funcs = parse_dts(
        r#"export interface Config {
                host: string;
                port: number;
            }
            export declare function inspect(
                config: Readonly<Config>,
                names: ReadonlyArray<string>
            ): Readonly<Config>;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("readonly utility types should preserve their ABI layout");
    };
    let config = HirType::Object(vec![
        ("host".into(), HirType::Str),
        ("port".into(), HirType::F64),
    ]);
    assert_eq!(
        signature.params,
        vec![config.clone(), HirType::Array(Box::new(HirType::Str)),]
    );
    assert_eq!(signature.ret, config);
}

#[test]
fn resolves_readonly_utility_types_inside_generic_interfaces() {
    let funcs = parse_dts(
        r#"export interface Config { host: string; }
            export interface Envelope<T> {
                value: Readonly<T>;
                labels: ReadonlyArray<string>;
            }
            export declare function inspect(value: Envelope<Config>): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("readonly utilities should resolve after generic substitution");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![
            (
                "value".into(),
                HirType::Object(vec![("host".into(), HirType::Str)]),
            ),
            ("labels".into(), HirType::Array(Box::new(HirType::Str)),),
        ])]
    );
}

#[test]
fn classifies_object_keyof_as_a_string_fast_path() {
    let funcs = parse_dts(
        r#"export interface Config {
                host: string;
                port: number;
            }
            export declare function get(config: Config, key: keyof Config): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("keyof a fixed object should use the string ABI");
    };
    assert_eq!(
        signature.params,
        vec![
            HirType::Object(vec![
                ("host".into(), HirType::Str),
                ("port".into(), HirType::F64),
            ]),
            HirType::Str,
        ]
    );
    assert_eq!(signature.ret, HirType::Str);
}

#[test]
fn rejects_keyof_collection_from_the_string_fast_path() {
    let funcs = parse_dts("export declare function get(key: keyof string[]): string;").unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn resolves_fixed_object_indexed_access_types() {
    let funcs = parse_dts(
        r#"export interface Config { host: string; port: number; }
            export declare function host(config: Config): Config["host"];
            export declare function port(config: Config): Config["port"];"#,
    )
    .unwrap();
    let Classification::FastPath(host) = classify(&funcs[0]) else {
        panic!("fixed string indexed access should resolve");
    };
    let Classification::FastPath(port) = classify(&funcs[1]) else {
        panic!("fixed number indexed access should resolve");
    };
    assert_eq!(host.ret, HirType::Str);
    assert_eq!(port.ret, HirType::F64);
}

#[test]
fn resolves_indexed_access_after_generic_substitution() {
    let funcs = parse_dts(
        r#"export interface Config { host: string; }
            export interface Field<T> { value: T["host"]; }
            export declare function inspect(field: Field<Config>): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("indexed access should resolve after generic substitution");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![("value".into(), HirType::Str)])]
    );
}

#[test]
fn rejects_unknown_indexed_access_properties() {
    let funcs = parse_dts(
        r#"export interface Config { host: string; }
            export declare function missing(config: Config): Config["missing"];"#,
    )
    .unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn classifies_pick_and_omit_utility_types() {
    let funcs = parse_dts(
        r#"export interface Config {
                host: string;
                port: number;
                secure: boolean;
            }
            export declare function project(
                input: Pick<Config, "host" | "secure">
            ): Omit<Config, "port">;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("finite Pick/Omit utility types should resolve");
    };
    let projected = HirType::Object(vec![
        ("host".into(), HirType::Str),
        ("secure".into(), HirType::Bool),
    ]);
    assert_eq!(signature.params, vec![projected.clone()]);
    assert_eq!(signature.ret, projected);
}

#[test]
fn preserves_optional_object_properties_in_native_layouts() {
    let funcs = parse_dts(
        r#"export interface Config<T> {
                label?: string;
                value?: T;
            }
            export declare function inspect(
                config: Config<number>,
                inline: { enabled?: boolean }
            ): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("optional object properties should use tagged native fields");
    };
    assert_eq!(
        signature.params,
        vec![
            HirType::Object(vec![
                ("label".into(), HirType::Optional(Box::new(HirType::Str)),),
                ("value".into(), HirType::Optional(Box::new(HirType::F64)),),
            ]),
            HirType::Object(vec![(
                "enabled".into(),
                HirType::Optional(Box::new(HirType::Bool)),
            )]),
        ]
    );
}

#[test]
fn classifies_partial_and_required_utility_types() {
    let funcs = parse_dts(
        r#"export interface Config {
                host: string;
                port?: number;
            }
            export declare function normalize(
                patch: Partial<Config>
            ): Required<Config>;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("Partial/Required object types should use tagged native fields");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![
            ("host".into(), HirType::Optional(Box::new(HirType::Str)),),
            ("port".into(), HirType::Optional(Box::new(HirType::F64)),),
        ])]
    );
    assert_eq!(
        signature.ret,
        HirType::Object(vec![
            ("host".into(), HirType::Str),
            ("port".into(), HirType::F64),
        ])
    );
}

#[test]
fn resolves_partial_and_required_after_generic_substitution() {
    let funcs = parse_dts(
        r#"export interface Config { host?: string; }
            export interface Changes<T> {
                patch: Partial<T>;
                complete: Required<T>;
            }
            export declare function inspect(value: Changes<Config>): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("Partial/Required should resolve after generic substitution");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![
            (
                "patch".into(),
                HirType::Object(vec![(
                    "host".into(),
                    HirType::Optional(Box::new(HirType::Str)),
                )]),
            ),
            (
                "complete".into(),
                HirType::Object(vec![("host".into(), HirType::Str)]),
            ),
        ])]
    );
}

#[test]
fn rejects_partial_of_non_object_types() {
    let funcs =
        parse_dts("export declare function inspect(value: Partial<string>): string;").unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn classifies_finite_record_utility_types() {
    let funcs = parse_dts(
        r#"export declare function totals(
                values: Record<"subtotal" | "tax", number>
            ): Record<"label", string>;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("finite Record keys should expand to a fixed object");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![
            ("subtotal".into(), HirType::F64),
            ("tax".into(), HirType::F64),
        ])]
    );
    assert_eq!(
        signature.ret,
        HirType::Object(vec![("label".into(), HirType::Str)])
    );
}

#[test]
fn resolves_finite_record_values_after_generic_substitution() {
    let funcs = parse_dts(
        r#"export interface Fields<T> {
                values: Record<"left" | "right", T>;
            }
            export declare function inspect(value: Fields<boolean>): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("Record values should resolve after generic substitution");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![(
            "values".into(),
            HirType::Object(vec![
                ("left".into(), HirType::Bool),
                ("right".into(), HirType::Bool),
            ]),
        )])]
    );
}

#[test]
fn classifies_dynamic_records_and_index_signatures_as_dictionaries() {
    let funcs = parse_dts(
        "export interface Flags { [key: string]: boolean; } export interface Values<T> { [key: string]: T; } export declare function inspect(values: Record<string, number>, flags: Flags, labels: { [key: string]: string }, generic: Values<number>): string;",
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("dynamic string-keyed values should use the native dictionary ABI");
    };
    assert_eq!(
        signature.params,
        vec![
            HirType::Dictionary(Box::new(HirType::F64)),
            HirType::Dictionary(Box::new(HirType::Bool)),
            HirType::Dictionary(Box::new(HirType::Str)),
            HirType::Dictionary(Box::new(HirType::F64)),
        ]
    );
}

#[test]
fn classifies_non_nullable_utility_types() {
    let funcs = parse_dts(
        "export declare function normalize(value: NonNullable<string | null | undefined>): NonNullable<number | undefined>;",
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("NonNullable should remove null and undefined branches");
    };
    assert_eq!(signature.params, vec![HirType::Str]);
    assert_eq!(signature.ret, HirType::F64);
}

#[test]
fn resolves_non_nullable_after_generic_substitution() {
    let funcs = parse_dts(
        r#"export interface Config { host?: string; }
            export interface Present<T> { value: NonNullable<T>; }
            export declare function inspect(value: Present<Config["host"]>): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("NonNullable should strip a substituted optional wrapper");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![("value".into(), HirType::Str)])]
    );
}

#[test]
fn rejects_non_nullable_without_a_value_branch() {
    let funcs = parse_dts(
        "export declare function impossible(value: NonNullable<null | undefined>): string;",
    )
    .unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn resolves_pick_and_omit_after_generic_substitution() {
    let funcs = parse_dts(
        r#"export interface Config { host: string; port: number; }
            export interface Projection<T> {
                selected: Pick<T, "host">;
                remainder: Omit<T, "host">;
            }
            export declare function inspect(value: Projection<Config>): string;"#,
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("Pick/Omit should resolve after generic substitution");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Object(vec![
            (
                "selected".into(),
                HirType::Object(vec![("host".into(), HirType::Str)]),
            ),
            (
                "remainder".into(),
                HirType::Object(vec![("port".into(), HirType::F64)]),
            ),
        ])]
    );
}

#[test]
fn rejects_unknown_pick_and_omit_keys() {
    let funcs = parse_dts(
        r#"export interface Config { host: string; }
            export declare function inspect(value: Pick<Config, "missing">): string;"#,
    )
    .unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn classifies_typed_callback_parameter_as_fast_path() {
    let funcs = parse_dts("export declare function f(cb: (err: string) => void): void;").unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("expected callback fast path");
    };
    assert_eq!(
        signature.params,
        vec![HirType::Function(
            vec![HirType::Str],
            Box::new(HirType::Void)
        )]
    );
}

#[test]
fn classifies_typed_rest_callback_parameter_as_fast_path() {
    let funcs = parse_dts(
        "export declare function f(cb: (prefix: string, ...values: number[]) => string): void;",
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("expected rest callback fast path");
    };
    assert_eq!(
        signature.params,
        vec![HirType::CallableFunction(
            vec![HirType::Str],
            HirOptionalMask::default(),
            Some(Box::new(HirType::F64)),
            Box::new(HirType::Str),
        )]
    );
}

#[test]
fn classifies_typed_optional_callback_parameter_as_fast_path() {
    let funcs = parse_dts(
        "export declare function f(cb: (prefix: string, value?: number) => string): void;",
    )
    .unwrap();
    let Classification::FastPath(signature) = classify(&funcs[0]) else {
        panic!("expected optional callback fast path");
    };
    assert_eq!(
        signature.params,
        vec![HirType::CallableFunction(
            vec![HirType::Str, HirType::Optional(Box::new(HirType::F64))],
            HirOptionalMask::from_bools(&[false, true]),
            None,
            Box::new(HirType::Str),
        )]
    );
}

/// A plausible subset of a real package's `.d.ts` (uuid-shaped): mixes
/// signatures that should and shouldn't classify as fast path.
#[test]
fn classifies_a_realistic_mixed_dts_file() {
    let source = r#"
            export declare function v4(): string;
            export declare function parse(input: string): number[];
            export declare function validate<T>(input: T): boolean;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 3);
    assert!(matches!(classify(&funcs[0]), Classification::FastPath(_)));
    assert!(matches!(classify(&funcs[1]), Classification::FastPath(_)));
    assert!(matches!(
        classify(&funcs[2]),
        Classification::Fallback { .. }
    ));
}

/// The exact shape found in a real npm package (`qs`): every function
/// lives inside `declare namespace QueryString { ... }` rather than
/// at the top level, with `export = QueryString;` outside it.
#[test]
fn extracts_functions_declared_inside_a_namespace() {
    let source = r#"
            export = QueryString;
            declare namespace QueryString {
                function stringify(obj: number): string;
                function parse(str: string): number;
            }
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 2);
    assert!(funcs.iter().any(|f| f.name == "stringify"));
    assert!(funcs.iter().any(|f| f.name == "parse"));
    // The bare name, not `QueryString.parse` -- that's what
    // `wrap_as_commonjs_module`'s object-export hoisting binds it to.
    assert!(!funcs.iter().any(|f| f.name.contains('.')));
}

#[test]
fn extracts_functions_from_a_nested_namespace() {
    let source = r#"
            declare namespace Outer {
                namespace Inner {
                    function deep(x: number): number;
                }
                function shallow(x: number): number;
            }
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 2);
    assert!(funcs.iter().any(|f| f.name == "deep"));
    assert!(funcs.iter().any(|f| f.name == "shallow"));
}

/// The `export = obj` shape backed by an *interface* rather than a
/// namespace's own `function` declarations -- real-world example:
/// lodash's `declare const _: _.LoDashStatic;` with `interface
/// LoDashStatic { chunk(...): ...; }`, its ~300 methods commonly spread
/// (TS declaration merging) across several files each re-opening the
/// same interface name, the way `thaw_registry`'s triple-slash-reference
/// inlining now brings them all into one file.
#[test]
fn extracts_methods_from_an_export_assignment_interface() {
    let source = r#"
            export = _;
            export as namespace _;
            declare const _: _.LoDashStatic;
            declare namespace _ {
                interface LoDashStatic {}
            }
            declare namespace _ {
                interface LoDashStatic {
                    now(): number;
                }
            }
            declare namespace _ {
                interface LoDashStatic {
                    capitalize(value: string): string;
                }
            }
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 2, "{funcs:?}");
    let now = funcs.iter().find(|f| f.name == "now").unwrap();
    assert!(matches!(classify(now), Classification::FastPath(_)));
    let capitalize = funcs.iter().find(|f| f.name == "capitalize").unwrap();
    assert!(matches!(classify(capitalize), Classification::FastPath(_)));
}

/// An interface used only as some *other* value's parameter/return type
/// (not the type an `export = obj` binding was declared with) must not
/// contribute its own methods as if they were package-level functions --
/// real-world example: `p-limit`'s `Limit` interface, the type its
/// default-exported factory function *returns*, whose `clearQueue()`
/// method exists on that returned object, not on the package itself.
#[test]
fn interface_methods_outside_the_export_assignment_type_are_not_extracted() {
    let source = r#"
            export interface Limit {
                <T>(task: () => T): Promise<T>;
                readonly activeCount: number;
                clearQueue(): void;
            }
            export default function pLimit(concurrency: number): Limit;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 1, "{funcs:?}");
    assert_eq!(funcs[0].name, "pLimit");
}

/// `export declare const NAME: SomeCallableInterface;` -- a factory
/// value bound directly to a name instead of declared `function`. Real-
/// world example: drizzle-orm's `export declare const sqliteTable:
/// SQLiteTableFn;`, where `SQLiteTableFn` is an interface with one call
/// signature per overload (each overload contributes its own
/// `DtsFunction`, same convention as an overloaded interface method).
#[test]
fn extracts_a_callable_const_export_from_its_interfaces_call_signatures() {
    let source = r#"
            export interface Factory {
                (a: string, b: number): string;
                (a: string): string;
            }
            export declare const factory: Factory;
            export declare function plain(x: number): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 3, "{funcs:?}");
    let overloads: Vec<_> = funcs.iter().filter(|f| f.name == "factory").collect();
    assert_eq!(overloads.len(), 2, "{funcs:?}");
    assert!(overloads
        .iter()
        .any(|f| f.params.len() == 2 && f.required_params == 2));
    assert!(overloads
        .iter()
        .any(|f| f.params.len() == 1 && f.required_params == 1));
    assert!(funcs.iter().any(|f| f.name == "plain"));
}

/// A `declare const` whose declared type does *not* resolve to a call-
/// signature interface (an ordinary object shape, or an unknown/unrelated
/// type name) must not contribute any function at all -- confirms the new
/// extraction is conditioned on the referenced interface actually having
/// a call signature, not triggered by every `const` export.
#[test]
fn a_plain_const_export_without_a_callable_interface_type_is_not_extracted() {
    let source = r#"
            export interface PlainShape {
                value: string;
            }
            export declare const notCallable: PlainShape;
            export declare const unknownType: SomethingUndeclared;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert!(funcs.is_empty(), "{funcs:?}");
}

/// `export declare const NAME: (params) => Ret;` -- a factory value with
/// a *direct* inline function type, no interface involved at all. Real-
/// world example: zod v3's `lib/types.d.ts`, `declare const objectType:
/// <T extends ZodRawShape>(shape: T, params?: RawCreateParams) =>
/// ZodObject<...>;`, one such const per zod primitive. Confirms
/// `extract_const_call_signature_decls`'s `Direct` branch (distinct from
/// the `Interface` branch the tests above cover).
#[test]
fn extracts_a_callable_const_export_with_a_direct_inline_function_type() {
    let source = r#"
            export declare const factory: (a: string, b: number) => string;
            export declare function plain(x: number): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 2, "{funcs:?}");
    let factory = funcs.iter().find(|f| f.name == "factory").unwrap();
    assert_eq!(factory.params.len(), 2);
    assert_eq!(factory.required_params, 2);
    assert!(funcs.iter().any(|f| f.name == "plain"));
}

/// The same direct-inline-function-type const, but *bare* (not directly
/// exported) and reached only through a *local* rename-export (no `from`
/// clause) -- the exact shape zod v3 actually uses:
/// `declare const objectType: (...) => ...;` declared plainly, then
/// `export { objectType as object };` elsewhere in the same file.
/// thaw-bridge's own extraction (this test) already handles a bare
/// `Decl::Var` -- this just confirms it stays reachable *by its original
/// name* here (`objectType`, not yet renamed); the rename itself is
/// thaw-registry's job (`install.rs`'s `all_reexported_function_
/// declarations`), exercised separately in that crate's own tests.
#[test]
fn a_bare_callable_const_with_a_direct_function_type_is_still_extracted_by_its_original_name() {
    let source = r#"
            declare const objectType: (shape: string) => string;
            export { objectType as object };
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 1, "{funcs:?}");
    assert_eq!(funcs[0].name, "objectType");
    assert_eq!(funcs[0].params.len(), 1);
}

/// The exact shape found in a real ESM npm package's `.d.ts`
/// (`escape-string-regexp`): `export default function name(...): T;`
/// is a different AST node (`ExportDefaultDecl`) than
/// `export declare function name(...): T;` (`ExportDecl`); without
/// handling it specifically, `parse_dts` found zero functions.
#[test]
fn extracts_a_named_export_default_function() {
    let source = "export default function escapeStringRegexp(string: string): string;";
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].name, "escapeStringRegexp");
    assert!(matches!(classify(&funcs[0]), Classification::FastPath(_)));
}

/// An anonymous `export default function(...): T;` has no name to
/// extract a callable `DtsFunction` under -- must be silently
/// skipped, not panic.
#[test]
fn anonymous_export_default_function_is_skipped_not_panicked_on() {
    let source = "export default function(string: string): string;";
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 0);
}

#[test]
fn namespaced_functions_classify_normally() {
    let source = r#"
            declare namespace Ns {
                function add(a: number, b: number): number;
                function identity<T>(x: T): T;
            }
        "#;
    let funcs = parse_dts(source).unwrap();
    let add = funcs.iter().find(|f| f.name == "add").unwrap();
    let identity = funcs.iter().find(|f| f.name == "identity").unwrap();
    assert!(matches!(classify(add), Classification::FastPath(_)));
    assert!(matches!(
        classify(identity),
        Classification::Fallback { .. }
    ));
}

/// The real `qs` round-trip: a namespaced Fallback function's
/// generated wrapper must actually parse and lower, same bar as
/// every other `generate_shim` round-trip test.
#[test]
fn namespaced_fallback_function_shim_round_trips_through_real_lowering() {
    let source = r#"
            export = QueryString;
            declare namespace QueryString {
                function parse(str: string, options?: object): object;
            }
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 1);
    let shim = generate_shim(&funcs, false, &[], &Default::default());
    assert!(shim.contains("function parse(argsArray: Json): Json {"));

    let program_source = format!(
            "{shim}\nfunction main(): void {{\n    const r = parse(JSON.parse(\"[\\\"a=1\\\"]\"));\n    console.log(String(r));\n}}\n"
        );
    let module = thaw_parser::parse_typescript(&program_source)
        .unwrap_or_else(|e| panic!("generated shim did not parse: {e}\n---\n{program_source}"));
    thaw_hir::lower_module(&module)
        .unwrap_or_else(|e| panic!("generated shim did not lower: {e}\n---\n{program_source}"));
}

#[test]
fn classifies_interface_typed_signature_as_fast_path() {
    let source = r#"
            export interface Point {
                x: number;
                y: number;
            }
            export declare function dist(p: Point): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 1);
    assert_eq!(
        classify(&funcs[0]),
        Classification::FastPath(Box::new(FfiSignature {
            symbol: "dist".into(),
            params: vec![HirType::Object(vec![
                ("x".into(), HirType::F64),
                ("y".into(), HirType::F64),
            ])],
            variadic: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
            ret: HirType::F64,
            error_abi: FfiErrorAbi::Direct,
            return_ownership: FfiOwnership::Borrowed,
            error_ownership: FfiOwnership::Borrowed,
            param_string_abis: vec![FfiStringAbi::NullTerminated; 1],
            return_string_abi: FfiStringAbi::NullTerminated,
            calling_convention: FfiCallingConvention::C,
            aggregate_return_abi: FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
        }))
    );
}

#[test]
fn interfaces_can_reference_each_other_regardless_of_order() {
    // `B` is declared before `A` and refers to it -- the resolver must
    // not depend on source order. Both interfaces are number-only, so
    // this still classifies as fast path (see the next test for what
    // happens when a field is itself a nested object).
    let source = r#"
            export interface B {
                a_sum: number;
                extra: number;
            }
            export interface A {
                x: number;
                y: number;
            }
            export declare function f(b: B): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert!(matches!(classify(&funcs[0]), Classification::FastPath(_)));
}

#[test]
fn nested_object_fields_classify_as_fast_path() {
    // `Line.start` is itself an object (`Point`), not a number --
    // hir_codegen's object layout now supports any representable
    // field type (fields are word-sized regardless of their own
    // type), including a nested object, so this classifies as fast
    // path just like a flat one.
    let source = r#"
            export interface Point {
                x: number;
                y: number;
            }
            export interface Line {
                start: Point;
                length: number;
            }
            export declare function len(l: Line): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(
        classify(&funcs[0]),
        Classification::FastPath(Box::new(FfiSignature {
            symbol: "len".into(),
            params: vec![HirType::Object(vec![
                (
                    "start".into(),
                    HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64),]),
                ),
                ("length".into(), HirType::F64),
            ])],
            variadic: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
            ret: HirType::F64,
            error_abi: FfiErrorAbi::Direct,
            return_ownership: FfiOwnership::Borrowed,
            error_ownership: FfiOwnership::Borrowed,
            param_string_abis: vec![FfiStringAbi::NullTerminated; 1],
            return_string_abi: FfiStringAbi::NullTerminated,
            calling_convention: FfiCallingConvention::C,
            aggregate_return_abi: FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
        }))
    );
}

#[test]
fn falls_back_on_self_referential_interface_without_breaking_other_functions() {
    let source = r#"
            export interface Node {
                value: number;
                next: Node;
            }
            export declare function head(n: Node): number;
            export declare function add(a: number, b: number): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
    assert!(matches!(classify(&funcs[1]), Classification::FastPath(_)));
}

#[test]
fn classifies_generic_interface_instantiation_as_fast_path() {
    let source = r#"
            export interface Box<T> {
                value: T;
            }
            export declare function unwrap(b: Box<number>): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(
        classify(&funcs[0]),
        Classification::FastPath(Box::new(FfiSignature {
            symbol: "unwrap".into(),
            params: vec![HirType::Object(vec![("value".into(), HirType::F64)])],
            variadic: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
            ret: HirType::F64,
            error_abi: FfiErrorAbi::Direct,
            return_ownership: FfiOwnership::Borrowed,
            error_ownership: FfiOwnership::Borrowed,
            param_string_abis: vec![FfiStringAbi::NullTerminated; 1],
            return_string_abi: FfiStringAbi::NullTerminated,
            calling_convention: FfiCallingConvention::C,
            aggregate_return_abi: FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
        }))
    );
}

#[test]
fn falls_back_on_wrong_number_of_generic_type_arguments() {
    let source = r#"
            export interface Pair<A, B> {
                first: A;
                second: B;
            }
            export declare function f(p: Pair<number>): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn falls_back_on_self_referential_generic_interface() {
    let source = r#"
            export interface Node<T> {
                value: T;
                next: Node<T>;
            }
            export declare function head(n: Node<number>): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn classifies_extends_as_fast_path_with_base_fields_prepended() {
    let source = r#"
            export interface Shape {
                color: number;
            }
            export interface Circle extends Shape {
                radius: number;
            }
            export declare function area(c: Circle): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(
        classify(&funcs[0]),
        Classification::FastPath(Box::new(FfiSignature {
            symbol: "area".into(),
            params: vec![HirType::Object(vec![
                ("color".into(), HirType::F64),
                ("radius".into(), HirType::F64),
            ])],
            variadic: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
            ret: HirType::F64,
            error_abi: FfiErrorAbi::Direct,
            return_ownership: FfiOwnership::Borrowed,
            error_ownership: FfiOwnership::Borrowed,
            param_string_abis: vec![FfiStringAbi::NullTerminated; 1],
            return_string_abi: FfiStringAbi::NullTerminated,
            calling_convention: FfiCallingConvention::C,
            aggregate_return_abi: FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
        }))
    );
}

#[test]
fn falls_back_on_extends_field_collision() {
    let source = r#"
            export interface A { x: number; }
            export interface B extends A { x: number; }
            export declare function f(b: B): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert!(matches!(
        classify(&funcs[0]),
        Classification::Fallback { .. }
    ));
}

#[test]
fn generates_ambient_declaration_for_fast_path_function() {
    let funcs = parse_dts("export declare function add(a: number, b: number): number;").unwrap();
    assert_eq!(
        generate_shim(&funcs, true, &[], &Default::default()),
        "declare function add(a: number, b: number): number;\n"
    );
}

#[test]
fn generates_object_typed_ambient_declaration() {
    let funcs = parse_dts(
        r#"export interface Point { x: number; y: number; }
            export declare function dist(p: Point): number;"#,
    )
    .unwrap();
    assert_eq!(
        generate_shim(&funcs, true, &[], &Default::default()),
        "declare function dist(p: { x: number; y: number }): number;\n"
    );
}

/// The actual bug: a real npm package fetched via `thaw registry
/// add` (e.g. date-fns's `daysToWeeks(days: number): number`) is
/// pure JS with no native library at all, but classifies FastPath on
/// type shape alone. Emitting the ambient `declare function` anyway
/// produced a real, reproduced `undefined reference` linker error --
/// `native_lib_available: false` must downgrade it to a Fallback
/// wrapper instead, since a working JS implementation is right there.
#[test]
fn downgrades_fast_path_to_fallback_when_no_native_lib_is_available() {
    let funcs = parse_dts("export declare function daysToWeeks(days: number): number;").unwrap();
    let shim = generate_shim(&funcs, false, &[], &Default::default());
    assert!(
        !shim.contains("declare function"),
        "must not emit an ambient FFI declaration with nothing to link against, got:\n{shim}"
    );
    assert!(shim.contains("function daysToWeeks(argsArray: Json): Json {"));
    assert!(shim.contains(r#"return callDynamic("daysToWeeks", argsArray);"#));
}

#[test]
fn native_lib_available_true_keeps_fast_path_as_before() {
    let funcs = parse_dts("export declare function add(a: number, b: number): number;").unwrap();
    assert_eq!(
        generate_shim(&funcs, true, &[], &Default::default()),
        "declare function add(a: number, b: number): number;\n"
    );
}

#[test]
fn effective_classifications_downgrades_fast_path_when_native_lib_unavailable() {
    let funcs = parse_dts("export declare function add(a: number, b: number): number;").unwrap();
    let result = effective_classifications(&funcs, false);
    assert_eq!(result.len(), 1);
    assert!(matches!(result[0].1, Classification::Fallback { .. }));
}

#[test]
fn effective_classifications_leaves_fallback_alone_regardless_of_native_lib() {
    let funcs = parse_dts("export declare function identity<T>(x: T): T;").unwrap();
    let with_native = effective_classifications(&funcs, true);
    let without_native = effective_classifications(&funcs, false);
    assert!(matches!(with_native[0].1, Classification::Fallback { .. }));
    assert!(matches!(
        without_native[0].1,
        Classification::Fallback { .. }
    ));
}

#[test]
fn generates_call_dynamic_wrapper_for_fallback_function() {
    let funcs = parse_dts("export declare function identity<T>(x: T): T;").unwrap();
    let shim = generate_shim(&funcs, true, &[], &Default::default());
    assert!(shim.contains("// Fallback (QuickJS-NG):"));
    assert!(shim.contains("function identity(argsArray: Json): Json {"));
    assert!(shim.contains(r#"return callDynamic("identity", argsArray);"#));
}

#[test]
fn classify_all_collapses_a_single_signature_exactly_like_classify() {
    let funcs = parse_dts("export declare function add(a: number, b: number): number;").unwrap();
    let grouped = classify_all(&funcs);
    assert_eq!(grouped.len(), 1);
    assert_eq!(grouped[0], ("add".to_string(), classify(&funcs[0])));
}

/// The exact shape found in a real npm package (`@types/ms`): two
/// `declare function ms(...)` overloads, one classifying FastPath on
/// its own and one Fallback. Thaw can't represent two native
/// signatures under one FFI symbol, so the whole name must fall back.
#[test]
fn classify_all_falls_back_an_overload_set_even_if_one_member_is_fast_path() {
    let dts = r#"
            declare function ms(value: number, options?: { long: boolean }): string;
            declare function ms(value: string): number;
        "#;
    let funcs = parse_dts(dts).unwrap();
    assert_eq!(funcs.len(), 2, "both overloads should be extracted");

    let grouped = classify_all(&funcs);
    assert_eq!(grouped.len(), 1, "one name -> one classification, not two");
    let (name, classification) = &grouped[0];
    assert_eq!(name, "ms");
    assert!(
        matches!(classification, Classification::Fallback { .. }),
        "an overloaded name must fall back even if one overload alone would be FastPath"
    );
}

/// Same rule even when *every* overload individually classifies
/// FastPath: Thaw still can't pick one signature over the other, so
/// this must not silently choose either.
#[test]
fn classify_all_falls_back_when_every_overload_is_individually_fast_path() {
    let dts = r#"
            declare function f(a: number): number;
            declare function f(a: string): string;
        "#;
    let funcs = parse_dts(dts).unwrap();
    let grouped = classify_all(&funcs);
    assert_eq!(grouped.len(), 1);
    assert!(matches!(grouped[0].1, Classification::Fallback { .. }));
}

/// Regression test for the actual bug: before `classify_all`,
/// `generate_shim` emitted one entry per raw `DtsFunction`, so an
/// overloaded name produced two colliding top-level declarations
/// (`declare function ms(...)` and `function ms(argsArray...) {...}`)
/// that thaw-hir/codegen resolved silently and order-dependently.
#[test]
fn generate_shim_emits_exactly_one_declaration_for_an_overloaded_name() {
    let dts = r#"
            declare function ms(value: number, options?: { long: boolean }): string;
            declare function ms(value: string): number;
        "#;
    let funcs = parse_dts(dts).unwrap();
    let shim = generate_shim(&funcs, true, &[], &Default::default());

    assert_eq!(
        shim.matches("function ms").count(),
        1,
        "exactly one top-level declaration named `ms`, got:\n{shim}"
    );
    assert!(shim.contains("// Fallback (QuickJS-NG):"));
    assert!(shim.contains(r#"return callDynamic("ms", argsArray);"#));
}

/// A name in `qualified` with `suppress_bare: true` is emitted only
/// under its alias, calling `callDynamic` with the qualified key --
/// not the bare name -- and the bare name isn't emitted as a
/// declaration at all (thaw-cli sets this once it's decided the bare
/// identifier must not be directly callable, to force qualified
/// syntax after an actual cross-package collision).
#[test]
fn generates_qualified_alias_for_a_cross_package_colliding_name() {
    let funcs = parse_dts("export declare function stringify<T>(x: T): T;").unwrap();
    let qualified = [QualifiedFallback {
        name: "stringify".to_string(),
        alias: "qs_stringify".to_string(),
        qualified_key: "qs::stringify".to_string(),
        suppress_bare: true,
    }];
    let shim = generate_shim(&funcs, true, &qualified, &Default::default());

    assert!(shim.contains("function qs_stringify(argsArray: Json): Json {"));
    assert!(shim.contains(r#"return callDynamic("qs::stringify", argsArray);"#));
    assert!(!shim.contains("function stringify(argsArray: Json): Json {"));
}

/// `suppress_bare: false` (the common, non-colliding case) emits
/// *both* the bare name and the qualified alias -- a name stays
/// callable either way (`parse(x)` or `qs.parse(x)`).
#[test]
fn generates_both_bare_and_qualified_alias_when_not_suppressed() {
    let funcs = parse_dts("export declare function identity<T>(x: T): T;").unwrap();
    let qualified = [QualifiedFallback {
        name: "identity".to_string(),
        alias: "qs_identity".to_string(),
        qualified_key: "qs::identity".to_string(),
        suppress_bare: false,
    }];
    let shim = generate_shim(&funcs, true, &qualified, &Default::default());

    assert!(shim.contains("function qs_identity(argsArray: Json): Json {"));
    assert!(shim.contains(r#"return callDynamic("qs::identity", argsArray);"#));
    assert!(shim.contains("function identity(argsArray: Json): Json {"));
    assert!(shim.contains(r#"return callDynamic("identity", argsArray);"#));
}

/// The generated shim isn't just plausible-looking text -- it must
/// actually be valid, lowerable Thaw source. Compiles a realistic
/// mixed `.d.ts` (fast path + fallback functions) into a shim, appends
/// a `main` that calls both, and runs it through the real
/// `thaw-parser`/`thaw-hir` pipeline used everywhere else.
#[test]
fn generated_shim_round_trips_through_real_lowering() {
    let dts = r#"
            export declare function add(a: number, b: number): number;
            export declare function identity<T>(x: T): T;
        "#;
    let funcs = parse_dts(dts).unwrap();
    let shim = generate_shim(&funcs, true, &[], &Default::default());

    let program_source = format!(
            "{shim}\nfunction main(): void {{\n    console.log(add(2, 3));\n    const r = identity(JSON.parse(\"[1]\"));\n    console.log(Number(r));\n}}\n"
        );

    let module = thaw_parser::parse_typescript(&program_source)
        .unwrap_or_else(|e| panic!("generated shim did not parse: {e}\n---\n{program_source}"));
    let program = thaw_hir::lower_module(&module)
        .unwrap_or_else(|e| panic!("generated shim did not lower: {e}\n---\n{program_source}"));

    // `add` is ambient (fast path) -> extern_functions; `identity`'s
    // wrapper and `main` both have real bodies -> functions.
    assert_eq!(program.extern_functions.len(), 1);
    assert_eq!(program.extern_functions[0].symbol, "add");
    assert_eq!(program.functions.len(), 2);
    assert!(program.functions.iter().any(|f| f.name == "identity"));
    assert!(program.functions.iter().any(|f| f.name == "main"));
}

#[test]
fn generate_module_init_is_empty_for_no_bundles() {
    assert_eq!(generate_module_init(&[]), "");
}

#[test]
fn generates_load_script_call_per_bundle() {
    let bundles = [
        ModuleBundle {
            package_name: "left-pad",
            js_source: "function pad(s) { return s; }",
            fallback_names: &[],
            qualified_aliases: &[],
            nested_namespace_aliases: &[],
        },
        ModuleBundle {
            package_name: "is-odd",
            js_source: "function isOdd(n) { return n % 2 === 1; }",
            fallback_names: &[],
            qualified_aliases: &[],
            nested_namespace_aliases: &[],
        },
    ];
    let init = generate_module_init(&bundles);
    assert!(init.starts_with("function __thaw_module_init(): void {\n"));
    assert!(init.contains("function pad(s) { return s; }"));
    assert!(init.contains("function isOdd(n) { return n % 2 === 1; }"));
    // Loaded in the given order.
    assert!(init.find("left-pad").unwrap() < init.find("is-odd").unwrap());
}

#[test]
fn escapes_quotes_and_newlines_in_bundled_source() {
    let bundles = [ModuleBundle {
        package_name: "pkg",
        js_source: "function f() {\n  return \"a\\b\";\n}",
        fallback_names: &[],
        qualified_aliases: &[],
        nested_namespace_aliases: &[],
    }];
    let init = generate_module_init(&bundles);
    // The wrapper adds its own quotes/backslashes/newlines too; this
    // just confirms the *bundle's own* problematic characters survived
    // escaping correctly once embedded inside the wrapped script.
    assert!(init.contains(r#"return \"a\\b\";\n"#));
}

#[test]
fn wraps_real_commonjs_source_and_binds_default_export() {
    // The exact shape of left-pad's actual published `index.js`:
    // `module.exports = leftPad;`, no named exports object.
    let js_source = "module.exports = function leftPad(str) { return str; };";
    let wrapped = wrap_as_commonjs_module(js_source, &["leftPad".to_string()], &[]);
    assert!(wrapped.contains("globalThis.module = { exports: {} };"));
    assert!(wrapped.contains("globalThis.require ="));
    assert!(wrapped.contains(js_source));
    assert!(wrapped.contains("globalThis.leftPad = module.exports;"));
}

#[test]
fn native_class_proxies_retain_js_properties_and_release_native_handles() {
    let wrapped = wrap_as_commonjs_module("module.exports = {};", &[], &[]);
    assert!(wrapped.contains("__thaw_napi_proxy_finalizers.register(proxy"));
    assert!(wrapped.contains("'release_handle'"));
    assert!(wrapped.contains("Reflect.set(_, name, value, receiver)"));
    assert!(wrapped.contains("result.value['$__thaw_napi_undefined$'] === true"));
}

/// The exact shape thaw-registry's ESM rewrite produces for a real
/// ESM package (`escape-string-regexp`'s `export default function
/// escapeStringRegexp(){}`): `module.exports.default = <fn>`, not
/// `module.exports = <fn>` directly. Runs through real QuickJS-NG to
/// confirm the Fallback name actually ends up callable, not just
/// that the generated text looks plausible.
#[test]
fn binds_esm_default_export_under_the_fallback_name() {
    use std::ffi::{CStr, CString};

    let js_source = "module.exports.__esModule = true;\nmodule.exports.default = function escapeIt(s) { return '[' + s + ']'; };";
    let wrapped = wrap_as_commonjs_module(js_source, &["escapeIt".to_string()], &[]);

    let source = CString::new(wrapped).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "failed to load"
    );

    let func = CString::new("escapeIt").unwrap();
    let args = CString::new("[\"hi\"]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(result, "\"[hi]\"");
}

/// A key whose assignment to `globalThis` throws (e.g. a schema-builder
/// function literally named `undefined`, `z.undefined()` -- real zod
/// exports exactly this, and `globalThis.undefined` is non-writable, so
/// `globalThis["undefined"] = ...` throws `TypeError: 'undefined' is
/// read-only` in strict mode) used to abort the whole `module.exports`
/// -> `globalThis` copy loop, since `for...in` enumerates in insertion
/// order and one throw stopped the loop entirely -- every export
/// enumerated *after* the offending key, including ones with nothing to
/// do with it, silently never reached `globalThis` at all. Runs through
/// real QuickJS-NG (not just checking the generated text) to confirm a
/// later, unrelated export actually stays callable.
#[test]
fn a_key_that_cannot_bind_to_globalthis_does_not_block_later_exports() {
    use std::ffi::{CStr, CString};

    let js_source = "module.exports.undefined = function() { return 'nope'; };\n\
                      module.exports.after = function(s) { return '[' + s + ']'; };";
    let wrapped = wrap_as_commonjs_module(js_source, &["after".to_string()], &[]);

    let source = CString::new(wrapped).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "failed to load"
    );

    let func = CString::new("after").unwrap();
    let args = CString::new("[\"hi\"]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(result, "\"[hi]\"");
}

#[test]
fn bare_global_function_bundle_is_unaffected_by_commonjs_wrapping() {
    // A hand-authored bundle with no `module.exports` at all (this
    // session's registry examples before real npm packages were
    // tested) must keep defining a plain global function, not get
    // hidden inside a nested scope.
    let wrapped = wrap_as_commonjs_module(
        "function greet(name) { return 'hi, ' + name; }",
        &["greet".to_string()],
        &[],
    );
    assert!(wrapped.contains("function greet(name) { return 'hi, ' + name; }"));
    // Not wrapped in an extra IIFE/function around the source itself.
    assert!(!wrapped.contains("(function(module, exports, require)"));
}

/// The exact pattern found in a real npm package (`@hapi/hoek`):
/// code that *guards* a Node-only global before using it (`Buffer &&
/// Buffer.isBuffer(x)`) still throws `ReferenceError: Buffer is not
/// defined` if `Buffer` was never declared anywhere -- referencing an
/// undeclared bare identifier throws regardless of the guard's
/// intent. Must load successfully and take the guard's "not
/// available" branch instead.
#[test]
fn guarded_buffer_reference_does_not_throw() {
    use std::ffi::{CStr, CString};

    let wrapped = wrap_as_commonjs_module(
            "module.exports = function checkBuffer(x) { return (Buffer && Buffer.isBuffer(x)) || false; };",
            &["checkBuffer".to_string()],
            &[],
        );

    let source = CString::new(wrapped).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "failed to load"
    );

    let func = CString::new("checkBuffer").unwrap();
    let args = CString::new("[1]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        result, "false",
        "the guard should take its no-Buffer branch, not throw"
    );
}

/// The exact pattern found in the same real npm package (`@hapi/hoek`):
/// `URL.prototype` accessed *unconditionally* (as a lookup-table key,
/// not behind a truthiness guard) -- `undefined` doesn't survive a
/// `.prototype` property access the way it survives `Buffer && ...`,
/// so `URL` needs an actual (empty) constructor stand-in instead.
#[test]
fn unguarded_url_prototype_access_does_not_throw() {
    use std::ffi::CString;

    let wrapped = wrap_as_commonjs_module(
        "module.exports = function getIt() { return typeof URL.prototype; };",
        &["getIt".to_string()],
        &[],
    );

    let source = CString::new(wrapped).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "failed to load"
    );
}

/// The exact pattern chasing a real native addon's load path
/// (`bcrypt`, via its `node-gyp-build` dependency's real, unmodified
/// `node-gyp-build.js`): several bare `process.*` reads with no
/// `require('process')` and no guard at all -- `process` needs to
/// exist as an ambient *global*, not just as thaw-registry's
/// requirable `process` module, or this throws `ReferenceError:
/// process is not defined` the same way an unguarded `Buffer`/`URL`
/// reference would without their global stand-ins.
#[test]
fn unguarded_process_global_reference_does_not_throw() {
    use std::ffi::{CStr, CString};

    let wrapped = wrap_as_commonjs_module(
            "module.exports = function readIt() {\n\
             \x20\x20var vars = (process.config && process.config.variables) || {};\n\
             \x20\x20var abi = process.versions.modules;\n\
             \x20\x20var uv = (process.versions.uv || '').split('.')[0];\n\
             \x20\x20return typeof process.env + ',' + typeof process.execPath + ',' + typeof __dirname + ',' + typeof __filename;\n\
             };",
            &["readIt".to_string()],
            &[],
        );

    let source = CString::new(wrapped).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "failed to load"
    );

    let func = CString::new("readIt").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(result, "\"object,string,string,string\"");
}

/// Same bar as `generated_shim_round_trips_through_real_lowering`: the
/// generated `__thaw_module_init` must actually be valid, lowerable
/// Thaw source, not just plausible text.
#[test]
fn generated_module_init_round_trips_through_real_lowering() {
    let fallback_names = vec!["greet".to_string()];
    let init = generate_module_init(&[ModuleBundle {
        package_name: "greeter",
        js_source: "function greet(){return 'hi';}",
        fallback_names: &fallback_names,
        qualified_aliases: &[],
        nested_namespace_aliases: &[],
    }]);
    let program_source = format!(
            "{init}\nfunction main(): void {{\n    console.log(String(callDynamic(\"greet\", JSON.parse(\"[]\"))));\n}}\n"
        );

    let module = thaw_parser::parse_typescript(&program_source).unwrap_or_else(|e| {
        panic!("generated module_init did not parse: {e}\n---\n{program_source}")
    });
    let program = thaw_hir::lower_module(&module).unwrap_or_else(|e| {
        panic!("generated module_init did not lower: {e}\n---\n{program_source}")
    });

    assert_eq!(program.functions.len(), 2);
    assert!(program
        .functions
        .iter()
        .any(|f| f.name == "__thaw_module_init"));
    assert!(program.functions.iter().any(|f| f.name == "main"));
}

/// A bundle's `qualified_aliases` capture the bare name under the
/// qualified key *immediately* after that bundle's own `loadScript`
/// -- before a later, colliding package's `loadScript` gets a chance
/// to overwrite the bare name.
#[test]
fn generate_module_init_captures_qualified_aliases_right_after_load() {
    let qs_fallback = vec!["stringify".to_string()];
    let qs_aliases = vec![("stringify".to_string(), "qs::stringify".to_string())];
    let hoek_fallback = vec!["stringify".to_string()];
    let hoek_aliases = vec![("stringify".to_string(), "hoek::stringify".to_string())];
    let bundles = [
        ModuleBundle {
            package_name: "qs",
            js_source: "module.exports = { stringify: function(x) { return 'qs:' + x; } };",
            fallback_names: &qs_fallback,
            qualified_aliases: &qs_aliases,
            nested_namespace_aliases: &[],
        },
        ModuleBundle {
            package_name: "@hapi/hoek",
            js_source: "module.exports = { stringify: function(x) { return 'hoek:' + x; } };",
            fallback_names: &hoek_fallback,
            qualified_aliases: &hoek_aliases,
            nested_namespace_aliases: &[],
        },
    ];
    let init = generate_module_init(&bundles);

    // Each capture line must appear *between* its own package's
    // loadScript and the next package's, not after both have loaded.
    // The capture JS is embedded (and so escaped) inside its own
    // `loadScript("...")` TS string literal -- a literal backslash
    // precedes each quote in the *generated Thaw source text*, not
    // just a bare `"`.
    let qs_load = init.find("// qs").unwrap();
    let qs_capture = init.find(r#"globalThis[\"qs::stringify\"]"#).unwrap();
    let hoek_load = init.find("// @hapi/hoek").unwrap();
    let hoek_capture = init.find(r#"globalThis[\"hoek::stringify\"]"#).unwrap();
    assert!(
        qs_load < qs_capture,
        "qs's capture must come after qs's own load"
    );
    assert!(
        qs_capture < hoek_load,
        "qs's capture must come before hoek's load"
    );
    assert!(hoek_load < hoek_capture);
}

/// Found by running the classifier against a real npm package's
/// `.d.ts` corpus (date-fns): before `describe_ts_type`, this reason
/// was an unreadable `{:?}` dump of the full nested AST (spans,
/// `Box`es, and all) instead of anything resembling what a developer
/// wrote.
#[test]
fn fallback_reason_renders_union_types_readably() {
    let funcs = parse_dts("export declare function f(x: string | number): void;").unwrap();
    let Classification::Fallback { reason, .. } = classify(&funcs[0]) else {
        panic!("expected Fallback");
    };
    assert_eq!(
        reason,
        "parameter `x` uses a tagged union without an explicit C ABI"
    );
}

/// The exact shape found in date-fns v4's real `.d.ts` files (e.g.
/// `addDays`'s `date: DateArg<DateType> & {}` parameter).
#[test]
fn fallback_reason_renders_intersection_and_generic_types_readably() {
    let funcs = parse_dts("export declare function f(x: DateArg<Date> & {}): void;").unwrap();
    let Classification::Fallback { reason, .. } = classify(&funcs[0]) else {
        panic!("expected Fallback");
    };
    assert_eq!(
        reason,
        "parameter `x`: unsupported type `DateArg<Date> & {}`"
    );
}

#[test]
fn fallback_reason_renders_unsupported_generic_callbacks_readably() {
    let funcs = parse_dts("export declare function f(cb: <T>(err: T) => void): void;").unwrap();
    let Classification::Fallback { reason, .. } = classify(&funcs[0]) else {
        panic!("expected Fallback");
    };
    assert_eq!(
        reason,
        "parameter `cb`: generic callback types are not supported"
    );
}

#[test]
fn fallback_reason_renders_unsupported_keywords_by_name() {
    let funcs = parse_dts("export declare function f(x: any): void;").unwrap();
    let Classification::Fallback { reason, .. } = classify(&funcs[0]) else {
        panic!("expected Fallback");
    };
    assert_eq!(reason, "parameter `x`: `any` is not supported");
}

#[test]
fn extracts_class_constructors_methods_properties_and_overloads() {
    let classes = parse_dts_classes(
        r#"
            export class Database extends EventEmitter {
                readonly open: boolean;
                constructor(filename: string);
                constructor(filename: string, mode: number);
                close(callback?: (error: Error | null) => void): void;
                sum(initial: number, ...values: number[]): number;
                run(sql: string): this;
                run(sql: string, params: any[]): this;
                static verbose(): Database;
                get name(): string;
            }
            "#,
    )
    .unwrap();
    assert_eq!(classes.len(), 1);
    let database = &classes[0];
    assert_eq!(database.name, "Database");
    assert_eq!(database.extends.as_deref(), Some("EventEmitter"));
    assert_eq!(database.constructors.len(), 2);
    assert!(database
        .constructors
        .iter()
        .all(|constructor| constructor.overloaded));
    let runs = database
        .methods
        .iter()
        .filter(|method| method.name == "run")
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|method| method.overloaded));
    let close = database
        .methods
        .iter()
        .find(|method| method.name == "close")
        .unwrap();
    assert_eq!(close.required_params, 0);
    assert_eq!(
        close.params[0].1,
        DtsType::Native(HirType::Function(
            vec![HirType::Json],
            Box::new(HirType::Void)
        ))
    );
    assert!(runs
        .iter()
        .all(|method| method.required_params == method.params.len()));
    let sum = database
        .methods
        .iter()
        .find(|method| method.name == "sum")
        .unwrap();
    assert_eq!(
        sum.params,
        vec![("initial".into(), DtsType::Native(HirType::F64))]
    );
    assert_eq!(
        sum.rest_param,
        Some(("values".into(), DtsType::Native(HirType::F64)))
    );
    assert!(database
        .methods
        .iter()
        .any(|method| { method.name == "verbose" && method.is_static && !method.overloaded }));
    assert!(database
        .methods
        .iter()
        .any(|method| method.name == "name" && method.kind == DtsMethodKind::Getter));
    assert_eq!(database.properties.len(), 1);
    assert_eq!(database.properties[0].name, "open");
    assert!(database.properties[0].readonly);
}

#[test]
fn extracts_quoted_and_numeric_class_property_keys() {
    let classes = parse_dts_classes(
        r#"export class Metadata {
                "content-type": string;
                200: boolean;
                "optional-key"?: number;
            }"#,
    )
    .unwrap();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].properties.len(), 3);
    assert_eq!(classes[0].properties[0].name, "content-type");
    assert_eq!(classes[0].properties[1].name, "200");
    assert_eq!(
        classes[0].properties[2].ty,
        DtsType::Native(HirType::Optional(Box::new(HirType::F64)))
    );
}

#[test]
fn records_optional_and_default_constructor_arities() {
    let classes = parse_dts_classes(
        r#"export class Client {
                constructor(url: string, timeout?: number, retries?: number);
            }
            export class Cache {
                constructor(size: number, enabled = true);
            }"#,
    )
    .unwrap();
    assert_eq!(classes[0].constructors[0].required_params, 1);
    assert_eq!(classes[0].constructors[0].params.len(), 3);
    assert_eq!(classes[1].constructors[0].required_params, 1);
    assert_eq!(classes[1].constructors[0].params.len(), 2);
}

#[test]
fn expands_inherited_external_class_members() {
    let classes = parse_dts_classes(
        r#"export class Base {
                constructor(value: number);
                base(value: number): number;
                inherited(): string;
                static version(): number;
                readonly id: number;
            }
            export class Derived extends Base {
                constructor(value: number, label?: string);
                base(value: string): string;
                own(): boolean;
            }
            export class GrandChild extends Derived {}"#,
    )
    .unwrap();
    let derived = classes
        .iter()
        .find(|class| class.name == "Derived")
        .unwrap();
    assert_eq!(derived.constructors.len(), 1);
    assert_eq!(
        derived
            .methods
            .iter()
            .filter(|method| method.name == "base")
            .count(),
        1
    );
    assert!(derived
        .methods
        .iter()
        .any(|method| method.name == "inherited"));
    assert!(derived
        .methods
        .iter()
        .any(|method| method.name == "version" && method.is_static));
    assert!(derived
        .properties
        .iter()
        .any(|property| property.name == "id"));

    let grand = classes
        .iter()
        .find(|class| class.name == "GrandChild")
        .unwrap();
    assert_eq!(grand.constructors.len(), 1);
    assert_eq!(grand.constructors[0].required_params, 1);
    assert_eq!(grand.constructors[0].params.len(), 2);
    assert!(grand.methods.iter().any(|method| method.name == "own"));
    assert!(grand
        .methods
        .iter()
        .any(|method| method.name == "inherited"));
    assert!(grand
        .properties
        .iter()
        .any(|property| property.name == "id"));
}

#[test]
fn excludes_inaccessible_external_class_members_and_constructors() {
    let classes = parse_dts_classes(
        r#"export class Base {
                protected hidden(): number;
                private secret: string;
                visible(): boolean;
            }
            export class Derived extends Base {}
            export class Locked {
                private constructor(value: number);
                static create(): Locked;
            }"#,
    )
    .unwrap();
    let derived = classes
        .iter()
        .find(|class| class.name == "Derived")
        .unwrap();
    assert!(derived
        .methods
        .iter()
        .any(|method| method.name == "visible"));
    assert!(!derived.methods.iter().any(|method| method.name == "hidden"));
    assert!(!derived
        .properties
        .iter()
        .any(|property| property.name == "secret"));

    let locked = classes.iter().find(|class| class.name == "Locked").unwrap();
    assert!(!locked.constructible);
    assert!(locked.constructors.is_empty());
    assert!(locked
        .methods
        .iter()
        .any(|method| method.name == "create" && method.is_static));
}

#[test]
fn excludes_abstract_external_class_constructors_but_inherits_members() {
    let classes = parse_dts_classes(
        r#"export abstract class Service {
                constructor(name: string);
                abstract start(): boolean;
                stop(): void;
            }
            export class ConcreteService extends Service {
                constructor(name: string);
                start(): boolean;
            }"#,
    )
    .unwrap();
    let abstract_class = classes
        .iter()
        .find(|class| class.name == "Service")
        .unwrap();
    assert!(!abstract_class.constructible);
    assert_eq!(abstract_class.constructors.len(), 1);

    let concrete = classes
        .iter()
        .find(|class| class.name == "ConcreteService")
        .unwrap();
    assert!(concrete.constructible);
    assert!(concrete.methods.iter().any(|method| method.name == "start"));
    assert!(concrete.methods.iter().any(|method| method.name == "stop"));
}

/// The actual regression this was validated against: a real date-fns
/// function (`milliseconds({ years, months, ... }: Duration)`) uses a
/// destructured parameter, which `parse_dts` used to reject by
/// returning `Err` for the *whole file* -- silently discarding every
/// other function defined alongside it. It must now still show up
/// (correctly, as Fallback), and every sibling function in the same
/// file must survive.
#[test]
fn destructured_parameter_falls_back_without_dropping_sibling_functions() {
    let source = r#"
            export declare function add(a: number, b: number): number;
            export declare function milliseconds({ hours, minutes }: Duration): number;
            export declare function subtract(a: number, b: number): number;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(
        funcs.len(),
        3,
        "one bad parameter pattern must not delete sibling functions"
    );

    assert!(matches!(
        classify(funcs.iter().find(|f| f.name == "add").unwrap()),
        Classification::FastPath(_)
    ));
    assert!(matches!(
        classify(funcs.iter().find(|f| f.name == "subtract").unwrap()),
        Classification::FastPath(_)
    ));

    let Classification::Fallback { reason, .. } =
        classify(funcs.iter().find(|f| f.name == "milliseconds").unwrap())
    else {
        panic!("expected Fallback");
    };
    assert!(reason.contains("unsupported parameter pattern"));
}
