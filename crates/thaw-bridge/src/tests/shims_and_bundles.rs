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
            value_exports: &[],
        },
        ModuleBundle {
            package_name: "is-odd",
            js_source: "function isOdd(n) { return n % 2 === 1; }",
            fallback_names: &[],
            qualified_aliases: &[],
            nested_namespace_aliases: &[],
            value_exports: &[],
        },
    ];
    let init = generate_module_init(&bundles);
    assert!(init.starts_with("function __thaw_module_init(): void {\n"));
    assert!(init.contains("function pad(s) { return s; }"));
    assert!(init.contains("function isOdd(n) { return n % 2 === 1; }"));
    assert!(init.contains("Buffer.isBuffer(value)"));
    assert!(init.contains("error.code = 'MODULE_NOT_FOUND'"));
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
        value_exports: &[],
    }];
    let init = generate_module_init(&bundles);
    // The wrapper adds its own quotes/backslashes/newlines too; this
    // just confirms the *bundle's own* problematic characters survived
    // escaping correctly once embedded inside the wrapped script.
    assert!(init.contains(r#"return \"a\\b\";\n"#));
}

#[test]
fn large_module_init_compresses_embedded_javascript() {
    let source = "module.exports.value = 1;\n".repeat(200);
    let init = generate_module_init(&[ModuleBundle {
        package_name: "large-package",
        js_source: &source,
        fallback_names: &[],
        qualified_aliases: &[],
        nested_namespace_aliases: &[],
        value_exports: &[],
    }]);

    assert!(init.contains("loadScript(\"gz:"));
    assert!(!init.contains(&source));
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
    assert!(wrapped.contains("globalThis.process.dlopen = function(target)"));
    assert!(wrapped.contains("__thaw_addon.QueryEngine"));
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
        value_exports: &[],
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
            value_exports: &[],
        },
        ModuleBundle {
            package_name: "@hapi/hoek",
            js_source: "module.exports = { stringify: function(x) { return 'hoek:' + x; } };",
            fallback_names: &hoek_fallback,
            qualified_aliases: &hoek_aliases,
            nested_namespace_aliases: &[],
            value_exports: &[],
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
