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
    assert!(runs.iter().all(|method| method.return_instance_class.is_none()));
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
