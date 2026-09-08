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

#[test]
fn resolves_namespace_scoped_types_in_ambient_functions() {
    let funcs = parse_dts(
        r#"declare namespace ParcelWatcher {
            export type BackendType = "fs-events" | "watchman" | "inotify" | "windows";
            export interface Options { backend?: BackendType; }
            export type SubscribeCallback = (err: Error | null, events: Event[]) => unknown;
            export interface Event { type: "create" | "update" | "delete"; path: string; }
            export function subscribe(dir: string, fn: SubscribeCallback, opts?: Options): Promise<void>;
        }
        export = ParcelWatcher;"#,
    )
    .unwrap();
    let subscribe = funcs
        .iter()
        .find(|function| function.name == "subscribe")
        .unwrap();
    assert_eq!(subscribe.params[0].1, DtsType::Native(HirType::Str));
    assert!(matches!(subscribe.params[1].1, DtsType::Unsupported(_)));
    assert_eq!(
        subscribe.params[2].1,
        DtsType::Native(HirType::Object(vec![(
            "backend".into(),
            HirType::Optional(Box::new(HirType::Str)),
        )]))
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

/// A `declare const NAME: T;` whose type `T` is a *local, unexported*
/// type alias (not a direct function type, not an interface) -- real
/// example: uuid's own `.d.ts` (via `@types/uuid`), `export const
/// validate: validate;` where `type validate = (uuid: string) =>
/// boolean;` is declared bare, elsewhere in the same file.
#[test]
fn extracts_a_callable_const_typed_through_a_local_type_alias() {
    let source = r#"
            type validate = (uuid: string) => boolean;
            export const validate: validate;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 1, "{funcs:?}");
    assert_eq!(funcs[0].name, "validate");
    assert_eq!(funcs[0].params.len(), 1);
    assert_eq!(funcs[0].required_params, 1);
}

/// The same shape, but the const's type alias resolves to an
/// *intersection* of two further local aliases, each themselves a direct
/// function type -- real example: uuid's `v1`/`v3`/`v4`/`v5`/`v6`/`v7`,
/// each declared `type vN = vNBuffer & vNString;` where `vNBuffer`/
/// `vNString` are themselves separate local aliases. Confirms both
/// overloads survive as separate `DtsFunction`s, the same convention
/// already used for an interface's own multiple call signatures.
#[test]
fn extracts_a_callable_const_typed_through_an_intersection_of_local_aliases() {
    let source = r#"
            type v4String = (options?: string) => string;
            type v4Buffer = <T>(options: string | null | undefined, buffer: T) => T;
            type v4 = v4Buffer & v4String;
            export const v4: v4;
        "#;
    let funcs = parse_dts(source).unwrap();
    assert_eq!(funcs.len(), 2, "{funcs:?}");
    assert!(funcs.iter().all(|f| f.name == "v4"), "{funcs:?}");
    assert!(
        funcs
            .iter()
            .any(|f| f.params.len() == 1 && f.required_params == 0),
        "{funcs:?}"
    );
    assert!(
        funcs
            .iter()
            .any(|f| f.params.len() == 2 && f.required_params == 2),
        "{funcs:?}"
    );
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

/// `export default X;` -- a *separate* AST node from `export { X as
/// default }`, and the shape a real npm package commonly uses instead.
/// Real example: zod v3's `lib/index.d.ts`, `import * as z from
/// "./external"; export { z }; export default z;` -- confirms
/// `self_referential_namespace_aliases` recognizes `default` here too,
/// not just the `export { X as default }` form.
#[test]
fn self_referential_namespace_aliases_recognizes_a_separate_export_default_statement() {
    let source = r#"
            import * as z from "./external";
            export * from "./external";
            export { z };
            export default z;
        "#;
    let aliases = self_referential_namespace_aliases(source);
    assert!(aliases.contains("z"), "{aliases:?}");
    assert!(aliases.contains("default"), "{aliases:?}");
}

/// A plain `export default someValue;` where `someValue` is *not* bound
/// by a namespace import at all must not be mistaken for a self-
/// referential alias -- confirms the new `export default` handling is
/// conditioned on the identifier actually being a namespace import, the
/// same restriction the existing `export { X as default }` form already
/// has.
#[test]
fn export_default_of_an_unrelated_identifier_is_not_a_self_referential_alias() {
    let source = r#"
            declare function helper(): number;
            export default helper;
        "#;
    let aliases = self_referential_namespace_aliases(source);
    assert!(aliases.is_empty(), "{aliases:?}");
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

