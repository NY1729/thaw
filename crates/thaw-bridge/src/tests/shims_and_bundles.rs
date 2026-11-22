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
        },
        ModuleBundle {
            package_name: "is-odd",
            js_source: "function isOdd(n) { return n % 2 === 1; }",
        },
    ];
    let init = generate_module_init(&bundles);
    assert!(init.starts_with("function __thaw_module_init(): void {\n"));
    assert!(init.contains("loadThawJs(\"left-pad\", \"function pad(s) { return s; }\")"), "{init}");
    assert!(init.contains("function isOdd(n) { return n % 2 === 1; }"));
    assert!(!init.contains("loadScript("), "{init}");
    // Loaded in the given order.
    assert!(init.find("left-pad").unwrap() < init.find("is-odd").unwrap());
}

#[test]
fn escapes_quotes_and_newlines_in_bundled_source() {
    let bundles = [ModuleBundle {
        package_name: "pkg",
        js_source: "function f() {\n  return \"a\\b\";\n}",
    }];
    let init = generate_module_init(&bundles);
    // The bundle's own quotes, backslashes and newlines survive escaping into the string literal.
    assert!(init.contains(r#"{\n  return \"a\\b\";\n}"#), "{init}");
}

/// Host requirements are computed from the raw bundle source.
#[test]
fn large_module_init_preserves_intl_requirements_as_metadata() {
    let source = format!(
        "{}\nnew Intl.Segmenter(); new Intl.NumberFormat('de'); new Intl.DateTimeFormat('ja');\n",
        "module.exports.value = 1;\n".repeat(200)
    );
    let init = generate_module_init(&[ModuleBundle {
        package_name: "large-intl-package",
        js_source: &source,
    }]);

    assert!(init.contains("loadThawJs(\"large-intl-package\""), "{init}");
    assert!(required_runtime_features(&source).contains("intl"));
    for constructor in ["NumberFormat", "DateTimeFormat", "Segmenter"] {
        let source = format!("new Intl.{constructor}('de')");
        assert!(required_runtime_features(&source).contains("intl"));
    }
    // A bare `Intl` reference (e.g. console.log(Intl)) and Intl.DisplayNames must also link it.
    assert!(required_runtime_features("console.log(Intl)").contains("intl"));
    assert!(required_runtime_features("new Intl.DisplayNames(['en'], { type: 'language' })").contains("intl"));
}


#[test]
fn native_addon_export_filter_uses_captured_intrinsics() {
    use std::ffi::{CStr, CString};

    thaw_js_shim::enable_napi_proxy_layer();
    let source = CString::new(
        r#"
        globalThis.checkNativeAddonExportFilter = function () {
          const forEach = Array.prototype.forEach;
          const indexOf = Array.prototype.indexOf;
          const startsWith = String.prototype.startsWith;
          const slice = String.prototype.slice;
          try {
            Array.prototype.forEach = Array.prototype.indexOf = function () { throw Error('array replaced'); };
            String.prototype.startsWith = String.prototype.slice = function () { throw Error('string replaced'); };
            const info = { names: ['a::one', 'b::two', 'plain'], qualifiedPackages: ['a'] };
            const qualified = [];
            const bare = [];
            __thaw_json_graph_napi_each_export(info, 'a::', 'a', (name, alias) => {
              qualified[qualified.length] = [name, alias];
            });
            __thaw_json_graph_napi_each_export(info, 'c::', 'c', (name, alias) => {
              bare[bare.length] = [name, alias];
            });
            return [qualified, bare];
          } finally {
            Array.prototype.forEach = forEach;
            Array.prototype.indexOf = indexOf;
            String.prototype.startsWith = startsWith;
            String.prototype.slice = slice;
          }
        };
        "#,
    )
    .unwrap();
    assert_eq!(thaw_js_shim::thaw_js_load(source.as_ptr()), 1);
    let result = thaw_js_shim::thaw_js_call(c"checkNativeAddonExportFilter".as_ptr(), c"[]".as_ptr());
    assert_eq!(
        unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
        "[[[\"a::one\",\"one\"]],[[\"plain\",\"plain\"]]]"
    );
}

/// Same bar as `generated_shim_round_trips_through_real_lowering`: the
/// generated `__thaw_module_init` must actually be valid, lowerable
/// Thaw source, not just plausible text.
#[test]
fn generated_module_init_round_trips_through_real_lowering() {
    let init = generate_module_init(&[ModuleBundle {
        package_name: "greeter",
        js_source: "function greet(){return 'hi';}",
    }]);
    let program_source = format!(
            "{init}\nfunction main(): void {{\n    console.log(\"ok\");\n}}\n"
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


#[test]
fn thaw_js_bundles_are_loaded_by_load_thaw_js_not_quickjs() {
    let init = generate_module_init(&[ModuleBundle {
        package_name: "ms",
        js_source: "module.exports = function ms() {};",
    }]);
    assert!(init.contains("loadThawJs(\"ms\", \"module.exports = function ms() {};\")"), "{init}");
    assert!(!init.contains("loadScript("), "{init}");
}
