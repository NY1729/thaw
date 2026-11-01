// Experimental thaw-js routing (`THAW_JS_RUNTIME=1`). These call `build_with_thaw_js_routing`
// directly so the tests never touch the process environment.

#[test]
fn thaw_js_declaration_scan_accepts_plain_data_and_rejects_everything_else() {
    let plain = |line: &str| thaw_js_declaration_plain(line);
    assert!(plain(
        "declare function __thaw_typed_js_6d73(value: number, options: { long: boolean }): string;"
    ));
    assert!(plain(
        "declare function __thaw_typed_js_6d73__arity_1(args: string[]): JsValue;"
    ));
    assert!(plain(
        "declare function __thaw_typed_js_6d73(a: string | number, b?: Json): null | undefined;"
    ));
    // type parameters declared by the function itself are fine (inferred from plain arguments)
    assert!(plain(
        "declare function __thaw_typed_js_6d73__generic_rest__arity_1<T>(array: T[]): T[];"
    ));
    assert!(plain("declare function __thaw_typed_js_6d73<TObject, TPath extends string>(data: TObject, path: TPath): JsValue;"));
    // callbacks, classes and other named types cannot cross the JSON boundary
    assert!(!plain(
        "declare function __thaw_typed_js_6d73(f: (x: number) => number): number;"
    ));
    assert!(!plain(
        "declare function __thaw_typed_js_6d73(v: Buffer): number;"
    ));
    assert!(!plain(
        "declare function __thaw_typed_js_6d73(v: SemVer): number;"
    ));
    assert!(!plain(
        "declare function __thaw_typed_js_6d73(): Promise<number>;"
    ));
    assert!(!plain("function __thaw_typed_wrapper_js_6d73(): number {"));
}

#[test]
fn thaw_js_symbols_decode_to_their_package_function() {
    let line = "declare function __thaw_typed_js_6d733a3a6d73__overload_0__arity_2(value: number): string;";
    assert_eq!(thaw_js_declared_function(line, "ms").as_deref(), Some("ms"));
    assert_eq!(thaw_js_declared_function(line, "other"), None);
    let generic = "declare function __thaw_typed_js_6c6f646173683a3a676574__generic_rest__arity_1<T>(a: T[]): T[];";
    assert_eq!(
        thaw_js_declared_function(generic, "lodash").as_deref(),
        Some("get")
    );
    assert_eq!(
        thaw_js_declared_function("declare function __thaw_typed_js_zz(): number;", "ms"),
        None
    );
    assert!(thaw_js_mentions("ms(1)", "ms") && !thaw_js_mentions("ms(1)", "semver"));
    assert!(thaw_js_mentions("a.get(1)", "a.get") && thaw_js_mentions("get()", "ns.get"));
}

#[test]
fn thaw_js_routed_packages_are_read_from_the_generated_shim() {
    let shim = "function __thaw_module_init(): void {\n    // a\n    if (!loadThawJs(\"semver\", \"x\")) throw 1;\n    if (!loadThawJs(\"ms\", \"y\")) throw 1;\n    if (!loadThawJs(\"ms\", \"y\")) throw 1;\n    loadScript(\"z\");\n}";
    assert_eq!(thaw_js_routed_packages(shim), ["ms", "semver"]);
    assert!(thaw_js_routed_packages("loadScript(\"z\");").is_empty());
}

/// `tjs-kit`: functions whose bodies the specialization JIT cannot take (they use `Map`), so the
/// QuickJS path is the baseline and thaw-js is the replacement.
fn thaw_js_fixture(name: &str, main: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("thaw-cli-thaw-js-{name}-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("tjs-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function describe(items: string[], label: string): any;\n\
         export declare function add(a: number, b: number): number;\n\
         export declare function apply(items: number[], f: (x: number) => number): number[];\n\
         export declare function pairs(items: string[]): string[][];\n\
         export type DateArg<D extends Date> = D | number | string;\n\
         export declare function shift<T extends Date, R extends Date = T>(date: DateArg<T> & {}, days: number): R;\n\
         export declare function iso<T extends Date>(date: DateArg<T> & {}): string;\n",
    )
    .unwrap();
    // The registry's bundle layout (one factory per module, a require map and the entry key), plus
    // the evaluation tail that makes the same text runnable as a plain QuickJS script.
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = (function(require){\n\
         var __thaw_bundle_factories = {\n\
         \"tjs-kit/index.js\": function(module, exports, require) {\n\
           exports.describe = function (items, label) { var m = new Map(); items.forEach(function (x) { m.set(x, (m.get(x) || 0) + 1); }); var counts = {}; m.forEach(function (n, k) { counts[k] = n; }); return { label: label.toUpperCase(), counts: counts }; };\n\
           exports.add = function (a, b) { var m = new Map(); m.set('t', a); return m.get('t') + b; };\n\
           exports.apply = function (items, f) { return items.map(f); };\n\
           exports.shift = function (d, n) { return new Date(d.getTime() + n * 86400000); };\n\
           exports.iso = function (d) { return d.toISOString(); };\n\
           exports.pairs = function (items) { var out = []; for (var i = 0; i < items.length; i += 2) out.push(items.slice(i, i + 2)); return out; };\n\
         },\n\
         };\n\
         var __thaw_bundle_require_maps = { \"tjs-kit/index.js\": {}, };\n\
         var __thaw_bundle_entry_key = \"tjs-kit/index.js\";\n\
         var __thaw_entry = { exports: {} };\n\
         __thaw_bundle_factories[__thaw_bundle_entry_key](__thaw_entry, __thaw_entry.exports, require);\n\
         return __thaw_entry.exports;\n\
         })(globalThis.require);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, main).unwrap();
    (dir.clone(), registry, entry)
}

fn thaw_js_build(entry: &Path, output: &Path, registry: &Path, routed: bool) -> serde_json::Value {
    build_with_thaw_js_routing(
        entry,
        output,
        &[],
        &[],
        &[],
        registry,
        &[],
        false,
        None,
        true,
        false,
        false,
        routed,
    )
    .unwrap();
    artifact_manifest_from_bytes(&std::fs::read(output).unwrap()).unwrap()
}

fn thaw_js_run(output: &Path) -> String {
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8_lossy(&result.stdout).into_owned()
}

#[test]
fn thaw_js_routing_serves_a_plain_data_package_without_linking_quickjs() {
    let (dir, registry, entry) = thaw_js_fixture(
        "plain",
        "import { describe, add } from 'tjs-kit';\n\
         function main(): void { console.log(describe(['a', 'b', 'a'], 'x')); console.log(add(2, 3)); console.log(describe([], 'none')); }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-kit"]));
    // `apply` (a callback parameter) is declared but never mentioned, so it does not block routing.
    std::fs::remove_dir_all(&registry).unwrap();
    // Same text real Node prints for the equivalent JavaScript.
    assert_eq!(
        thaw_js_run(&output),
        "{ label: 'X', counts: { a: 2, b: 1 } }\n5\n{ label: 'NONE', counts: {} }\n"
    );
    // The linked binary carries no QuickJS engine at all.
    let bytes = std::fs::read(&output).unwrap();
    assert!(
        !bytes.windows(8).any(|window| window == b"rquickjs"),
        "QuickJS was linked"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_carries_dates_both_ways_without_linking_quickjs() {
    let (dir, registry, entry) = thaw_js_fixture(
        "dates",
        "import { shift, iso } from 'tjs-kit';\n\
         function main(): void { const d = new Date(Date.UTC(2024, 2, 5, 10)); console.log(iso(shift(d, 2))); console.log(iso(shift(d, 1))); console.log(iso(d)); }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-kit"]));
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(
        thaw_js_run(&output),
        "2024-03-07T10:00:00.000Z\n2024-03-06T10:00:00.000Z\n2024-03-05T10:00:00.000Z\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_off_is_the_unchanged_quickjs_path_with_the_same_result() {
    let main =
        "import { add } from 'tjs-kit';\nfunction main(): void { console.log(add(40, 2)); }\n";
    let (dir, registry, entry) = thaw_js_fixture("parity", main);
    let routed_output = dir.join("routed");
    let plain_output = dir.join("plain");
    let routed = thaw_js_build(&entry, &routed_output, &registry, true);
    let plain = thaw_js_build(&entry, &plain_output, &registry, false);
    assert_eq!(routed["quickjs"], false);
    assert_eq!(plain["quickjs"], true);
    assert_eq!(plain["thaw_js_routed"], serde_json::json!([]));
    assert_eq!(thaw_js_run(&routed_output), "42\n");
    assert_eq!(thaw_js_run(&plain_output), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_decodes_a_nested_array_result_without_linking_quickjs() {
    // A typed `string[][]` result is decoded from plain JSON: no interned-native-owner query (which
    // would need the QuickJS realm) may be compiled into a thaw-js build.
    let main = "import { pairs } from 'tjs-kit';\nfunction main(): void { const p = pairs(['a', 'b', 'c']); console.log(p.length, p[0][1], p[1][0]); }\n";
    let (dir, registry, entry) = thaw_js_fixture("nested-array", main);
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-kit"]));
    assert_eq!(thaw_js_run(&output), "2 b c\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn quickjs_path_prints_a_plain_object_result_like_node() {
    // The package's `any` result is a live QuickJS object; `console.log` must inspect it like Node
    // (not `String(value)`, which printed `[object Object]`).
    let main = "import { describe } from 'tjs-kit';\nfunction main(): void { console.log(describe(['a', 'b', 'a'], 'x')); }\n";
    let (dir, registry, entry) = thaw_js_fixture("quickjs-print", main);
    let output = dir.join("plain");
    let manifest = thaw_js_build(&entry, &output, &registry, false);
    assert_eq!(manifest["quickjs"], true);
    assert_eq!(
        thaw_js_run(&output),
        "{ label: 'X', counts: { a: 2, b: 1 } }\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_serves_a_callback_through_thaw_js_without_quickjs() {
    let (dir, registry, entry) = thaw_js_fixture(
        "callback",
        "import { apply } from 'tjs-kit';\nfunction main(): void { console.log(apply([1, 2, 3], (x: number): number => x * 2).join(',')); }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-kit"]));
    assert_eq!(thaw_js_run(&output), "2,4,6\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_serves_real_leaf_packages_like_node_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    // (package, program, stdout of real Node 22 for the equivalent `require` program)
    let cases = [
        (
            "ms",
            "import ms from 'ms';\nfunction main(): void { console.log(ms('2 days')); console.log(ms(60000)); }\n",
            "172800000\n1m\n",
        ),
        (
            "minimist",
            "import minimist from 'minimist';\nfunction main(): void { console.log(minimist(['-a', '1', '--b=2', 'x'])); }\n",
            "{ _: [ 'x' ], a: 1, b: 2 }\n",
        ),
        (
            "semver",
            "import semver from 'semver';\nfunction main(): void { console.log(semver.gt('1.2.3', '1.2.0')); console.log(semver.satisfies('1.2.3', '^1.0.0')); console.log(semver.inc('1.2.3', 'minor')); console.log(semver.valid('1.2')); }\n",
            "true\ntrue\n1.3.0\nnull\n",
        ),
        (
            "lodash",
            "import { chunk, sum, uniq, camelCase, get, merge } from 'lodash';\nfunction main(): void { console.log(chunk([1, 2, 3, 4, 5], 2)); console.log(sum([1, 2, 3])); console.log(uniq([1, 1, 2])); console.log(camelCase('foo bar')); console.log(get({ a: { b: 3 } }, 'a.b')); console.log(merge({ a: 1 }, { b: 2 })); }\n",
            "[ [ 1, 2 ], [ 3, 4 ], [ 5 ] ]\n6\n[ 1, 2 ]\nfooBar\n3\n{ a: 1, b: 2 }\n",
        ),
    ];
    for (package, program, expected) in cases {
        let dir = std::env::temp_dir().join(format!(
            "thaw-cli-thaw-js-real-{package}-{}",
            std::process::id()
        ));
        let registry = dir.join("modules");
        std::fs::create_dir_all(&registry).unwrap();
        let entry = dir.join("main.ts");
        std::fs::write(&entry, program).unwrap();
        let output = dir.join("app");
        build_with_thaw_js_routing(
            &entry,
            &output,
            &[],
            &[],
            &[],
            &registry,
            &[],
            false,
            None,
            true,
            true,
            false,
            true,
        )
        .unwrap();
        let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(manifest["quickjs"], false, "{package}: {manifest}");
        assert_eq!(
            manifest["thaw_js_routed"],
            serde_json::json!([package]),
            "{package}"
        );
        assert_eq!(thaw_js_run(&output), expected, "{package}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn thaw_js_async_overload_is_plain_when_its_resolved_type_is() {
    assert!(thaw_js_async_declaration_plain(
        "declare function __thaw_typed_js_6d73(t: string, d: Json): Promise<string>;"
    ));
    assert!(thaw_js_async_declaration_plain(
        "declare function __thaw_typed_js_6d73(t: string): string | Promise<string>;"
    ));
    assert!(!thaw_js_async_declaration_plain(
        "declare function __thaw_typed_js_6d73(t: string): Promise<Buffer>;"
    ));
    assert!(!thaw_js_async_declaration_plain(
        "declare function __thaw_typed_js_6d73(t: Promise<string>): string;"
    ));
}

#[test]
fn thaw_js_path_flavours_are_registry_bundles_exporting_the_selected_flavour() {
    let root = thaw_registry::resolve_builtin("node:path")
        .unwrap()
        .bundle_js
        .unwrap();
    for (builtin, separator) in [
        ("node:path", "/"),
        ("node:path/posix", "/"),
        ("node:path/win32", "\\"),
    ] {
        let form = thaw_js_path_bundle(&root, builtin);
        let report = thaw_js::scan::scan_bundle(&form).unwrap();
        assert!(report.is_clean(), "{builtin}: {:?}", report.blockers);
        assert_eq!(report.entry, builtin);
        let mut package = thaw_js::host::Package::load(&form).unwrap();
        assert_eq!(
            package.get("sep").ok(),
            Some(thaw_js::host::Native::Str(separator.into())),
            "{builtin}"
        );
    }
}

#[test]
fn thaw_js_builtin_modules_are_an_explicit_allowlist() {
    for module in [
        "node:events",
        "node:path",
        "node:assert",
        "node:os",
        "node:util",
        "node:constants",
        "node:url",
        "node:fs",
    ] {
        assert!(THAW_JS_BUILTIN_MODULES.contains(&module), "{module}");
    }
    // modules whose cases do not match Node on thaw-js yet stay on QuickJS
    for module in [
        "node:stream",
        "node:crypto",
        "node:net",
        "node:zlib",
    ] {
        assert!(!THAW_JS_BUILTIN_MODULES.contains(&module), "{module}");
    }
}

#[test]
fn thaw_js_builtin_modules_include_promise_variants() {
    // every mini-manifest node-compat case of these modules matches Node under THAW_JS_RUNTIME=1 (tj9)
    for module in ["node:timers/promises", "node:fs/promises"] {
        assert!(THAW_JS_BUILTIN_MODULES.contains(&module), "{module}");
    }
}

#[test]
fn thaw_js_bundle_form_wraps_plain_scripts_and_keeps_registered_forms() {
    let mut forms = std::collections::HashMap::new();
    forms.insert("node:x".to_string(), "registered".to_string());
    assert_eq!(
        thaw_js_bundle_form(&forms, "node:x", "ignored"),
        "registered"
    );
    let wrapped = thaw_js_bundle_form(&forms, "node:y", "module.exports = 1;");
    assert!(thaw_js::scan::scan_bundle(&wrapped).unwrap().is_clean());
}
