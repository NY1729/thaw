#[test]
fn generates_native_addon_wrapper_and_module_initializer() {
    let funcs = parse_dts("export declare function add(args: any): any;").unwrap();
    let shim = generate_native_addon_shim(&funcs, &[], &Default::default());
    assert!(shim.contains(r#"return callNativeAddon("add", argsArray);"#));
    let init = generate_native_addon_init(&[NativeAddon {
        package_name: "native-add",
        bytes: &[0xde, 0xad, 0xbe, 0xef],
        dependencies: vec![&[0xca, 0xfe]],
        root_export: None,
    }]);
    assert!(init.contains("function __thaw_native_module_init(): void"));
    assert!(init.contains(r#"loadNativeAddonEmbedded("deadbeef", "");"#));
    assert!(init.contains(r#"loadNativeSharedLibraryEmbedded("cafe");"#));
    let init = generate_native_addon_path_init(&[NativeAddonPath {
        package_name: "native-add",
        path: "/registry/native-add/native.node",
        dependencies: vec!["/registry/native-add/libvalue.so"],
        root_export: Some("NativeAdd"),
    }]);
    assert!(init.contains(r#"loadNativeSharedLibrary("/registry/native-add/libvalue.so");"#));
    assert!(init.contains(r#"loadNativeAddon("/registry/native-add/native.node", "NativeAdd");"#));
}

#[test]
fn compresses_large_embedded_native_payloads() {
    let bytes = vec![0x5a; 4096];
    let init = generate_native_addon_init(&[NativeAddon {
        package_name: "large-native",
        bytes: &bytes,
        dependencies: vec![],
        root_export: None,
    }]);
    assert!(init.contains(r#"loadNativeAddonEmbedded("gz:"#));
    assert!(init.len() < bytes.len());
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
fn expands_generic_callback_aliases_for_call_site_specialization() {
    let functions = parse_dts(
        "type Iterator<T, Result> = (value: T, index: number, values: T[]) => Result;\nexport declare function map<T, Result>(values: T[], iterator: Iterator<T, Result>): Result[];",
    )
    .unwrap();
    let generic = functions[0].generic.as_ref().unwrap();
    assert_eq!(
        generic.contextual_param_types,
        vec!["T[]", "(value: T, index: number, values: T[]) => Result"]
    );
}

#[test]
fn preserves_indexed_access_inside_generic_callback_aliases() {
    let functions = parse_dts(
        "type TupleIterator<T extends readonly unknown[], Result> = (value: T[number], index: number) => Result;\nexport declare function map<T extends readonly unknown[], Result>(values: T, iterator: TupleIterator<T, Result>): Result[];",
    )
    .unwrap();
    assert_eq!(
        functions[0]
            .generic
            .as_ref()
            .unwrap()
            .contextual_param_types,
        vec!["T", "(value: T[number], index: number) => Result"]
    );
}

#[test]
fn expands_nested_generic_callback_aliases_inside_unions() {
    let functions = parse_dts(
        "type Iterator<T, Result> = (value: T, index: number) => Result;\ntype Iteratee<T> = Iterator<T, boolean> | string;\nexport declare function every<T>(values: T[], iteratee: Iteratee<T>): boolean;",
    )
    .unwrap();
    assert_eq!(
        functions[0]
            .generic
            .as_ref()
            .unwrap()
            .contextual_param_types,
        vec!["T[]", "(value: T, index: number) => boolean | string"]
    );
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
