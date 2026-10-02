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
    // A fluent `run(...): this` return now records the enclosing class
    // as its instance-return type (chainable `JsValue` handle), the same
    // as an explicit `static verbose(): Database` does.
    assert!(runs
        .iter()
        .all(|method| method.return_instance_class.as_deref() == Some("Database")));
    assert_eq!(
        database
            .methods
            .iter()
            .find(|method| method.name == "verbose")
            .and_then(|method| method.return_instance_class.as_deref()),
        Some("Database")
    );
    assert!(runs.iter().all(|method| method.overloaded));
    assert!(runs
        .iter()
        .all(|method| method.ret == DtsType::Native(HirType::Void)));
    let close = database
        .methods
        .iter()
        .find(|method| method.name == "close")
        .unwrap();
    assert_eq!(close.required_params, 0);
    assert_eq!(
        close.params[0].1,
        DtsType::Native(HirType::Function(
            vec![HirType::JsValue],
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
fn extracts_constructor_value_aliases_as_classes() {
    let classes = parse_dts_classes(
        r#"declare class InternalServer {
                constructor(options?: { port?: number });
                close(callback?: () => void): void;
            }
            export const PublicServer: typeof InternalServer;
            export interface PublicServer extends InternalServer {}"#,
    )
    .unwrap();
    let public = classes
        .iter()
        .find(|class| class.name == "PublicServer")
        .unwrap();
    assert_eq!(public.constructors.len(), 1);
    assert!(public.methods.iter().any(|method| method.name == "close"));
}

#[test]
fn retains_defaulted_instance_types_delivered_to_method_callbacks() {
    let classes = parse_dts_classes(
        r#"declare class Socket { send(value: string): void; }
            declare class Server<T extends typeof Socket = typeof Socket> {
                on(event: "connection", callback: (socket: InstanceType<T>) => void): this;
            }
            export const PublicServer: typeof Server;"#,
    )
    .unwrap();
    let method = classes
        .iter()
        .find(|class| class.name == "PublicServer")
        .unwrap()
        .methods
        .iter()
        .find(|method| method.name == "on")
        .unwrap();
    assert_eq!(
        method.callback_instance_classes[1],
        vec![Some("Socket".into())]
    );
}

#[test]
fn keeps_opaque_dynamic_callback_values_as_live_handles() {
    let classes = parse_dts_classes(
        r#"type RawData = Buffer | ArrayBuffer | Buffer[];
            declare class Socket {
                on(event: "message", callback: (data: RawData) => void): this;
            }"#,
    )
    .unwrap();
    assert!(
        matches!(
            &classes[0].methods[0].params[1].1,
            DtsType::Native(HirType::Function(params, _)) if params == &[HirType::JsValue]
        ),
        "{:?}",
        classes[0].methods[0].params[1].1
    );
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
fn records_optional_constructor_arities() {
    let classes = parse_dts_classes(
        r#"export class Client {
                constructor(url: string, timeout?: number, retries?: number);
            }
            export class Cache {
                constructor(size: number, enabled?: boolean);
            }"#,
    )
    .unwrap();
    assert_eq!(classes[0].constructors[0].required_params, 1);
    assert_eq!(classes[0].constructors[0].params.len(), 3);
    assert_eq!(classes[1].constructors[0].required_params, 1);
    assert_eq!(classes[1].constructors[0].params.len(), 2);
}

#[test]
fn classifies_this_bound_rest_callbacks_for_contextual_inference() {
    let classes = parse_dts_classes(
        "export class Command {\n\
             action(fn: (this: this, ...args: any[]) => void | Promise<void>): this;\n\
         }",
    )
    .unwrap();
    let callback = &classes[0].methods[0].params[0].1;
    assert!(
        matches!(
            callback,
            DtsType::Native(HirType::CallableFunction(params, _, Some(rest), ret))
                if params.is_empty() && **rest == HirType::Json && **ret == HirType::Void
        ),
        "{callback:?}"
    );
}

#[test]
fn resolves_class_method_type_parameters_inside_generic_interfaces() {
    let classes = parse_dts_classes(
        r#"export interface Box<T> { value: T; }
           export class Service {
             register<T extends string>(options: Box<T>): void;
           }"#,
    )
    .unwrap();
    assert_eq!(
        classes[0].methods[0].params[0].1,
        DtsType::Native(HirType::Object(vec![("value".into(), HirType::Str)]))
    );
}

#[test]
fn expands_inherited_external_class_members() {
    let classes = parse_dts_classes(
        r#"export class Base {
                constructor(value: number);
                base(value: number): number;
                inherited(): string;
                request: (path: string) => Promise<Response>;
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
        .any(|method| method.name == "request" && method.required_params == 1));
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

/// A namespace-qualified `extends` clause (`class Parser extends stream.
/// Transform { ... }`, real example: csv-parse's own `Parser`, `import *
/// as stream from "stream"`) used to inherit nothing at all -- `extends`
/// was only ever derived from a bare `Expr::Ident` super-class
/// expression, never a qualified `Expr::Member` one, so `Transform`'s
/// own members (declared elsewhere in the same flattened source, same
/// as any other same-file extends) were silently unreachable regardless
/// of whether `Transform` itself was ever found. This is the thaw-bridge
/// half of the fix; the other half (thaw-registry inlining a Node
/// builtin's own class declaration into a third-party package's
/// flattened `.d.ts` when its `extends` clause references one) is
/// exercised end to end by `thaw-cli`'s `registry_fallback` tests and
/// the real csv-parse pinned integration test.
#[test]
fn expands_members_inherited_through_a_namespace_qualified_extends() {
    let classes = parse_dts_classes(
        r#"export class Transform {
                read(size?: number): any;
                write(chunk: any): boolean;
            }
            export class Parser extends stream.Transform {
                constructor(options: any);
                parse(): void;
            }"#,
    )
    .unwrap();
    let parser = classes.iter().find(|class| class.name == "Parser").unwrap();
    assert!(parser.methods.iter().any(|method| method.name == "read"));
    assert!(parser.methods.iter().any(|method| method.name == "write"));
    assert!(parser.methods.iter().any(|method| method.name == "parse"));
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

#[test]
fn extracts_non_callable_values_without_duplicating_callable_consts() {
    let values = parse_dts_values(
        r#"export interface Factory { (): string; }
            export declare const factory: Factory;
            export declare const NIL: string;
            export declare const count: number;
            export declare class Mime { getType(path: string): string | null; }
            declare const mime: Mime;"#,
    )
    .unwrap();
    assert_eq!(
        values,
        vec![
            DtsValue {
                name: "NIL".into(),
                ty: DtsType::Native(HirType::Str)
            },
            DtsValue {
                name: "count".into(),
                ty: DtsType::Native(HirType::F64)
            },
            DtsValue {
                name: "mime".into(),
                ty: DtsType::Native(HirType::JsValue)
            },
        ]
    );
}

/// mathjs publishes its API through an annotated object destructuring
/// declaration instead of one declaration per named export.
#[test]
fn extracts_callable_exports_from_an_annotated_object_pattern() {
    let functions = parse_dts(
        r#"interface Add { (left: number, right: number): number; }
            interface MathJsInstance {
                add: Add;
                sqrt(value: number): number;
                label: string;
            }
            export declare const { add: sum, sqrt, label }: MathJsInstance;"#,
    )
    .unwrap();
    assert_eq!(
        functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>(),
        ["sum", "sqrt"]
    );
    assert!(functions
        .iter()
        .all(|function| matches!(classify(function), Classification::FastPath(_))));
    assert_eq!(
        parse_dts_values(
            r#"interface MathJsInstance {
                add(left: number, right: number): number;
                label: string;
            }
            export declare const { add, label: title }: MathJsInstance;"#,
        )
        .unwrap(),
        [DtsValue {
            name: "title".into(),
            ty: DtsType::Native(HirType::Str),
        }]
    );
}

/// `.d.ts` flattening (thaw-registry) concatenates every file a
/// package's type declarations span into one source string; the same
/// value name declared in more than one of those files (a genuine
/// re-export, or the same ambient declaration duplicated across two
/// concatenated files) used to produce two `DtsValue` entries for the
/// same binding, which the shim generator's own duplicate-binding
/// check (rightly) rejected as a hard build error -- real examples:
/// `marked`'s own `_defaults`, `js-yaml`'s own `binaryTag`. Same class
/// of bug already fixed for `parse_dts_classes` (yaml's `NodeBase`,
/// socket.io's `StrictEventEmitter`), just never applied to values
/// too. First occurrence wins.
#[test]
fn deduplicates_a_value_name_declared_more_than_once() {
    let values = parse_dts_values(
        r#"export declare const count: number;
            export declare const count: number;"#,
    )
    .unwrap();
    assert_eq!(
        values,
        vec![DtsValue {
            name: "count".into(),
            ty: DtsType::Native(HirType::F64)
        }]
    );
}

#[test]
fn constructor_rest_parameter_keeps_element_type() {
    let classes = parse_dts_classes("declare class C { constructor(...parts: string[]); }").unwrap();
    assert!(classes[0].constructors[0].params.is_empty());
    assert_eq!(
        classes[0].constructors[0].rest_param,
        Some(("parts".to_string(), DtsType::Native(HirType::Str)))
    );
}

#[test]
fn merged_constructible_interface_keeps_each_constructor() {
    let classes = parse_dts_classes(
        "interface Maker { new(x: string): Item } interface Maker { new(x: number): Item } declare const Factory: Maker;",
    )
    .unwrap();
    let factory = classes.iter().find(|class| class.name == "Factory").unwrap();
    assert_eq!(factory.constructors.len(), 2);
    assert!(factory.constructors.iter().all(|constructor| constructor.overloaded));
}

#[test]
fn merged_namespace_constructor_interface_keeps_each_signature() {
    let classes = parse_dts_classes(
        "interface Maker { new(x: string): Item } interface Maker { new(x: number): Item } declare namespace makers { export { Maker as Public }; }",
    )
    .unwrap();
    let maker = classes.iter().find(|class| class.name == "Maker").unwrap();
    assert_eq!(maker.constructors.len(), 2);
}

#[test]
fn namespace_constructible_interfaces_with_same_bare_name_stay_distinct() {
    let classes = parse_dts_classes(
        "declare namespace A { interface Client { new(x: string): Client } export { Client as Public }; } \
         declare namespace B { interface Client { new(x: number): Client } export { Client as Public }; }",
    ).unwrap();
    let a = classes.iter().find(|class| class.name == "A.Client").unwrap();
    let b = classes.iter().find(|class| class.name == "B.Client").unwrap();
    assert_eq!(a.constructors[0].params[0].1, DtsType::Native(HirType::Str));
    assert_eq!(b.constructors[0].params[0].1, DtsType::Native(HirType::F64));
}
#[test]
fn class_return_identity_excludes_scalar_aliases_and_ambiguous_namespace_classes() {
    let classes = parse_dts_classes(r#"
        type Label = string;
        declare namespace A { class Label {} }
        declare namespace A { class Client {} }
        declare namespace B { class Client {} }
        declare namespace A { class Solo {} }
        declare class Provider {
            label(): Label;
            ambiguous(): A.Client;
            unique(): A.Solo;
            self(): this;
        }
    "#).unwrap();
    let provider = classes.iter().find(|class| class.name == "Provider").unwrap();
    let label = provider.methods.iter().find(|method| method.name == "label").unwrap();
    assert_eq!(label.ret, DtsType::Native(HirType::Str));
    assert_eq!(label.return_instance_class, None);
    let ambiguous = provider.methods.iter().find(|method| method.name == "ambiguous").unwrap();
    assert_eq!(ambiguous.return_instance_class.as_deref(), Some("A.Client"));
    let unique = provider.methods.iter().find(|method| method.name == "unique").unwrap();
    assert_eq!(unique.return_instance_class.as_deref(), Some("A.Solo"));
    let this = provider.methods.iter().find(|method| method.name == "self").unwrap();
    assert_eq!(this.return_instance_class.as_deref(), Some("Provider"));
}

#[test]
fn duplicate_namespace_classes_keep_independent_identity_and_inheritance() {
    let classes = parse_dts_classes(r#"
        declare namespace A {
            class Base { fromA(): string; }
            class Client extends Base {
                constructor(value: number);
                chain(): this;
                other(): B.Client;
            }
        }
        declare namespace B {
            class Base { fromB(): number; }
            class Client extends Base {
                constructor(value: string);
                chain(): this;
                other(): A.Client;
            }
        }
    "#).unwrap();
    let a = classes.iter().find(|class| class.name == "A.Client").unwrap();
    let b = classes.iter().find(|class| class.name == "B.Client").unwrap();
    assert_eq!(a.extends.as_deref(), Some("A.Base"));
    assert_eq!(b.extends.as_deref(), Some("B.Base"));
    assert!(a.methods.iter().any(|method| method.name == "fromA"));
    assert!(!a.methods.iter().any(|method| method.name == "fromB"));
    assert!(b.methods.iter().any(|method| method.name == "fromB"));
    assert!(!b.methods.iter().any(|method| method.name == "fromA"));
    assert_eq!(a.constructors[0].params[0].1, DtsType::Native(HirType::F64));
    assert_eq!(b.constructors[0].params[0].1, DtsType::Native(HirType::Str));
    assert_eq!(a.methods.iter().find(|method| method.name == "chain").unwrap().return_instance_class.as_deref(), Some("A.Client"));
    assert_eq!(b.methods.iter().find(|method| method.name == "chain").unwrap().return_instance_class.as_deref(), Some("B.Client"));
    assert_eq!(a.methods.iter().find(|method| method.name == "other").unwrap().return_instance_class.as_deref(), Some("B.Client"));
    assert_eq!(b.methods.iter().find(|method| method.name == "other").unwrap().return_instance_class.as_deref(), Some("A.Client"));
}

#[test]
fn imported_base_alias_preserves_inheritance_and_self_return() {
    // Unrun regression for a flattened `import { Base as Parent }`.
    let classes = parse_dts_classes(r#"
        declare class Parent { base(): string; self(): Parent; }
        declare namespace Parent { type Key = string; }
        export type { Parent };
        export declare class Derived extends Parent { own(): boolean; }
    "#).unwrap();
    let derived = classes.iter().find(|class| class.name == "Derived").unwrap();
    assert_eq!(derived.extends.as_deref(), Some("Parent"));
    assert!(derived.methods.iter().any(|method| method.name == "base"));
    assert_eq!(derived.methods.iter().find(|method| method.name == "self")
        .unwrap().return_instance_class.as_deref(), Some("Parent"));
}

#[test]
fn type_only_class_alias_keeps_instance_shape_and_separate_value_alias() {
    // Unrun regression: the public type alias and live constructor alias
    // can name the same declaration without turning the type alias into a
    // runtime class value.
    let source = r#"
        declare class Client {
            constructor(name: string);
            getName(): string;
            chain(): this;
            static create(): Client;
        }
        export type { Client as TypeClient };
        export { Client as LiveClient };
    "#;
    let classes = parse_dts_classes(source).unwrap();
    for name in ["TypeClient", "LiveClient"] {
        let class = classes.iter().find(|class| class.name == name).unwrap();
        assert!(class.methods.iter().any(|method| method.name == "getName" && !method.is_static));
        assert_eq!(class.methods.iter().find(|method| method.name == "chain").unwrap()
            .return_instance_class.as_deref(), Some(name));
    }
    let type_names = exported_type_names(source);
    let value_names = exported_value_names(source);
    assert!(type_names.contains("TypeClient"));
    assert!(!value_names.contains("TypeClient"));
    assert!(value_names.contains("LiveClient"));
}

#[test]
fn namespace_class_alias_keeps_constructor_and_self_return_identity() {
    // Unrun regression for a named class re-export moved into a value
    // namespace. Both class shape and runtime member key stay qualified.
    let source = r#"
        declare namespace api {
            class Hidden {
                constructor(name: string);
                self(): Hidden;
            }
            export type { Hidden };
            export { Hidden as Client };
        }
    "#;
    let classes = parse_dts_classes(source).unwrap();
    let client = classes.iter().find(|class| class.name == "api.Client").unwrap();
    assert!(client.constructible);
    assert_eq!(client.methods.iter().find(|method| method.name == "self").unwrap()
        .return_instance_class.as_deref(), Some("api.Client"));
    let members = nested_namespace_members(source);
    assert_eq!(members["api"]["Client"], "Hidden");
    let type_names = exported_type_names(source);
    let value_names = exported_value_names(source);
    assert!(type_names.contains("api.Hidden"));
    assert!(!value_names.contains("api.Hidden"));
    assert!(value_names.contains("api.Client"));
    assert!(!exported_value_names(
        "declare namespace api { interface Secret {} export { Secret as PublicShape }; }"
    ).contains("api.PublicShape"));
}

#[test]
fn reserved_class_and_value_exports_keep_public_runtime_names() {
    // Unrun regression for the registry's valid synthetic identifiers.
    let source = r#"
        export declare class __thaw_public_6e756c6c_deadbeef {
            constructor(value: number);
            label(): string;
        }
        export { __thaw_public_6e756c6c_deadbeef as null };
        export declare const __thaw_public_766f6964_deadbeef: number;
        export { __thaw_public_766f6964_deadbeef as void };
    "#;
    let classes = parse_dts_classes(source).unwrap();
    assert!(classes.iter().any(|class| class.name == "null"
        && class.methods.iter().any(|method| method.name == "label")));
    assert!(!classes.iter().any(|class| class.name.starts_with("__thaw_public_")));
    let values = parse_dts_values(source).unwrap();
    assert!(values.iter().any(|value| value.name == "void"));
    assert!(!values.iter().any(|value| value.name.starts_with("__thaw_public_")));
}

#[test]
fn interface_alias_without_type_modifier_is_not_a_value_export() {
    let source = "interface Shape { size: number; } export { Shape as PublicShape };";
    assert!(!exported_value_names(source).contains("PublicShape"));
}

#[test]
fn namespace_class_method_uses_local_interface_layout() {
    let classes = parse_dts_classes(r#"
        declare namespace A {
            interface Options { x: number; }
            class Provider { use(value: Options): void; get(): Options; }
        }
        declare namespace B { interface Options { y: string; } class Options {} }
    "#).unwrap();
    let provider = classes.iter().find(|class| class.name == "A.Provider").unwrap();
    let method = provider.methods.iter().find(|method| method.name == "use").unwrap();
    assert_eq!(method.params[0].1, DtsType::Native(HirType::Object(vec![("x".into(), HirType::F64)])));
    let getter = provider.methods.iter().find(|method| method.name == "get").unwrap();
    assert_eq!(getter.return_instance_class, None);
}

#[test]
fn namespace_construct_signature_uses_local_parameter_layout() {
    let classes = parse_dts_classes(r#"
        interface Options { wrong: boolean; }
        declare namespace API {
            interface Options { x: number; }
            interface Factory { new(value: Options): Client; }
        }
        declare const Client: API.Factory;
    "#).unwrap();
    let client = classes.iter().find(|class| class.name == "Client").unwrap();
    assert_eq!(client.constructors[0].params[0].1,
        DtsType::Native(HirType::Object(vec![("x".into(), HirType::F64)])));
}

#[test]
fn namespace_alias_construct_signature_uses_target_scope() {
    let classes = parse_dts_classes(r#"
        interface Options { wrong: boolean; }
        declare namespace API {
            interface Options { x: number; }
            interface Maker { new(value: Options): Client; }
            export { Maker as Public };
        }
    "#).unwrap();
    let maker = classes.iter().find(|class| class.name == "Maker").unwrap();
    assert_eq!(maker.constructors[0].params[0].1,
        DtsType::Native(HirType::Object(vec![("x".into(), HirType::F64)])));
}

#[test]
fn named_declaration_helpers_keep_one_source_map_identity_through_reparses() {
    use thaw_parser::common::{FileName, Spanned};

    let source = "declare namespace N { export function make(): number; } export { N as Public };";
    let filename = FileName::Real(std::path::PathBuf::from("/fixture/package.d.ts"));
    let (module, map) = thaw_parser::parse_declarations_with_source_map_named(source, filename.clone()).unwrap();
    assert_eq!(map.lookup_char_pos(module.body[0].span().lo).file.name.as_ref(), &filename);
    assert_eq!(parse_dts_named(source, filename.clone()).unwrap(), parse_dts(source).unwrap());
    assert_eq!(parse_dts_classes_named(source, &filename).unwrap(), parse_dts_classes(source).unwrap());
    assert_eq!(parse_dts_values_named(source, &filename).unwrap(), parse_dts_values(source).unwrap());
    assert_eq!(nested_namespace_members_named(source, &filename), nested_namespace_members(source));
    assert_eq!(exported_value_names_named(source, &filename), exported_value_names(source));
    assert_eq!(nonpublic_namespace_sources_named(source, &filename), nonpublic_namespace_sources(source));

    let malformed = "declare function ;";
    assert!(parse_dts_named(malformed, filename.clone()).is_err());
    assert!(parse_dts_values_named(malformed, &filename).is_err());
    assert!(type_only_namespace_names_named(malformed, &filename).is_empty());
    assert!(nested_namespace_members_named(malformed, &filename).is_empty());
}
