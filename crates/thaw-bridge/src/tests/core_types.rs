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
