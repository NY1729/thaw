use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

include!("tests/static_build.rs");

include!("tests/module_graph.rs");

include!("tests/registry_modules.rs");

include!("tests/ffi_metadata.rs");

include!("tests/native_addons.rs");

#[test]
fn rewrite_qualified_calls_is_a_no_op_with_no_rewrites() {
    let source = "function main(): void { console.log(qs.stringify(x)); }";
    assert_eq!(rewrite_qualified_calls(source, &[]).unwrap(), source);
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
    let rewritten = rewrite_qualified_calls(source, &rewrites).unwrap();

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
    let rewritten = rewrite_qualified_calls(source, &rewrites).unwrap();
    assert!(rewritten.contains("const r = String(qs_stringify(x));"));
}

#[test]
fn rewrites_external_class_constructors_without_touching_other_new_expressions() {
    let source = "const a = new Database(\":memory:\"); const b = new sqlite3.Database(\"db.sqlite\"); const c = new LocalBox(1);";
    let rewritten = rewrite_external_class_constructors(
        source,
        &[(
            "sqlite3".into(),
            "Database".into(),
            vec![(1, "Database_ctor".into())],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const a = Database_ctor(\":memory:\"); const b = Database_ctor(\"db.sqlite\"); const c = new LocalBox(1);"
        );
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
                (0, "Database_ctor0".into()),
                (1, "Database_ctor1".into()),
                (2, "Database_ctor2".into()),
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
    let helpers = generate_napi_class_constructors(&class, &mut shim);
    assert_eq!(
        helpers.iter().map(|(arity, _)| *arity).collect::<Vec<_>>(),
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
    for (_, helper) in helpers {
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
        &mut default_shim,
    );
    assert_eq!(default_helpers.len(), 1);
    assert_eq!(default_helpers[0].0, 0);
    assert!(default_shim.contains("(): JsValue;"));

    let mut locked_shim = String::new();
    let mut locked = class;
    locked.constructible = false;
    assert!(generate_napi_class_constructors(&locked, &mut locked_shim).is_empty());
    assert!(locked_shim.is_empty());
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
    .is_none());
}

#[test]
fn rewrites_inherited_external_class_methods() {
    let classes = thaw_bridge::parse_dts_classes(
        r#"export class Base { inherited(value: number): number; }
            export class Derived extends Base { constructor(); }"#,
    )
    .unwrap();
    let derived = classes
        .iter()
        .find(|class| class.name == "Derived")
        .unwrap();
    let mut shim = String::new();
    let generated = generate_napi_class_method_overloads(
        derived,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
    );
    let inherited = generated
        .iter()
        .find(|(method, _, _, _, _)| method == "inherited")
        .unwrap();
    let rewritten = rewrite_external_class_methods(
        "const value = new Derived(); value.inherited(4);",
        &[(
            "pkg".into(),
            "Derived".into(),
            vec![(0, "Derived_ctor".into())],
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
            vec![(1, "Database_ctor".into())],
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
            "const db = new Database(\":memory:\"); __thaw_configure(db, \"busyTimeout\", 1000); const local = new LocalBox(1); local.configure(2);"
        );
}

#[test]
fn rewrites_named_and_namespace_static_class_methods() {
    let source = "NativeBox.create(1); addon.NativeBox.create(\"text\"); LocalBox.create(2);";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
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
            vec![(1, "NativeBox_ctor".into())],
        )],
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
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const box = new NativeBox(42); console.log(__thaw_get_value(box));"
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
            vec![(1, "NativeBox_ctor".into())],
        )],
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
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const box = new NativeBox(42); const assigned: number = __thaw_set_value(box, 7);"
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
    );
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].0, "create");
    assert!(shim.contains("(value: number): number;"));
    assert!(!shim.contains("receiver"));
}

#[test]
fn rewrites_zero_argument_external_class_methods() {
    let source = "const box = new NativeBox(42); const value = box.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into())],
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
        "const box = new NativeBox(42); const value = __thaw_get(box);"
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
            vec![(1, "NativeBox_ctor".into())],
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
            "const box = new NativeBox(42); const alias = box; __thaw_get(alias); let assigned = alias; __thaw_get(assigned); assigned = box; __thaw_get(assigned); assigned = unknown; assigned.get();"
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
            vec![(1, "NativeBox_ctor".into())],
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
            "const box = new NativeBox(42); const holder = { box }; __thaw_get(holder.box); __thaw_get(holder[\"box\"]); const nested = { inner: { value: new NativeBox(7) } }; __thaw_get(nested.inner.value); holder.box = box; __thaw_get(holder.box); holder.box = unknown; holder.box.get();"
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
            vec![(1, "NativeBox_ctor".into())],
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
            "const box = new NativeBox(42); let holder = { box }; if (flag) { holder.box = box; } else { holder.box = box; } __thaw_get(holder.box); if (flag) { holder.box = box; } else { holder.box = unknown; } holder.box.get(); holder = unknown; holder.box.get();"
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
            vec![(1, "Database_ctor".into())],
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
            "const db = new Database(\":memory:\"); const done = (error: Json): void => {}; __run_sync(db, \"select 1\"); __run_callback(db, \"select 1\", done);"
        );
}

#[test]
fn selects_same_arity_external_method_overloads_by_argument_type() {
    let source = "const box = new NativeBox(1); const n = 42; const s = \"hello\"; box.set(n); box.set(s); box.set(7); box.set(\"world\");";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into())],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = new NativeBox(1); const n = 42; const s = \"hello\"; __set_number(box, n); __set_string(box, s); __set_number(box, 7); __set_string(box, \"world\");"
        );
}

#[test]
fn infers_external_overload_types_from_composed_expressions() {
    let source = r#"const box = new NativeBox(1); const n = 20 + 22; const s = "hel" + "lo"; const b = n > 0; const config = { n, nested: { text: s }, enabled: b }; box.set(n); box.set(s); box.set(b); box.set(Number("7")); box.set(`value-${s}`); box.set(true ? "yes" : "no"); box.set(config.n); box.set(config.nested.text); box.set(config.enabled); box.set(({ value: 7 }).value);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into())],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_boolean".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, n)"));
    assert!(rewritten.contains("__set_string(box, s)"));
    assert!(rewritten.contains("__set_boolean(box, b)"));
    assert!(rewritten.contains("__set_number(box, Number(\"7\"))"));
    assert!(rewritten.contains("__set_string(box, `value-${s}`)"));
    assert!(rewritten.contains("__set_string(box, true ? \"yes\" : \"no\")"));
    assert!(rewritten.contains("__set_number(box, config.n)"));
    assert!(rewritten.contains("__set_string(box, config.nested.text)"));
    assert!(rewritten.contains("__set_boolean(box, config.enabled)"));
    assert!(rewritten.contains("__set_number(box, ({ value: 7 }).value)"));
}

#[test]
fn infers_external_overload_types_from_user_function_returns_and_forward_references() {
    let source = r#"const box = new NativeBox(1); const makeText = (): string => "text"; const makeFlag = function(): boolean { return true; }; const inferredFlag = () => true; box.set(makeNumber()); box.set(makeText()); box.set(makeFlag()); box.set(inferredNumber()); box.set(inferredText()); box.set(inferredFlag()); function makeNumber(): number { return 42; } function inferredNumber() { return 40 + 2; } function inferredText() { return forwardText(); } function forwardText() { return "text"; }"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into())],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_boolean".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, makeNumber())"));
    assert!(rewritten.contains("__set_string(box, makeText())"));
    assert!(rewritten.contains("__set_boolean(box, makeFlag())"));
    assert!(rewritten.contains("__set_number(box, inferredNumber())"));
    assert!(rewritten.contains("__set_string(box, inferredText())"));
    assert!(rewritten.contains("__set_boolean(box, inferredFlag())"));
}

#[test]
fn selects_object_overloads_from_structural_property_types() {
    let source = r#"function makeNumeric(): { value: number } { return { value: 11 }; } const box = new NativeBox(1); const numeric = { value: 42 }; const textual = { value: "text" }; const choose = true; box.configure(numeric); box.configure(textual); box.configure({ value: 7 }); box.configure({ ["value"]: "computed" }); box.configure({ ...numeric }); box.configure({ ...numeric, value: "override" }); box.configure({ ...{ value: 9 } }); box.configure({ ...makeNumeric() }); box.configure({ ...(choose ? makeNumeric() : numeric) });"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into())],
        )],
        &[
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::F64,
                )])],
            ),
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::Str,
                )])],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__configure_number(box, numeric)"));
    assert!(rewritten.contains("__configure_string(box, textual)"));
    assert!(rewritten.contains("__configure_number(box, { value: 7 })"));
    assert!(rewritten.contains("__configure_string(box, { [\"value\"]: \"computed\" })"));
    assert!(rewritten.contains("__configure_number(box, { ...numeric })"));
    assert!(rewritten.contains("__configure_string(box, { ...numeric, value: \"override\" })"));
    assert!(rewritten.contains("__configure_number(box, { ...{ value: 9 } })"));
    assert!(rewritten.contains("__configure_number(box, { ...makeNumeric() })"));
    assert!(
        rewritten.contains("__configure_number(box, { ...(choose ? makeNumeric() : numeric) })")
    );
}

#[test]
fn tracks_assignment_flow_for_variables_and_nested_object_properties() {
    let source = r#"const box = new NativeBox(1); let value = 42; box.set(value); value = "text"; box.set(value); const config = { nested: { value: 1 }, direct: true }; config.nested.value = "nested"; config["direct"] = 7; box.set(config.nested.value); box.set(config.direct);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into())],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, value); value = \"text\""));
    assert!(rewritten.contains("__set_string(box, value); const config"));
    assert!(rewritten.contains("__set_string(box, config.nested.value)"));
    assert!(rewritten.contains("__set_number(box, config.direct)"));
}

#[test]
fn joins_if_branch_types_and_discards_conflicting_facts() {
    let source = r#"const box = new NativeBox(1); const flag = true; let stable = 1; if (flag) { stable = 2; } else { stable = 3; } box.set(stable); let conflict = 1; if (flag) { conflict = "text"; box.set(conflict); } else { conflict = 2; box.set(conflict); } box.set(conflict); let oneSided = 1; if (flag) { oneSided = "changed"; } box.set(oneSided);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into())],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, stable)"));
    assert!(rewritten.contains("conflict = \"text\"; __set_string(box, conflict)"));
    assert!(rewritten.contains("conflict = 2; __set_number(box, conflict)"));
    assert!(rewritten.contains("} __set_unknown(box, conflict)"));
    assert!(rewritten.contains("} __set_unknown(box, oneSided)"));
}

#[test]
fn joins_switch_fallthrough_break_and_no_match_paths() {
    let source = r#"const choice = 1; const flag = true; const box = new NativeBox(1); let stable = 1; switch (choice) { case 0: stable = 2; break; default: stable = 3; } box.set(stable); let conflict = 1; switch (choice) { case 0: conflict = "text"; break; default: conflict = 2; } box.set(conflict); let noDefault = 1; switch (choice) { case 0: noDefault = "text"; break; } box.set(noDefault); let fallen = 1; switch (choice) { case 0: fallen = "temporary"; case 1: fallen = 2; break; default: fallen = 3; } box.set(fallen); let guarded = 1; switch (choice) { case 0: if (flag) { guarded = "text"; break; guarded = 4; } guarded = 2; break; default: guarded = 3; } box.set(guarded); let loopBreak = 1; switch (choice) { case 0: while (flag) { break; } loopBreak = 2; break; default: loopBreak = 3; } box.set(loopBreak); let stableCallback = (): void => {}; switch (choice) { case 0: stableCallback = (): void => {}; break; default: stableCallback = (): void => {}; } box.use(stableCallback); let conflictCallback = (): void => {}; switch (choice) { case 0: conflictCallback = 1; break; default: conflictCallback = (): void => {}; } box.use(conflictCallback); let stableBox = new NativeBox(1); switch (choice) { case 0: stableBox = new NativeBox(2); break; default: stableBox = new NativeBox(3); } stableBox.get(); let conflictBox = new NativeBox(1); switch (choice) { case 0: conflictBox = "text"; break; default: conflictBox = new NativeBox(3); } conflictBox.get();"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into())],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "use".into(),
                "__use_value".into(),
                1,
                false,
                vec![thaw_hir::HirType::Json],
            ),
            (
                "NativeBox".into(),
                "use".into(),
                "__use_callback".into(),
                1,
                true,
                vec![thaw_hir::HirType::Json],
            ),
            (
                "NativeBox".into(),
                "get".into(),
                "__get".into(),
                0,
                false,
                Vec::new(),
            ),
        ],
    )
    .unwrap();

    assert!(rewritten.contains("__set_number(box, stable)"));
    assert!(rewritten.contains("__set_unknown(box, conflict)"));
    assert!(rewritten.contains("__set_unknown(box, noDefault)"));
    assert!(rewritten.contains("__set_number(box, fallen)"));
    assert!(rewritten.contains("__set_unknown(box, guarded)"));
    assert!(rewritten.contains("__set_number(box, loopBreak)"));
    assert!(rewritten.contains("__use_callback(box, stableCallback)"));
    assert!(rewritten.contains("__use_value(box, conflictCallback)"));
    assert!(rewritten.contains("__get(stableBox)"));
    assert!(rewritten.contains("conflictBox.get()"));
}

#[test]
fn joins_while_and_for_types_against_the_zero_iteration_path() {
    let source = r#"const box = new NativeBox(1); const flag = true; let stable = 1; while (flag) { stable = 2; box.set(stable); break; } box.set(stable); let changed = 1; while (flag) { changed = "text"; box.set(changed); break; } box.set(changed); let loopValue = 1; for (let index = 0; index < 1; index = index + 1) { box.set(index); loopValue = "loop"; box.set(loopValue); } box.set(loopValue);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into())],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("stable = 2; __set_number(box, stable)"));
    assert!(rewritten.contains("} __set_number(box, stable)"));
    assert!(rewritten.contains("changed = \"text\"; __set_string(box, changed)"));
    assert!(rewritten.contains("} __set_unknown(box, changed)"));
    assert!(rewritten.contains("__set_number(box, index)"));
    assert!(rewritten.contains("loopValue = \"loop\"; __set_string(box, loopValue)"));
    assert!(rewritten.contains("} __set_unknown(box, loopValue)"));
}

#[test]
fn joins_try_catch_paths_and_applies_finally_to_every_exit() {
    let source = r#"const box = new NativeBox(1); let stable = 1; try { stable = 2; } catch (error) { stable = 3; } box.set(stable); let conflict = 1; try { conflict = "try"; box.set(conflict); } catch (error) { conflict = 2; box.set(conflict); } box.set(conflict); let catchInput = 1; try { catchInput = "changed"; throw "fail"; } catch (error) { box.set(catchInput); } let finalized = 1; try { finalized = "try"; } catch (error) { finalized = true; } finally { finalized = 7; box.set(finalized); } box.set(finalized);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into())],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, stable)"));
    assert!(rewritten.contains("conflict = \"try\"; __set_string(box, conflict)"));
    assert!(rewritten.contains("conflict = 2; __set_number(box, conflict)"));
    assert!(rewritten.contains("} __set_unknown(box, conflict)"));
    assert!(rewritten.contains("catch (error) { __set_unknown(box, catchInput)"));
    assert!(rewritten.contains("finalized = 7; __set_number(box, finalized)"));
    assert!(rewritten.ends_with("__set_number(box, finalized);"));
}
