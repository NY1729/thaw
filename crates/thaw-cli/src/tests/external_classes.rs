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
        &std::collections::HashMap::new(),
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
                None,
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
                None,
            ),
            (
                "Socket".into(),
                "send".into(),
                "Socket_send".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
                None,
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
        &std::collections::HashMap::new(),
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
        &std::collections::HashMap::new(),
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
        &std::collections::HashMap::new(),
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
            rest_param: None,
            overloaded: false,
        }],
        methods: vec![],
        properties: vec![],
    };
    let mut shim = String::new();
    let helpers = generate_napi_class_constructors(&class, true, &std::collections::HashMap::new(), &mut shim);
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
        &std::collections::HashMap::new(),
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
        &std::collections::HashMap::new(), &mut default_shim,
    );
    assert_eq!(default_helpers.len(), 1);
    assert_eq!(default_helpers[0].0, 0);
    assert!(default_shim.contains("(): JsValue;"));

    let mut locked_shim = String::new();
    let mut locked = class;
    locked.constructible = false;
    assert!(generate_napi_class_constructors(&locked, true, &std::collections::HashMap::new(), &mut locked_shim).is_empty());
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
            rest_param: None,
            overloaded: false,
        }],
        methods: vec![],
        properties: vec![],
    };
    let mut shim = String::new();
    let helpers = generate_napi_class_constructors(&class, false, &std::collections::HashMap::new(), &mut shim);
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
                rest_param: None,
                overloaded: true,
            },
            thaw_bridge::DtsConstructor {
                params: vec![(
                    "value".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                )],
                required_params: 1,
                rest_param: None,
                overloaded: true,
            },
        ],
        methods: vec![],
        properties: vec![],
    };
    let mut shim = String::new();
    let helpers = generate_napi_class_constructors(&class, true, &std::collections::HashMap::new(), &mut shim);
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
fn selects_constructor_overloads_by_object_method_shape() {
    let callback = thaw_hir::HirType::Dynamic;
    let basic = thaw_hir::HirType::Object(vec![("transform".into(), callback.clone())]);
    let flush = thaw_hir::HirType::Object(vec![
        ("transform".into(), callback.clone()),
        ("flush".into(), callback),
    ]);
    let rewritten = rewrite_external_class_methods(
        "const a = new NativeBox({ transform() {} }); const b = new NativeBox({ transform() {}, flush() {} });",
        &[(
            "pkg".into(),
            "NativeBox".into(),
            vec![
                (1, "basic_ctor".into(), vec![basic]),
                (1, "flush_ctor".into(), vec![flush]),
            ],
        )],
        &[],
    )
    .unwrap();
    assert!(rewritten.contains("const a = basic_ctor({ transform() {} })"));
    assert!(rewritten.contains("const b = flush_ctor({ transform() {}, flush() {} })"));
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
    let constructors = generate_napi_class_constructors(&class, true, &std::collections::HashMap::new(), &mut shim);
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
        true,
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
        &std::collections::HashMap::new(),
    )
    .unwrap();
    assert!(!rewritten.contains("new PairBox"));
    assert!(rewritten.contains(&methods[0].1));
}

#[test]
fn class_returning_methods_keep_a_live_js_handle() {
    let class = thaw_bridge::parse_dts_classes(
        "export class Hash { update(value: string): Hash; digest(): string; }",
    )
    .unwrap()
    .remove(0);
    let mut shim = String::new();
    generate_napi_class_method_overloads(
        &class,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
        false,
    );
    assert!(shim.contains("receiver: JsValue, value: string): JsValue;"));
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
    let constructors = generate_napi_class_constructors(&class, true, &std::collections::HashMap::new(), &mut shim);
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
        true,
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
    let constructors = generate_napi_class_constructors(&class, true, &std::collections::HashMap::new(), &mut shim);
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
        true,
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
        &std::collections::HashMap::new(),
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
    let constructors = generate_napi_class_constructors(&class, true, &std::collections::HashMap::new(), &mut shim);
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
        true,
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
        generate_napi_class_property_setter("Client", "name", &ty, false, true, &mut shim)
            .unwrap();
    let static_getter =
        generate_napi_class_property_getter("Client", "version", &ty, true, &mut shim).unwrap();
    let (static_setter, _) =
        generate_napi_class_property_setter("Client", "version", &ty, true, true, &mut shim)
            .unwrap();

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
    let constructors = generate_napi_class_constructors(derived, true, &std::collections::HashMap::new(), &mut shim);
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
        &std::collections::HashMap::new(),
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
        &std::collections::HashMap::new(),
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
            return_instance_class: None,
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
            return_instance_class: None,
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
        &std::collections::HashMap::new(),
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
        &std::collections::HashMap::new(),
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
        &std::collections::HashMap::new(),
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
        &std::collections::HashMap::new(),
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

#[test]
fn constructor_rest_emits_helper_for_observed_arity() {
    let class = thaw_bridge::parse_dts_classes(
        "declare class Parts { constructor(...parts: string[]); }",
    )
    .unwrap()
    .remove(0);
    let observed = std::collections::HashMap::from([(
        "Parts".to_string(),
        std::collections::BTreeSet::from([2]),
    )]);
    let mut shim = String::new();
    let helpers = generate_napi_class_constructors(&class, true, &observed, &mut shim);
    assert!(helpers.iter().any(|(arity, _, _)| *arity == 2));
    assert!(shim.contains("(parts0: string, parts1: string): JsValue;"));
    let two_arg_helper = helpers.iter().find(|(arity, _, _)| *arity == 2).unwrap().1.clone();
    let rewritten = rewrite_external_class_constructors(
        "new pkg.ns.Parts('a', 'b')",
        &[("pkg".into(), "ns.Parts".into(), helpers)],
        &std::collections::HashMap::new(),
    ).unwrap();
    assert!(rewritten.starts_with(&two_arg_helper));
}

#[test]
fn imported_constructor_aliases_keep_package_identity() {
    let classes = vec![
        ("one".into(), "Client".into(), vec![(1, "one_ctor".into(), vec![])]),
        ("two".into(), "Client".into(), vec![(1, "two_ctor".into(), vec![])]),
        ("one".into(), "default".into(), vec![(1, "default_ctor".into(), vec![])]),
        ("one".into(), "nested.Client".into(), vec![(1, "nested_ctor".into(), vec![])]),
    ];
    let packages = std::collections::HashMap::from([
        ("package-one".into(), "one".into()),
        ("package-two".into(), "two".into()),
    ]);
    let source = "import { Client as First, nested as Nested } from 'package-one'; import { Client as Second } from 'package-two'; import DefaultClient from 'package-one'; import * as Namespace from 'package-two'; new First(1); new Second(2); new DefaultClient(3); new Namespace.Client(4); new Nested.Client(5);";
    let rewritten = rewrite_external_class_constructors(source, &classes, &packages).unwrap();
    assert!(rewritten.contains("one_ctor(1)"));
    assert!(rewritten.contains("two_ctor(2)"));
    assert!(rewritten.contains("default_ctor(3)"));
    assert!(rewritten.contains("two_ctor(4)"));
    assert!(rewritten.contains("nested_ctor(5)"));
}

#[test]
fn constructor_alias_rewrite_respects_local_shadowing() {
    let classes = vec![("pkg".into(), "Client".into(), vec![(0, "pkg_ctor".into(), vec![])])];
    let packages = std::collections::HashMap::from([("package".into(), "pkg".into())]);
    let source = "import { Client as Alias } from 'package'; new Alias(); { const Alias = Local; new Alias(); } new Alias(); function f(Alias: any) { new Alias(); }";
    let rewritten = rewrite_external_class_constructors(source, &classes, &packages).unwrap();
    assert_eq!(rewritten.matches("pkg_ctor()").count(), 2);
    assert_eq!(rewritten.matches("new Alias()").count(), 2);
}

#[test]
fn constructor_alias_rewrite_respects_hoisted_vars() {
    let classes = vec![("pkg".into(), "Client".into(), vec![(0, "pkg_ctor".into(), vec![])])];
    let packages = std::collections::HashMap::from([("package".into(), "pkg".into())]);
    let source = "import { Client as Alias } from 'package'; function f() { new Alias(); if (false) { var Alias; } } const g = () => { new Alias(); if (false) { var Alias; } }; new Alias();";
    let rewritten = rewrite_external_class_constructors(source, &classes, &packages).unwrap();
    assert_eq!(rewritten.matches("pkg_ctor()").count(), 1);
    assert_eq!(rewritten.matches("new Alias()").count(), 2);
}

#[test]
fn default_declarations_shadow_external_constructor_names() {
    let classes = vec![("pkg".into(), "Client".into(), vec![(0, "pkg_ctor".into(), vec![])])];
    let class_source = "export default class Client {} new Client();";
    let rewritten = rewrite_external_class_constructors(
        class_source, &classes, &std::collections::HashMap::new(),
    ).unwrap();
    assert_eq!(rewritten, class_source);

    let function_source = "export default function Client() {} new Client();";
    let rewritten = rewrite_external_class_constructors(
        function_source, &classes, &std::collections::HashMap::new(),
    ).unwrap();
    assert_eq!(rewritten, function_source);
}

#[test]
fn switch_case_lexical_bindings_shadow_imports_across_cases() {
    let classes = vec![("pkg".into(), "Client".into(), vec![(0, "pkg_ctor".into(), vec![])])];
    let packages = std::collections::HashMap::from([("package".into(), "pkg".into())]);
    let source = "import { Client as Alias } from 'package'; switch (mode) { case 0: new Alias(); break; case 1: let Alias = Local; new Alias(); } new Alias();";
    let rewritten = rewrite_external_class_constructors(source, &classes, &packages).unwrap();
    assert_eq!(rewritten.matches("pkg_ctor()").count(), 1);
    assert_eq!(rewritten.matches("new Alias()").count(), 2);
}

#[test]
fn default_exported_class_name_is_available_to_constructor_aliases() {
    assert_eq!(
        commonjs_export_name("export default class Client {}").unwrap(),
        Some("Client".to_string())
    );
}

#[test]
fn constructor_alias_rewrite_ignores_unresolved_imports_and_shadowed_namespaces() {
    let classes = vec![("pkg".into(), "Client".into(), vec![(0, "pkg_ctor".into(), vec![])])];
    let packages = std::collections::HashMap::from([("package".into(), "pkg".into())]);
    let source = "import { Client } from './local'; import * as P from 'package'; new Client(); new P.Client(); function f(P: any) { new P.Client(); }";
    let rewritten = rewrite_external_class_constructors(source, &classes, &packages).unwrap();
    assert_eq!(rewritten.matches("pkg_ctor()").count(), 1);
    assert_eq!(rewritten.matches("new Client()").count(), 1);
    assert_eq!(rewritten.matches("new P.Client()").count(), 1);
}

#[test]
fn export_assignment_namespace_import_constructs_root_class() {
    let classes = vec![("pkg".into(), "__namespace_root__".into(), vec![(0, "pkg_ctor".into(), vec![])])];
    let packages = std::collections::HashMap::from([("package".into(), "pkg".into())]);
    let source = "import * as Factory from 'package'; new Factory();";
    let rewritten = rewrite_external_class_constructors(source, &classes, &packages).unwrap();
    assert!(rewritten.contains("pkg_ctor()"));
}


#[test]
fn qualified_namespace_classes_rewrite_distinct_construction_methods_and_returns() {
    let parsed = thaw_bridge::parse_dts_classes(
        "declare namespace A { class Client { constructor(value: number); kind(): string; } } \
         declare namespace B { class Client { constructor(value: string); kind(): string; } }",
    ).unwrap();
    let mut shim = String::new();
    let a = generate_napi_class_constructors(
        parsed.iter().find(|class| class.name == "A.Client").unwrap(),
        true, &std::collections::HashMap::new(), &mut shim,
    );
    let b = generate_napi_class_constructors(
        parsed.iter().find(|class| class.name == "B.Client").unwrap(),
        true, &std::collections::HashMap::new(), &mut shim,
    );
    assert_ne!(a[0].1, b[0].1);
    let classes = vec![
        ("pkg".into(), "A.Client".into(), vec![(1, "a_ctor".into(), vec![thaw_hir::HirType::F64])]),
        ("pkg".into(), "B.Client".into(), vec![(1, "b_ctor".into(), vec![thaw_hir::HirType::Str])]),
    ];
    let source = "const a = new pkg.A.Client(1); const b = new pkg.B.Client('x'); a.kind(); b.kind(); const again = a.chain(); again.kind(); pkg.A.Client.describe(); pkg.B.Client.describe();";
    let constructors = rewrite_external_class_constructors(
        source, &classes, &std::collections::HashMap::new(),
    ).unwrap();
    assert!(constructors.contains("a_ctor(1)"));
    assert!(constructors.contains("b_ctor('x')"));

    let methods: Vec<ClassMethodRewrite> = vec![
        ("A.Client".into(), "kind".into(), "a_kind".into(), 0, false, vec![], None),
        ("B.Client".into(), "kind".into(), "b_kind".into(), 0, false, vec![], None),
        ("A.Client".into(), "chain".into(), "a_chain".into(), 0, false, vec![], Some("A.Client".into())),
    ];
    let static_methods: Vec<StaticClassMethodRewrite> = vec![
        ("pkg".into(), "A.Client".into(), "describe".into(), "a_describe".into(), 0, false, vec![]),
        ("pkg".into(), "B.Client".into(), "describe".into(), "b_describe".into(), 0, false, vec![]),
    ];
    let rewritten = rewrite_external_class_methods_with_static(
        source, &classes, &methods, &[], &static_methods, &[], &[], &[], &[], &[], &[],
    ).unwrap();
    assert_eq!(rewritten.matches("a_kind(").count(), 2);
    assert_eq!(rewritten.matches("b_kind(").count(), 1);
    assert!(rewritten.contains("a_chain("));
    assert!(rewritten.contains("a_describe()"));
    assert!(rewritten.contains("b_describe()"));

    let imported = "import { A as Alias } from 'package'; import * as Bundle from 'package'; \
        const x = new Alias.Client(1); x.kind(); Alias.Client.describe(); \
        const y = new Bundle.B.Client('x'); y.kind(); Bundle.B.Client.describe();";
    let imported_rewrites = rewrite_external_class_methods_with_static(
        imported, &classes, &methods, &[], &static_methods, &[], &[], &[], &[], &[], &[],
    ).unwrap();
    assert!(imported_rewrites.contains("a_kind("));
    assert!(imported_rewrites.contains("b_kind("));
    assert!(imported_rewrites.contains("a_describe()"));
    assert!(imported_rewrites.contains("b_describe()"));
}


#[test]
fn named_class_import_alias_keeps_constructor_instance_and_static_methods() {
    let classes = vec![("pkg".into(), "Client".into(), vec![(0, "client_ctor".into(), vec![])])];
    let methods: Vec<ClassMethodRewrite> = vec![
        ("Client".into(), "kind".into(), "client_kind".into(), 0, false, vec![], None),
    ];
    let static_methods: Vec<StaticClassMethodRewrite> = vec![
        ("pkg".into(), "Client".into(), "describe".into(), "client_describe".into(), 0, false, vec![]),
    ];
    let source = "import { Client as Alias } from 'package'; const c = new Alias(); c.kind(); Alias.describe();";
    let imported = rewrite_external_class_methods_with_static(
        source, &classes, &methods, &[], &static_methods, &[], &[], &[], &[], &[], &[],
    ).unwrap();
    assert!(imported.contains("client_kind("));
    assert!(imported.contains("client_describe()"));
    let constructors = rewrite_external_class_constructors(
        source, &classes, &std::collections::HashMap::from([("package".into(), "pkg".into())]),
    ).unwrap();
    assert!(constructors.contains("client_ctor()"));
}

#[test]
fn same_named_classes_keep_package_identity_through_methods_and_returns() {
    let qualifiers = std::collections::HashMap::from([
        ("first-package".into(), "first".into()),
        ("second-package".into(), "second".into()),
    ]);
    let source = "import { Client as First } from 'first-package'; import { Client as Second } from 'second-package'; \
        import * as Other from 'second-package'; \
        const a = new First(); const b = new Second(); const c = new Other.Client(); \
        a.kind(); b.kind(); c.kind(); const next = b.chain(); next.kind(); First.describe(); Second.describe(); Other.Client.describe();";
    for reverse in [false, true] {
        let mut classes = vec![
            ("first".into(), "Client".into(), vec![(0, "first_ctor".into(), vec![])]),
            ("second".into(), "Client".into(), vec![(0, "second_ctor".into(), vec![])]),
        ];
        let mut methods: Vec<ClassMethodRewrite> = vec![
            ("first::Client".into(), "kind".into(), "first_kind".into(), 0, false, vec![], None),
            ("second::Client".into(), "kind".into(), "second_kind".into(), 0, false, vec![], None),
            ("second::Client".into(), "chain".into(), "second_chain".into(), 0, false, vec![], Some("second::Client".into())),
        ];
        let mut statics: Vec<StaticClassMethodRewrite> = vec![
            ("first".into(), "Client".into(), "describe".into(), "first_describe".into(), 0, false, vec![]),
            ("second".into(), "Client".into(), "describe".into(), "second_describe".into(), 0, false, vec![]),
        ];
        if reverse { classes.reverse(); methods.reverse(); statics.reverse(); }
        let rewritten = rewrite_external_class_methods_with_static_qualified(
            source, &classes, &methods, &[], &statics, &[], &[], &[], &[], &[], &[], &qualifiers,
        ).unwrap();
        assert_eq!(rewritten.matches("first_kind(").count(), 1);
        assert_eq!(rewritten.matches("second_kind(").count(), 3);
        assert_eq!(rewritten.matches("second_chain(").count(), 1);
        assert_eq!(rewritten.matches("first_describe(").count(), 1);
        assert_eq!(rewritten.matches("second_describe(").count(), 2);
        let constructors = rewrite_external_class_constructors(source, &classes, &qualifiers).unwrap();
        assert_eq!(constructors.matches("first_ctor(").count(), 1);
        assert_eq!(constructors.matches("second_ctor(").count(), 2);
    }
}

#[test]
fn same_named_factories_keep_their_return_class_package() {
    let qualifiers = std::collections::HashMap::from([
        ("first-package".into(), "first".into()),
        ("second-package".into(), "second".into()),
    ]);
    let source = "import { make as firstMake } from 'first-package'; import { make as secondMake } from 'second-package'; \
        import * as other from 'second-package'; import DefaultSecond from 'second-package'; \
        const a = firstMake(); const b = secondMake(); const c = other.make(); const d = DefaultSecond(); \
        a.kind(); b.kind(); c.kind(); d.kind();";
    for reverse in [false, true] {
        let mut factories: Vec<FactoryClassRewrite> = vec![
            ("first::make".into(), "first::Client".into()),
            ("second::make".into(), "second::Client".into()),
        ];
        let mut methods: Vec<ClassMethodRewrite> = vec![
            ("first::Client".into(), "kind".into(), "first_kind".into(), 0, false, vec![], None),
            ("second::Client".into(), "kind".into(), "second_kind".into(), 0, false, vec![], None),
        ];
        if reverse { factories.reverse(); methods.reverse(); }
        let rewritten = rewrite_external_class_methods_with_static_qualified(
            source, &[], &methods, &[], &[], &[], &[], &[], &[], &factories, &[], &qualifiers,
        ).unwrap();
        assert_eq!(rewritten.matches("first_kind(").count(), 1);
        assert_eq!(rewritten.matches("second_kind(").count(), 3);
    }
}

#[test]
fn class_methods_respect_shadowed_import_aliases_and_restore_outer_imports() {
    let qualifiers = std::collections::HashMap::from([("package".into(), "pkg".into())]);
    let classes = vec![("pkg".into(), "Client".into(), vec![(0, "client_ctor".into(), vec![])])];
    let methods: Vec<ClassMethodRewrite> = vec![
        ("pkg::Client".into(), "kind".into(), "client_kind".into(), 0, false, vec![], None),
    ];
    let statics: Vec<StaticClassMethodRewrite> = vec![
        ("pkg".into(), "Client".into(), "describe".into(), "client_describe".into(), 0, false, vec![]),
    ];
    let source = "import { Client as Alias } from 'package'; import * as Ns from 'package'; \
        const outer = new Alias(); outer.kind(); Alias.describe(); Ns.Client.describe(); \
        function parameter(Alias: any, { Ns }: any) { const local = new Alias(); local.kind(); Alias.describe(); Ns.Client.describe(); } \
        const arrow = (Alias: any) => { const local = new Alias(); local.kind(); Alias.describe(); }; \
        { Alias.describe(); let Alias: any; Alias.describe(); } \
        try {} catch (Alias) { Alias.describe(); } \
        function hoisted() { Alias.describe(); var Alias: any; } \
        Alias.describe(); Ns.Client.describe();";
    let rewritten = rewrite_external_class_methods_with_static_qualified(
        source, &classes, &methods, &[], &statics, &[], &[], &[], &[], &[], &[], &qualifiers,
    ).unwrap();
    assert_eq!(rewritten.matches("client_kind(").count(), 1);
    assert_eq!(rewritten.matches("client_describe(").count(), 4);
    assert!(rewritten.contains("function parameter(Alias: any, { Ns }: any) { const local = new Alias(); local.kind(); Alias.describe(); Ns.Client.describe(); }"));
    assert!(rewritten.contains("{ Alias.describe(); let Alias: any; Alias.describe(); }"));
    let constructors = rewrite_external_class_constructors(source, &classes, &qualifiers).unwrap();
    assert_eq!(constructors.matches("client_ctor(").count(), 1);
}

#[test]
fn factory_return_tracking_ignores_shadowed_import_alias() {
    let qualifiers = std::collections::HashMap::from([("package".into(), "pkg".into())]);
    let factories: Vec<FactoryClassRewrite> = vec![("pkg::make".into(), "pkg::Client".into())];
    let methods: Vec<ClassMethodRewrite> = vec![
        ("pkg::Client".into(), "kind".into(), "client_kind".into(), 0, false, vec![], None),
    ];
    let source = "import { make as factory } from 'package'; \
        const external = factory(); external.kind(); \
        function local(factory: any) { const value = factory(); value.kind(); } \
        { const value = factory(); value.kind(); let factory: any; } \
        const again = factory(); again.kind();";
    let rewritten = rewrite_external_class_methods_with_static_qualified(
        source, &[], &methods, &[], &[], &[], &[], &[], &[], &factories, &[], &qualifiers,
    ).unwrap();
    assert_eq!(rewritten.matches("client_kind(").count(), 2);
    assert!(rewritten.contains("const value = factory(); value.kind(); let factory: any;"));
}

#[test]
fn type_only_class_alias_keeps_instance_methods_without_runtime_constructor() {
    // Unrun regression: a type-only alias and a live value alias share
    // structural class metadata, but only the latter gets a constructor.
    let source = r#"
        declare class Client {
            constructor(value: string);
            method(): string;
            static create(): Client;
        }
        export type { Client };
        export type { Client as TypeClient };
        export { Client as LiveClient };
    "#;
    let mut classes = thaw_bridge::parse_dts_classes(source).unwrap();
    let type_names = thaw_bridge::exported_type_names(source);
    let value_names = thaw_bridge::exported_value_names(source);
    let only_types = exclusive_type_only_value_names(&type_names, &value_names, &[], &classes, &[]);
    restrict_type_only_class_values(&mut classes, &only_types);
    let internal = classes.iter().find(|class| class.name == "Client").unwrap();
    assert!(!internal.constructible);
    assert!(internal.constructors.is_empty());
    let ty = classes.iter().find(|class| class.name == "TypeClient").unwrap();
    assert!(!ty.constructible);
    assert!(ty.constructors.is_empty());
    assert!(ty.methods.iter().any(|method| method.name == "method" && !method.is_static));
    assert!(!ty.methods.iter().any(|method| method.is_static));
    let live = classes.iter().find(|class| class.name == "LiveClient").unwrap();
    assert!(live.constructible);
    assert!(!live.constructors.is_empty());
    assert!(live.methods.iter().any(|method| method.name == "create" && method.is_static));
}

#[test]
fn type_only_namespace_gates_nested_class_runtime_value() {
    let source = r#"
        declare namespace Types {
            export class Client { constructor(); method(): string; static create(): Client; }
        }
        export type { Types };
    "#;
    let mut classes = thaw_bridge::parse_dts_classes(source).unwrap();
    let type_names = thaw_bridge::exported_type_names(source);
    let value_names = thaw_bridge::exported_value_names(source);
    let only_types = exclusive_type_only_value_names(&type_names, &value_names, &[], &classes, &[]);
    assert!(only_types.contains("Types.Client"));
    restrict_type_only_class_values(&mut classes, &only_types);
    let client = classes.iter().find(|class| class.name == "Types.Client").unwrap();
    assert!(!client.constructible);
    assert!(client.methods.iter().any(|method| method.name == "method"));
    assert!(!client.methods.iter().any(|method| method.is_static));
}

#[test]
fn qualified_namespace_functions_keep_separate_runtime_keys() {
    // Unrun regression: two live `make` methods are distinct, while a
    // type-only namespace contributes no captured runtime member.
    let source = r#"
        declare namespace Types { function make(value: boolean): string; }
        export type { Types };
        declare namespace Left { function make(value: string): string; }
        declare namespace Right { function make(value: number): number; }
        export { Left, Right };
    "#;
    let functions = thaw_bridge::parse_dts(source).unwrap();
    let type_only = thaw_bridge::type_only_namespace_names(source);
    let mut namespaces = thaw_bridge::nested_namespace_members(source);
    namespaces.retain(|name, _| !type_only.contains(name));
    for (namespace, members) in &mut namespaces {
        for (member, target) in members {
            let path = format!("{namespace}.{member}");
            if functions.iter().any(|function| function.name == path) { *target = path; }
        }
    }
    assert!(!namespaces.contains_key("Types"));
    assert_eq!(namespaces["Left"]["make"], "Left.make");
    assert_eq!(namespaces["Right"]["make"], "Right.make");
    assert_ne!(function_identifier("Left.make"), function_identifier("Right.make"));
    assert!(!function_identifier("Left.make").contains('.'));
    let rewrites = vec![("pkg".to_string(), "Left.make".to_string(),
        "pkg___thaw_function_4c6566742e6d616b65".to_string())];
    let rewritten = rewrite_qualified_calls(
        "function main(): void { pkg.Left.make('x'); }",
        &rewrites,
        &std::collections::HashSet::new(),
    ).unwrap();
    assert!(rewritten.contains("pkg___thaw_function_4c6566742e6d616b65('x')"), "{rewritten}");
}

#[test]
fn deeply_qualified_namespace_function_keeps_its_runtime_path() {
    // Unrun regression: the nested map and package call rewrite agree on
    // the full path, including two namespaces with a shared bare method.
    let source = r#"
        declare namespace Outer {
            namespace Left { function make(value: string): string; }
            namespace Right { function make(value: number): number; }
        }
        export { Outer };
    "#;
    let functions = thaw_bridge::parse_dts(source).unwrap();
    assert!(functions.iter().any(|function| function.name == "Outer.Left.make"));
    assert!(functions.iter().any(|function| function.name == "Outer.Right.make"));
    let nested = thaw_bridge::nested_namespace_members(source);
    assert_eq!(nested["Outer"]["Left.make"], "Left.make");
    assert_eq!(nested["Outer"]["Right.make"], "Right.make");
    let alias = format!("pkg_{}", function_identifier("Outer.Left.make"));
    let rewritten = rewrite_qualified_calls(
        "function main(): void { pkg.Outer.Left.make('x'); }",
        &[("pkg".to_string(), "Outer.Left.make".to_string(), alias.clone())],
        &std::collections::HashSet::new(),
    ).unwrap();
    assert!(rewritten.contains(&format!("{alias}('x')")), "{rewritten}");
}

#[test]
fn value_export_of_same_class_name_wins_over_type_only_marker() {
    let source = "export declare class Client { constructor(); static create(): Client; } export type { Client };";
    let mut classes = thaw_bridge::parse_dts_classes(source).unwrap();
    let type_names = thaw_bridge::exported_type_names(source);
    let value_names = thaw_bridge::exported_value_names(source);
    let only_types = exclusive_type_only_value_names(&type_names, &value_names, &[], &classes, &[]);
    assert!(!only_types.contains("Client"));
    restrict_type_only_class_values(&mut classes, &only_types);
    assert!(classes.iter().find(|class| class.name == "Client").unwrap().constructible);
}

#[test]
fn explicitly_type_only_callable_has_no_runtime_function_classification() {
    let source = "declare function f(value: string): number; export type { f };";
    let mut functions = thaw_bridge::parse_dts(source).unwrap();
    let only_types = exclusive_type_only_value_names(
        &thaw_bridge::exported_type_names(source),
        &thaw_bridge::exported_value_names(source),
        &functions, &[], &[]);
    functions.retain(|function| !only_types.contains(&function.name));
    assert!(functions.is_empty());
}

#[test]
fn external_class_iteration_flow_includes_the_zero_iteration_path() {
    let classes = vec![("addon".into(), "NativeBox".into(),
        vec![(1, "NativeBox_ctor".into(), vec![])])];
    let methods = vec![("NativeBox".into(), "get".into(),
        "__thaw_get".into(), 0, false, vec![])];
    for loop_head in ["for (const key in {})", "for (const item of [])"] {
        let source = format!("let box = unknown; {loop_head} {{ box = new NativeBox(1); box.get(); }} box.get();");
        let rewritten = rewrite_external_class_methods(
            &source, &classes, &methods).unwrap();
        assert_eq!(rewritten.matches("__thaw_get(box)").count(), 1, "{rewritten}");
        assert!(rewritten.ends_with("box.get();"), "{rewritten}");

        let source = format!("let box = new NativeBox(1); {loop_head} {{ box = new NativeBox(2); }} box.get();");
        let rewritten = rewrite_external_class_methods(
            &source, &classes, &methods).unwrap();
        assert!(rewritten.ends_with("__thaw_get(box);"), "{rewritten}");

        let source = format!("let box = new NativeBox(1); {loop_head} {{ box = unknown; }} box.get();");
        let rewritten = rewrite_external_class_methods(
            &source, &classes, &methods).unwrap();
        assert!(rewritten.ends_with("box.get();"), "{rewritten}");
    }
}

#[test]
fn external_class_latent_function_bodies_do_not_change_outer_flow() {
    let classes = vec![("addon".into(), "NativeBox".into(),
        vec![(1, "NativeBox_ctor".into(), vec![])])];
    let methods = vec![("NativeBox".into(), "get".into(),
        "__thaw_get".into(), 0, false, vec![])];
    for latent in [
        "function later() { box = new NativeBox(1); box.get(); }",
        "const later = function() { box = new NativeBox(1); box.get(); };",
        "const later = () => { box = new NativeBox(1); box.get(); };",
        "class Later { method() { box = new NativeBox(1); box.get(); } }",
        "class Later { constructor() { box = new NativeBox(1); box.get(); } }",
        "function later(value = (box = new NativeBox(1))) {}",
    ] {
        let source = format!("let box = unknown; {latent} box.get();");
        let rewritten = rewrite_external_class_methods(
            &source, &classes, &methods).unwrap();
        assert!(rewritten.ends_with("box.get();"), "{rewritten}");
        let expected_inside = usize::from(latent.contains("box.get();"));
        assert_eq!(rewritten.matches("__thaw_get(box)").count(), expected_inside, "{rewritten}");
    }

    let source = "let box = new NativeBox(1); const later = () => { box = unknown; }; box.get();";
    let rewritten = rewrite_external_class_methods(
        source, &classes, &methods).unwrap();
    assert!(rewritten.ends_with("__thaw_get(box);"), "{rewritten}");
}

#[test]
fn external_class_definition_time_and_block_effects_remain_visible() {
    let classes = vec![("addon".into(), "NativeBox".into(),
        vec![(1, "NativeBox_ctor".into(), vec![])])];
    let methods = vec![("NativeBox".into(), "get".into(),
        "__thaw_get".into(), 0, false, vec![])];
    for effect in [
        "{ box = new NativeBox(1); }",
        "class Later { [((box = new NativeBox(1)), 'method')]() {} }",
        "class Later { constructor() {} [((box = new NativeBox(1)), 'method')]() {} }",
        "class Later { @decorate(box = new NativeBox(1)) method() {} }",
    ] {
        let source = format!("let box = unknown; {effect} box.get();");
        let rewritten = rewrite_external_class_methods(
            &source, &classes, &methods).unwrap();
        assert!(rewritten.ends_with("__thaw_get(box);"), "{rewritten}");
    }
}

#[test]
fn external_class_callback_seed_survives_latent_flow_restore() {
    let source = "let box = unknown; const server = new Server(); \
        server.on('connection', (socket) => { box = new NativeBox(1); socket.send('hi'); }); \
        box.get();";
    let classes = vec![
        ("pkg".into(), "Server".into(), vec![(0, "Server_ctor".into(), vec![])]),
        ("addon".into(), "NativeBox".into(), vec![(1, "NativeBox_ctor".into(), vec![])]),
    ];
    let methods = vec![
        ("Server".into(), "on".into(), "Server_on".into(), 2, true,
            vec![thaw_hir::HirType::Str, thaw_hir::HirType::Function(
                vec![thaw_hir::HirType::Json], Box::new(thaw_hir::HirType::Void))], None),
        ("Socket".into(), "send".into(), "Socket_send".into(), 1, false,
            vec![thaw_hir::HirType::Str], None),
        ("NativeBox".into(), "get".into(), "__thaw_get".into(), 0, false, vec![], None),
    ];
    let contexts = vec![ClassMethodContext::CallbackInstance(
        "Server_on".into(), 1, 0, "Socket".into())];
    let rewritten = rewrite_external_class_methods_with_static(
        source, &classes, &methods, &contexts, &[], &[], &[], &[], &[], &[], &[],
    ).unwrap();
    assert!(rewritten.contains("Socket_send(socket, 'hi')"), "{rewritten}");
    assert!(rewritten.ends_with("box.get();"), "{rewritten}");
}

#[test]
fn external_class_parameter_decorators_run_once_at_definition() {
    let classes = vec![("addon".into(), "NativeBox".into(),
        vec![(1, "NativeBox_ctor".into(), vec![])])];
    let methods = vec![("NativeBox".into(), "get".into(),
        "__thaw_get".into(), 0, false, vec![])];
    for declaration in [
        "class Later { method(@decorate(box = new NativeBox(1), box.get()) value: any) {} }",
        "class Later { constructor(@decorate(box = new NativeBox(1), box.get()) value: any) {} }",
        "class Later { constructor(@decorate(box = new NativeBox(1), box.get()) public value: any) {} }",
    ] {
        let source = format!("let box = unknown; {declaration} box.get();");
        let rewritten = rewrite_external_class_methods(&source, &classes, &methods).unwrap();
        assert!(rewritten.ends_with("__thaw_get(box);"), "{rewritten}");
        assert_eq!(rewritten.matches("__thaw_get(box)").count(), 2, "{rewritten}");
    }
}
