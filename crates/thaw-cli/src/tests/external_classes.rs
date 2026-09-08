#[test]
fn rewrite_qualified_calls_is_a_no_op_with_no_rewrites() {
    let source = "function main(): void { console.log(qs.stringify(x)); }";
    assert_eq!(
        rewrite_qualified_calls(source, &[], &Default::default()).unwrap(),
        source
    );
}

#[test]
fn rewrite_qualified_calls_replaces_matching_qualified_calls_only() {
    let source = "function main(): void {\n\
             console.log(qs.stringify(x));\n\
             console.log(hoek.stringify(y));\n\
             console.log(qs.parse(z));\n\
             console.log(unrelated.stringify(w));\n\
         }";
    let rewrites = vec![
        (
            "qs".to_string(),
            "stringify".to_string(),
            "qs_stringify".to_string(),
        ),
        (
            "hoek".to_string(),
            "stringify".to_string(),
            "hoek_stringify".to_string(),
        ),
    ];
    let rewritten = rewrite_qualified_calls(source, &rewrites, &Default::default()).unwrap();

    assert!(rewritten.contains("console.log(qs_stringify(x));"));
    assert!(rewritten.contains("console.log(hoek_stringify(y));"));
    // Not in `rewrites` (no collision for `parse`, or the object
    // isn't a known qualifier at all) -- left completely alone.
    assert!(rewritten.contains("console.log(qs.parse(z));"));
    assert!(rewritten.contains("console.log(unrelated.stringify(w));"));
}

#[test]
fn rewrite_qualified_calls_handles_a_call_nested_in_an_expression() {
    let source = "function main(): void { const r = String(qs.stringify(x)); }";
    let rewrites = vec![(
        "qs".to_string(),
        "stringify".to_string(),
        "qs_stringify".to_string(),
    )];
    let rewritten = rewrite_qualified_calls(source, &rewrites, &Default::default()).unwrap();
    assert!(rewritten.contains("const r = String(qs_stringify(x));"));
}

/// Namespace imports resolve through the package-qualified typed alias before
/// module bundling, allowing the overload pass to inspect the real call shape.
#[test]
fn rewrite_qualified_calls_resolves_a_real_namespace_import() {
    let source =
        "import * as qs from \"qs\";\nfunction main(): void { console.log(qs.stringify(x)); }";
    let rewrites = vec![(
        "qs".to_string(),
        "stringify".to_string(),
        "qs_stringify".to_string(),
    )];
    let overloads = std::collections::HashSet::from(["qs_stringify".to_string()]);
    let rewritten = rewrite_qualified_calls(source, &rewrites, &overloads).unwrap();
    assert!(rewritten.contains("console.log(qs_stringify(x));"));
}

/// Default imports expose the package namespace too; a renamed named import is
/// still an ordinary value and must not be treated as a namespace.
#[test]
fn rewrite_qualified_calls_distinguishes_default_and_named_imports() {
    let rewrites = vec![(
        "qs".to_string(),
        "stringify".to_string(),
        "qs_stringify".to_string(),
    )];
    let default_import =
        "import qs from \"qs\";\nfunction main(): void { console.log(qs.stringify(x)); }";
    let overloads = std::collections::HashSet::from(["qs_stringify".to_string()]);
    assert!(
        rewrite_qualified_calls(default_import, &rewrites, &overloads)
            .unwrap()
            .contains("console.log(qs_stringify(x));")
    );

    let renamed_named_import = "import { stringify as qs } from \"qs\";\nfunction main(): void { console.log(qs.stringify(x)); }";
    assert!(
        rewrite_qualified_calls(renamed_named_import, &rewrites, &overloads)
            .unwrap()
            .contains("console.log(qs.stringify(x));")
    );
}

#[test]
fn rewrites_external_class_constructors_without_touching_other_new_expressions() {
    let source = "const a = new Database(\":memory:\"); const b = new sqlite3.Database(\"db.sqlite\"); const c = new LocalBox(1);";
    let rewritten = rewrite_external_class_constructors(
        source,
        &[(
            "sqlite3".into(),
            "Database".into(),
            vec![(1, "Database_ctor".into(), vec![])],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const a = Database_ctor(\":memory:\"); const b = Database_ctor(\"db.sqlite\"); const c = new LocalBox(1);"
        );
}

#[test]
fn rewrites_methods_on_instances_received_by_callbacks() {
    let source =
        "const server = new Server(); server.on(\"connection\", (socket) => socket.send(\"hi\"));";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[(
            "pkg".into(),
            "Server".into(),
            vec![(0, "Server_ctor".into(), vec![])],
        )],
        &[
            (
                "Server".into(),
                "on".into(),
                "Server_on_error".into(),
                2,
                true,
                vec![
                    thaw_hir::HirType::Str,
                    thaw_hir::HirType::Function(
                        vec![thaw_hir::HirType::Json],
                        Box::new(thaw_hir::HirType::Void),
                    ),
                ],
            ),
            (
                "Server".into(),
                "on".into(),
                "Server_on".into(),
                2,
                true,
                vec![
                    thaw_hir::HirType::Str,
                    thaw_hir::HirType::Function(
                        vec![thaw_hir::HirType::Json],
                        Box::new(thaw_hir::HirType::Void),
                    ),
                ],
            ),
            (
                "Socket".into(),
                "send".into(),
                "Socket_send".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
        &[
            ClassMethodContext::LiteralArgument("Server_on_error".into(), 0, "error".into()),
            ClassMethodContext::LiteralArgument("Server_on".into(), 0, "connection".into()),
            ClassMethodContext::CallbackInstance("Server_on".into(), 1, 0, "Socket".into()),
        ],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const server = Server_ctor(); Server_on(server, \"connection\", (socket) => Socket_send(socket, \"hi\"));"
    );
}

#[test]
fn rewrites_external_class_constructors_with_erased_type_arguments() {
    let source = "const app = new Hono<{ Bindings: Bindings }>();";
    let rewritten = rewrite_external_class_constructors(
        source,
        &[(
            "hono".into(),
            "Hono".into(),
            vec![(0, "Hono_ctor".into(), vec![])],
        )],
    )
    .unwrap();
    assert_eq!(rewritten, "const app = Hono_ctor();");
}

#[test]
fn rewrites_external_class_constructors_by_argument_count() {
    let source = "const a = new Database(); const b = new Database('db'); const c = new Database('db', 6); const d = new Database('db', 6, true);";
    let rewritten = rewrite_external_class_constructors(
        source,
        &[(
            "sqlite3".into(),
            "Database".into(),
            vec![
                (0, "Database_ctor0".into(), vec![]),
                (1, "Database_ctor1".into(), vec![]),
                (2, "Database_ctor2".into(), vec![]),
            ],
        )],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const a = Database_ctor0(); const b = Database_ctor1('db'); const c = Database_ctor2('db', 6); const d = new Database('db', 6, true);"
    );
}

#[test]
fn does_not_guess_between_same_arity_constructor_helpers() {
    let source = "const value = new NativeBox(true);";
    let rewritten = rewrite_external_class_constructors(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![
                (1, "NativeBox_string".into(), vec![thaw_hir::HirType::Str]),
                (1, "NativeBox_number".into(), vec![thaw_hir::HirType::F64]),
            ],
        )],
    )
    .unwrap();
    assert_eq!(rewritten, source);
}

#[test]
fn generates_napi_constructor_helpers_for_each_supported_arity() {
    let class = thaw_bridge::DtsClass {
        name: "Client".into(),
        extends: None,
        constructible: true,
        constructors: vec![thaw_bridge::DtsConstructor {
            params: vec![
                (
                    "url".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
                ),
                (
                    "timeout".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                ),
            ],
            required_params: 0,
            overloaded: false,
        }],
        methods: vec![],
        properties: vec![],
    };
    let mut shim = String::new();
    let helpers = generate_napi_class_constructors(&class, true, &mut shim);
    assert_eq!(
        helpers
            .iter()
            .map(|(arity, _, _)| *arity)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(shim.contains("(): JsValue;"));
    assert!(shim.contains("(url: string): JsValue;"));
    assert!(shim.contains("(url: string, timeout: number): JsValue;"));

    let rewritten = rewrite_external_class_constructors(
        "const a = new Client(); const b = new Client('x'); const c = new Client('x', 5);",
        &[("pkg".into(), "Client".into(), helpers.clone())],
    )
    .unwrap();
    for (_, helper, _) in helpers {
        assert!(rewritten.contains(&helper));
    }

    let mut default_shim = String::new();
    let default_helpers = generate_napi_class_constructors(
        &thaw_bridge::DtsClass {
            name: "DefaultBox".into(),
            extends: None,
            constructible: true,
            constructors: vec![],
            methods: vec![],
            properties: vec![],
        },
        true,
        &mut default_shim,
    );
    assert_eq!(default_helpers.len(), 1);
    assert_eq!(default_helpers[0].0, 0);
    assert!(default_shim.contains("(): JsValue;"));

    let mut locked_shim = String::new();
    let mut locked = class;
    locked.constructible = false;
    assert!(generate_napi_class_constructors(&locked, true, &mut locked_shim).is_empty());
    assert!(locked_shim.is_empty());
}

#[test]
fn generates_dynamic_constructor_helpers_for_unclassified_options() {
    let class = thaw_bridge::DtsClass {
        name: "Server".into(),
        extends: None,
        constructible: true,
        constructors: vec![thaw_bridge::DtsConstructor {
            params: vec![(
                "options".into(),
                thaw_bridge::DtsType::Unsupported("generic options".into()),
            )],
            required_params: 0,
            overloaded: false,
        }],
        methods: vec![],
        properties: vec![],
    };
    let mut shim = String::new();
    let helpers = generate_napi_class_constructors(&class, false, &mut shim);
    assert_eq!(
        helpers.iter().map(|helper| helper.0).collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(shim.contains("(options: Json): JsValue;"));
}

#[test]
fn selects_same_arity_napi_constructors_by_argument_type() {
    let class = thaw_bridge::DtsClass {
        name: "NativeBox".into(),
        extends: None,
        constructible: true,
        constructors: vec![
            thaw_bridge::DtsConstructor {
                params: vec![(
                    "value".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
                )],
                required_params: 1,
                overloaded: true,
            },
            thaw_bridge::DtsConstructor {
                params: vec![(
                    "value".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                )],
                required_params: 1,
                overloaded: true,
            },
        ],
        methods: vec![],
        properties: vec![],
    };
    let mut shim = String::new();
    let helpers = generate_napi_class_constructors(&class, true, &mut shim);
    assert_eq!(helpers.len(), 2);
    let string_helper = helpers
        .iter()
        .find(|(_, _, types)| types == &[thaw_hir::HirType::Str])
        .unwrap()
        .1
        .clone();
    let number_helper = helpers
        .iter()
        .find(|(_, _, types)| types == &[thaw_hir::HirType::F64])
        .unwrap()
        .1
        .clone();
    let rewritten = rewrite_external_class_methods(
        "const n = 42; const s = 'value'; const a = new NativeBox(n); const b = new NativeBox(s); const c = new NativeBox(7); const d = new NativeBox('text');",
        &[("pkg".into(), "NativeBox".into(), helpers)],
        &[],
    )
    .unwrap();
    assert!(rewritten.contains(&format!("const a = {number_helper}(n)")));
    assert!(rewritten.contains(&format!("const b = {string_helper}(s)")));
    assert!(rewritten.contains(&format!("const c = {number_helper}(7)")));
    assert!(rewritten.contains(&format!("const d = {string_helper}('text')")));
}

#[test]
fn generates_typed_napi_tuple_class_shims() {
    let class = thaw_bridge::parse_dts_classes(
        r#"export class PairBox {
                constructor(value: [number, string]);
                swap(value: [number, string]): [string, number];
                pair: [number, string];
            }"#,
    )
    .unwrap()
    .remove(0);
    let tuple = thaw_hir::HirType::Tuple(vec![thaw_hir::HirType::F64, thaw_hir::HirType::Str]);
    let mut shim = String::new();
    let constructors = generate_napi_class_constructors(&class, true, &mut shim);
    assert_eq!(constructors[0].2, vec![tuple.clone()]);
    let methods = generate_napi_class_method_overloads(
        &class,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
        true,
    );
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].4, vec![tuple.clone()]);
    let property = &class.properties[0];
    assert!(generate_napi_class_property_getter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(generate_napi_class_property_setter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(shim.contains("value: [number, string]"));
    assert!(shim.contains("): [string, number]"));

    let rewritten = rewrite_external_class_methods(
        "const value = unknown as [number, string]; const box = new PairBox(value); box.swap(value);",
        &[("pkg".into(), "PairBox".into(), constructors)],
        &[(
            "PairBox".into(),
            "swap".into(),
            methods[0].1.clone(),
            1,
            false,
            vec![tuple],
        )],
    )
    .unwrap();
    assert!(!rewritten.contains("new PairBox"));
    assert!(rewritten.contains(&methods[0].1));
}

#[test]
fn generates_typed_napi_recursive_array_shims() {
    let class = thaw_bridge::parse_dts_classes(
        r#"export class ArrayBox {
                constructor(labels: string[]);
                flags(values: boolean[]): string[];
                group(values: string[][]): string[][];
                records: { name: string }[];
            }"#,
    )
    .unwrap()
    .remove(0);
    let strings = thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Str));
    let booleans = thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Bool));
    let mut shim = String::new();
    let constructors = generate_napi_class_constructors(&class, true, &mut shim);
    assert_eq!(constructors[0].2, vec![strings.clone()]);
    let methods = generate_napi_class_method_overloads(
        &class,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
        true,
    );
    assert_eq!(methods[0].4, vec![booleans]);
    assert_eq!(
        methods[1].4,
        vec![thaw_hir::HirType::Array(Box::new(strings.clone()))]
    );
    let property = &class.properties[0];
    assert!(generate_napi_class_property_getter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(generate_napi_class_property_setter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(shim.contains("value: { name: string }[]"));
    assert!(shim.contains("values: boolean[]"));
    assert!(shim.contains("values: string[][]"));

    let rewritten = rewrite_external_class_methods(
        "const box = new ArrayBox([\"a\"]); box.flags([true, false]); box.group([[\"a\"]]);",
        &[("pkg".into(), "ArrayBox".into(), constructors)],
        &[
            (
                "ArrayBox".into(),
                "flags".into(),
                methods[0].1.clone(),
                1,
                false,
                methods[0].4.clone(),
            ),
            (
                "ArrayBox".into(),
                "group".into(),
                methods[1].1.clone(),
                1,
                false,
                methods[1].4.clone(),
            ),
        ],
    )
    .unwrap();
    assert!(!rewritten.contains("new ArrayBox"));
    assert!(rewritten.contains(&methods[0].1));
    assert!(rewritten.contains(&methods[1].1));
}

#[test]
fn generates_typed_napi_nullable_shims() {
    let class = thaw_bridge::parse_dts_classes(
        r#"export class NullableBox {
                constructor(value: string | null);
                normalize(value: string | null): string | null;
                value: string | null;
            }"#,
    )
    .unwrap()
    .remove(0);
    let nullable = thaw_hir::HirType::Nullable(Box::new(thaw_hir::HirType::Str));
    let mut shim = String::new();
    let constructors = generate_napi_class_constructors(&class, true, &mut shim);
    assert_eq!(constructors[0].2, vec![nullable.clone()]);
    let methods = generate_napi_class_method_overloads(
        &class,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
        true,
    );
    assert_eq!(methods[0].4, vec![nullable]);
    let property = &class.properties[0];
    assert!(generate_napi_class_property_getter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(generate_napi_class_property_setter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(shim.contains("value: string | null"));
    assert!(shim.contains("): string | null"));

    let rewritten = rewrite_external_class_methods(
        "const box = new NullableBox(null); box.normalize(null);",
        &[("pkg".into(), "NullableBox".into(), constructors)],
        &[(
            "NullableBox".into(),
            "normalize".into(),
            methods[0].1.clone(),
            1,
            false,
            methods[0].4.clone(),
        )],
    )
    .unwrap();
    assert!(!rewritten.contains("new NullableBox"));
    assert!(rewritten.contains(&methods[0].1));
}

#[test]
fn generates_typed_napi_optional_and_nullish_shims() {
    let class = thaw_bridge::parse_dts_classes(
        r#"export class OptionalBox {
                constructor(value: string | undefined);
                normalize(value: string | undefined): string | undefined;
                mixed(value: string | null | undefined): string | null | undefined;
                value: string | undefined;
            }"#,
    )
    .unwrap()
    .remove(0);
    let optional = thaw_hir::HirType::Optional(Box::new(thaw_hir::HirType::Str));
    let nullish = thaw_hir::HirType::Nullish(Box::new(thaw_hir::HirType::Str));
    let mut shim = String::new();
    let constructors = generate_napi_class_constructors(&class, true, &mut shim);
    assert_eq!(constructors[0].2, vec![optional.clone()]);
    let methods = generate_napi_class_method_overloads(
        &class,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
        true,
    );
    assert_eq!(methods[0].4, vec![optional]);
    assert_eq!(methods[1].4, vec![nullish]);
    let property = &class.properties[0];
    assert!(generate_napi_class_property_getter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(generate_napi_class_property_setter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(shim.contains("value: string | undefined"));
    assert!(shim.contains("value: string | null | undefined"));

    let rewritten = rewrite_external_class_methods(
        "const box = new OptionalBox(undefined); box.normalize(undefined); box.mixed(null);",
        &[("pkg".into(), "OptionalBox".into(), constructors)],
        &[
            (
                "OptionalBox".into(),
                "normalize".into(),
                methods[0].1.clone(),
                1,
                false,
                methods[0].4.clone(),
            ),
            (
                "OptionalBox".into(),
                "mixed".into(),
                methods[1].1.clone(),
                1,
                false,
                methods[1].4.clone(),
            ),
        ],
    )
    .unwrap();
    assert!(!rewritten.contains("new OptionalBox"));
    assert!(rewritten.contains(&methods[0].1));
    assert!(rewritten.contains(&methods[1].1));
}

#[test]
fn generates_napi_class_property_accessor_helpers() {
    let mut shim = String::new();
    let ty = thaw_bridge::DtsType::Native(thaw_hir::HirType::Str);
    let instance_getter =
        generate_napi_class_property_getter("Client", "name", &ty, false, &mut shim).unwrap();
    let (instance_setter, setter_type) =
        generate_napi_class_property_setter("Client", "name", &ty, false, &mut shim).unwrap();
    let static_getter =
        generate_napi_class_property_getter("Client", "version", &ty, true, &mut shim).unwrap();
    let (static_setter, _) =
        generate_napi_class_property_setter("Client", "version", &ty, true, &mut shim).unwrap();

    assert_eq!(setter_type, thaw_hir::HirType::Str);
    assert!(shim.contains(&format!(
        "declare function {instance_getter}(receiver: JsValue): string;"
    )));
    assert!(shim.contains(&format!(
        "declare function {instance_setter}(receiver: JsValue, value: string): string;"
    )));
    assert!(shim.contains(&format!("declare function {static_getter}(): string;")));
    assert!(shim.contains(&format!(
        "declare function {static_setter}(value: string): string;"
    )));
    assert!(generate_napi_class_property_getter(
        "Client",
        "optional",
        &thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
            thaw_hir::HirType::Str,
        ))),
        false,
        &mut shim,
    )
    .is_some());
}

#[test]
fn rewrites_inherited_external_class_methods() {
    let classes = thaw_bridge::parse_dts_classes(
        r#"export class Base {
                constructor(value: number);
                inherited(value: number): number;
            }
            export class Derived extends Base {}"#,
    )
    .unwrap();
    let derived = classes
        .iter()
        .find(|class| class.name == "Derived")
        .unwrap();
    let mut shim = String::new();
    let constructors = generate_napi_class_constructors(derived, true, &mut shim);
    assert_eq!(constructors.len(), 1);
    assert_eq!(constructors[0].0, 1);
    let generated = generate_napi_class_method_overloads(
        derived,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
        true,
    );
    let inherited = generated
        .iter()
        .find(|(method, _, _, _, _)| method == "inherited")
        .unwrap();
    let rewritten = rewrite_external_class_methods(
        "const value = new Derived(4); value.inherited(4);",
        &[(
            "pkg".into(),
            "Derived".into(),
            vec![(1, "Derived_ctor".into(), vec![])],
        )],
        &[(
            "Derived".into(),
            inherited.0.clone(),
            inherited.1.clone(),
            inherited.2,
            inherited.3,
            inherited.4.clone(),
        )],
    )
    .unwrap();
    assert!(rewritten.contains(&format!("{}(value, 4)", inherited.1)));
}

#[test]
fn rewrites_methods_on_values_created_from_external_classes() {
    let source = "const db = new Database(\":memory:\"); db.configure(\"busyTimeout\", 1000); const local = new LocalBox(1); local.configure(2);";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "sqlite3".into(),
            "Database".into(),
            vec![(1, "Database_ctor".into(), vec![])],
        )],
        &[(
            "Database".into(),
            "configure".into(),
            "__thaw_configure".into(),
            2,
            false,
            vec![thaw_hir::HirType::Str, thaw_hir::HirType::F64],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const db = Database_ctor(\":memory:\"); __thaw_configure(db, \"busyTimeout\", 1000); const local = new LocalBox(1); local.configure(2);"
        );
}

#[test]
fn rewrites_named_and_namespace_static_class_methods() {
    let source = "NativeBox.create(1); addon.NativeBox.create(\"text\"); LocalBox.create(2);";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[],
        &[],
        &[],
        &[
            (
                "addon".into(),
                "NativeBox".into(),
                "create".into(),
                "__thaw_create_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "addon".into(),
                "NativeBox".into(),
                "create".into(),
                "__thaw_create_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "__thaw_create_number(1); __thaw_create_string(\"text\"); LocalBox.create(2);"
    );
}

#[test]
fn rewrites_typed_napi_instance_getters() {
    let source = "const box = new NativeBox(42); console.log(box.value);";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[],
        &[],
        &[],
        &[(
            "NativeBox".into(),
            "value".into(),
            "__thaw_get_value".into(),
        )],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const box = NativeBox_ctor(42); console.log(__thaw_get_value(box));"
    );
}

#[test]
fn rewrites_typed_napi_instance_setters_and_preserves_expression_values() {
    let source = "const box = new NativeBox(42); const assigned: number = box.value = 7;";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[],
        &[],
        &[],
        &[],
        &[(
            "NativeBox".into(),
            "value".into(),
            "__thaw_set_value".into(),
            thaw_hir::HirType::F64,
        )],
        &[],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const box = NativeBox_ctor(42); const assigned: number = __thaw_set_value(box, 7);"
    );
}

#[test]
fn rewrites_named_and_namespace_static_accessors() {
    let source = "console.log(NativeBox.version); addon.NativeBox.version = 7; console.log(addon.NativeBox.version);";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[(
            "addon".into(),
            "NativeBox".into(),
            "version".into(),
            "__thaw_get_version".into(),
        )],
        &[(
            "addon".into(),
            "NativeBox".into(),
            "version".into(),
            "__thaw_set_version".into(),
            thaw_hir::HirType::F64,
        )],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "console.log(__thaw_get_version()); __thaw_set_version(7); console.log(__thaw_get_version());"
        );
}

#[test]
fn generates_typed_napi_static_method_shims_without_instance_receivers() {
    let class = thaw_bridge::DtsClass {
        name: "NativeBox".into(),
        extends: None,
        constructible: true,
        constructors: vec![],
        methods: vec![thaw_bridge::DtsMethod {
            name: "create".into(),
            params: vec![(
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            )],
            required_params: 1,
            rest_param: None,
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            callback_instance_classes: vec![vec![]],
            literal_params: vec![None],
            is_static: true,
            kind: thaw_bridge::DtsMethodKind::Method,
            overloaded: false,
        }],
        properties: vec![],
    };
    let mut shim = String::new();
    let generated = generate_napi_class_method_overloads(
        &class,
        true,
        &std::collections::HashMap::new(),
        &mut shim,
        true,
    );
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].0, "create");
    assert!(shim.contains("(value: number): number;"));
    assert!(!shim.contains("receiver"));
}

#[test]
fn generates_napi_method_arity_that_omits_an_unsupported_optional_parameter() {
    let class = thaw_bridge::DtsClass {
        name: "Database".into(),
        extends: None,
        constructible: true,
        constructors: vec![],
        methods: vec![thaw_bridge::DtsMethod {
            name: "run".into(),
            params: vec![
                (
                    "sql".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
                ),
                (
                    "params".into(),
                    thaw_bridge::DtsType::Unsupported("`any` is not supported".into()),
                ),
                (
                    "callback".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::Function(
                        vec![thaw_hir::HirType::Json],
                        Box::new(thaw_hir::HirType::Void),
                    )),
                ),
            ],
            required_params: 2,
            rest_param: None,
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Void),
            callback_instance_classes: vec![vec![], vec![], vec![]],
            literal_params: vec![None, None, None],
            is_static: false,
            kind: thaw_bridge::DtsMethodKind::Method,
            overloaded: false,
        }],
        properties: vec![],
    };
    let mut shim = String::new();
    let generated = generate_napi_class_method_overloads(
        &class,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
        true,
    );
    assert_eq!(generated.len(), 2);
    assert_eq!(generated[0].2, 2);
    assert_eq!(generated[1].2, 3);
    assert!(shim.contains("receiver: JsValue, sql: string, params: Json"));
}

#[test]
fn rewrites_zero_argument_external_class_methods() {
    let source = "const box = new NativeBox(42); const value = box.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const box = NativeBox_ctor(42); const value = __thaw_get(box);"
    );
}

#[test]
fn tracks_external_class_instance_aliases_and_invalidates_reassignments() {
    let source = "const box = new NativeBox(42); const alias = box; alias.get(); let assigned = alias; assigned.get(); assigned = box; assigned.get(); assigned = unknown; assigned.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = NativeBox_ctor(42); const alias = box; __thaw_get(alias); let assigned = alias; __thaw_get(assigned); assigned = box; __thaw_get(assigned); assigned = unknown; assigned.get();"
        );
}

#[test]
fn tracks_external_class_instances_through_object_properties() {
    let source = "const box = new NativeBox(42); const holder = { box }; holder.box.get(); holder[\"box\"].get(); const nested = { inner: { value: new NativeBox(7) } }; nested.inner.value.get(); holder.box = box; holder.box.get(); holder.box = unknown; holder.box.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = NativeBox_ctor(42); const holder = { box }; __thaw_get(holder.box); __thaw_get(holder[\"box\"]); const nested = { inner: { value: NativeBox_ctor(7) } }; __thaw_get(nested.inner.value); holder.box = box; __thaw_get(holder.box); holder.box = unknown; holder.box.get();"
        );
}

#[test]
fn joins_object_property_instance_facts_across_branches() {
    let source = "const box = new NativeBox(42); let holder = { box }; if (flag) { holder.box = box; } else { holder.box = box; } holder.box.get(); if (flag) { holder.box = box; } else { holder.box = unknown; } holder.box.get(); holder = unknown; holder.box.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = NativeBox_ctor(42); let holder = { box }; if (flag) { holder.box = box; } else { holder.box = box; } __thaw_get(holder.box); if (flag) { holder.box = box; } else { holder.box = unknown; } holder.box.get(); holder = unknown; holder.box.get();"
        );
}

#[test]
fn selects_external_method_overloads_by_arity_and_callback_shape() {
    let source = "const db = new Database(\":memory:\"); const done = (error: Json): void => {}; db.run(\"select 1\"); db.run(\"select 1\", done);";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "sqlite3".into(),
            "Database".into(),
            vec![(1, "Database_ctor".into(), vec![])],
        )],
        &[
            (
                "Database".into(),
                "run".into(),
                "__run_sync".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "Database".into(),
                "run".into(),
                "__run_callback".into(),
                2,
                true,
                vec![
                    thaw_hir::HirType::Str,
                    thaw_hir::HirType::Function(
                        vec![thaw_hir::HirType::Json],
                        Box::new(thaw_hir::HirType::Void),
                    ),
                ],
            ),
        ],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const db = Database_ctor(\":memory:\"); const done = (error: Json): void => {}; __run_sync(db, \"select 1\"); __run_callback(db, \"select 1\", done);"
        );
}
