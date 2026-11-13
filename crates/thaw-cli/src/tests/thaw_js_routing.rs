// Experimental thaw-js routing (`THAW_JS_RUNTIME=1`). These call `build_with_thaw_js_routing`
// directly so the tests never touch the process environment.

#[test]
fn thaw_js_bytes_result_gate_switches_off_for_node_http_and_tls_programs() {
    let bytes = "declare function __thaw_typed_js_6d73(path: string): Uint8Array;";
    assert!(!thaw_js_declaration_plain_with(bytes, false, false));
    assert!(thaw_js_declaration_plain_with(bytes, true, false));
    assert!(thaw_js_declaration_plain(bytes));
    // a Uint8Array result under a Promise follows the same switch
    let async_bytes = "declare function __thaw_typed_js_6d73(path: string): Promise<Uint8Array>;";
    assert!(!thaw_js_async_declaration_plain_with(async_bytes, false));
    assert!(thaw_js_async_declaration_plain_with(async_bytes, true));
    // the switch only relaxes the bare bytes result, never a rejected parameter
    let buffer_param = "declare function __thaw_typed_js_6d73(v: Buffer): Uint8Array;";
    assert!(!thaw_js_declaration_plain_with(buffer_param, true, false));

    assert!(thaw_js_bytes_result_allowed("import fs from 'node:fs';\nfs.readFileSync(p);"));
    assert!(!thaw_js_bytes_result_allowed("import https from 'node:https';\nhttps.get(u);"));
    assert!(!thaw_js_bytes_result_allowed("import http from 'node:http';"));
    assert!(!thaw_js_bytes_result_allowed("import tls from 'node:tls';"));
    assert!(!thaw_js_bytes_result_allowed("import http2 from 'node:http2';"));
}

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
    // a bare `Uint8Array` result crosses as bytes; in params, unions and nested positions it stays out
    assert!(plain(
        "declare function __thaw_typed_js_6d73(path: string): Uint8Array;"
    ));
    assert!(!plain(
        "declare function __thaw_typed_js_6d73(v: Uint8Array): number;"
    ));
    assert!(!plain(
        "declare function __thaw_typed_js_6d73(): Uint8Array | string;"
    ));
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

/// `thaw_js_run` with a 20 s guard: a hung binary is killed, its captured output is printed, and the test
/// panics instead of stalling the whole test run.
fn thaw_js_run_guarded(output: &Path) -> String {
    let (stdout, stderr, code) = thaw_js_run_guarded_status(output);
    assert!(code == Some(0), "{stderr}");
    stdout
}

/// `thaw_js_run_guarded` without the success assertion: returns stdout, stderr and the exit code.
fn thaw_js_run_guarded_status(output: &Path) -> (String, String, Option<i32>) {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = Command::new(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut bytes);
        bytes
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let stdout = stdout_reader.join().unwrap();
            let stderr = stderr_reader.join().unwrap();
            eprintln!("hung child stdout:\n{}", String::from_utf8_lossy(&stdout));
            eprintln!("hung child stderr:\n{}", String::from_utf8_lossy(&stderr));
            panic!("hung: {} did not exit within 20s", output.display());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let status = child.wait().unwrap();
    let stdout = stdout_reader.join().unwrap();
    let stderr = stderr_reader.join().unwrap();
    (
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
        status.code(),
    )
}

#[test]
fn thaw_js_package_eligible_accepts_a_native_addon_package_with_a_clean_bundle() {
    let pkg = ResolvedPackage {
        name: "tjs-kit".into(),
        commonjs_export_name: None,
        commonjs_export_assignment: None,
        called_commonjs_namespace_properties: Default::default(),
        functions: Vec::new(),
        values: Vec::new(),
        classes: Vec::new(),
        classifications: Vec::new(),
        native_lib: None,
        native_addon: Some(std::path::PathBuf::from("native.node")),
        native_dependencies: Vec::new(),
        platform_executable: None,
        bundle_js: None,
        factory_class_returns: Default::default(),
        namespace_self_aliases: Default::default(),
        type_only_exports: Default::default(),
        internal_support_classes: Default::default(),
        type_only_value_names: Default::default(),
        nested_namespaces: Default::default(),
    };
    let bundle = "module.exports = (function(require){ var __thaw_bundle_factories = { \"tjs-kit/index.js\": function(module, exports, require) { exports.add = function (a, b) { return a + b; }; } }; var __thaw_bundle_require_maps = { \"tjs-kit/index.js\": {} }; var __thaw_bundle_entry_key = \"tjs-kit/index.js\"; })(globalThis.require);";
    let user = "import { add } from 'tjs-kit';\nfunction main(): void { console.log(add(2, 3)); }\n";
    assert!(thaw_js_package_eligible(&pkg, bundle, user, &Default::default(), ""));
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
    assert!(thaw_js_async_declaration_plain(
        "declare function __thaw_typed_js_6d73(path: string): Promise<Uint8Array>;"
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
        "node:module",
        "node:path",
        "node:assert",
        "node:os",
        "node:util",
        "node:constants",
        "node:url",
        "node:fs",
        "node:vm",
        "node:async_hooks",
        "node:querystring",
        "node:timers",
        "node:worker_threads",
        "node:stream",
        "node:readline",
        "node:readline/promises",
        "node:dns",
        "node:dns/promises",
        "node:inspector",
        "node:inspector/promises",
        "node:util/types",
        "node:string_decoder",
        "node:tty",
        "node:dgram",
        "node:tls",
        "node:buffer",
        "node:process",
        "node:http",
        "node:child_process",
        "node:perf_hooks",
        "node:zlib",
        "node:net",
        "node:crypto",
        "node:https",
        "node:http2",
        "node:stream/consumers",
        "node:diagnostics_channel",
        "node:trace_events",
            "node:repl",
        "node:punycode",
        "node:domain",
        "node:stream/promises",
        "node:test",
    ] {
        assert!(THAW_JS_BUILTIN_MODULES.contains(&module), "{module}");
    }
    // modules not yet validated on thaw-js (e.g. node:cluster) stay on QuickJS
    assert!(!THAW_JS_BUILTIN_MODULES.contains(&"node:cluster"), "node:cluster");
}

#[test]
fn node_domain_test_stream_promises_bundles_scan_clean() {
    for module in ["node:domain", "node:test", "node:stream/promises"] {
        let bundle = thaw_registry::builtin_bundle_with(module, &[]).unwrap();
        let report = thaw_js::scan::scan_bundle(&bundle).unwrap();
        assert!(report.is_clean(), "{module}: {:?}", report.blockers);
        assert_eq!(report.entry, module);
    }
}

#[test]
fn node_batch1_builtin_bundles_scan_clean() {
    for module in [
        "node:stream/consumers",
        "node:diagnostics_channel",
        "node:trace_events",
            "node:repl",
        "node:punycode",
    ] {
        let bundle = thaw_registry::builtin_bundle_with(module, &[]).unwrap();
        let report = thaw_js::scan::scan_bundle(&bundle).unwrap();
        assert!(report.is_clean(), "{module}: {:?}", report.blockers);
        assert_eq!(report.entry, module);
    }
}

#[test]
fn node_module_thaw_js_bundle_scans_clean_with_path_inside() {
    // the registry bundle must be what thaw-js's install_module_loader parses (BUILTIN_NAMES_MARK)
    let bundle = thaw_registry::builtin_bundle_with("node:module", &["path"]).unwrap();
    assert!(bundle.contains("var __thaw_bundle_builtin_names = Object.freeze("));
    let report = thaw_js::scan::scan_bundle(&bundle).unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
    assert_eq!(report.entry, "node:module");
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

#[test]
fn thaw_js_blockers_are_recorded_once_and_reported_as_package_and_why() {
    clear_thaw_js_blockers();
    record_thaw_js_blocker("tjs-ctr", "`add`: declare function add(n: (x: number) => number): number;");
    record_thaw_js_blocker("tjs-ctr", "`add`: declare function add(n: (x: number) => number): number;");
    record_thaw_js_blocker("tjs-kit", "`apply`: declare function apply(f: (x: number) => number): number[];");
    assert_eq!(
        thaw_js_blockers_manifest(),
        serde_json::json!([
            { "package": "tjs-ctr", "why": "`add`: declare function add(n: (x: number) => number): number;" },
            { "package": "tjs-kit", "why": "`apply`: declare function apply(f: (x: number) => number): number[];" },
        ])
    );
    clear_thaw_js_blockers();
    assert_eq!(thaw_js_blockers_manifest(), serde_json::json!([]));
}

#[test]
fn thaw_js_user_dynamic_blocker_needs_routing_and_no_opt_in() {
    let dynamic = "function main(): void { const f: any = new Function('return 1'); console.log(f()); }";
    clear_thaw_js_blockers();
    record_thaw_js_user_dynamic_blocker(dynamic, false, false);
    record_thaw_js_user_dynamic_blocker(dynamic, true, true);
    record_thaw_js_user_dynamic_blocker("function main(): void { console.log(1); }", true, false);
    assert_eq!(thaw_js_blockers_manifest(), serde_json::json!([]));
    record_thaw_js_user_dynamic_blocker(dynamic, true, false);
    record_thaw_js_user_dynamic_blocker(dynamic, true, false);
    assert_eq!(
        thaw_js_blockers_manifest(),
        serde_json::json!([{
            "package": "<program>",
            "why": "program source uses dynamic code (new Function/eval): needs THAW_JS_USER_DYNAMIC=1",
        }])
    );
    clear_thaw_js_blockers();
}

#[test]
fn thaw_js_user_dynamic_is_on_unless_set_to_zero() {
    assert!(thaw_js_user_dynamic_from(None));
    assert!(thaw_js_user_dynamic_from(Some("1")));
    assert!(thaw_js_user_dynamic_from(Some("")));
    assert!(!thaw_js_user_dynamic_from(Some("0")));
}

#[test]
fn thaw_js_engine_env_selects_engine() {
    assert!(thaw_js_engine_from(Some("thaw-js"), None, false));
    assert!(!thaw_js_engine_from(Some("quickjs"), None, true));
    // THAW_JS_ENGINE wins over a conflicting THAW_JS_RUNTIME
    assert!(!thaw_js_engine_from(Some("quickjs"), Some("1"), false));
    assert!(thaw_js_engine_from(Some("thaw-js"), Some("0"), false));
}

#[test]
fn thaw_js_runtime_alias_still_works() {
    assert!(thaw_js_engine_from(None, Some("1"), false));
    assert!(!thaw_js_engine_from(None, Some("0"), true));
    assert!(!thaw_js_engine_from(None, Some("2"), false));
    assert!(thaw_js_engine_from(None, Some(""), true));
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn thaw_js_default_is_thaw_js() {
    assert!(THAW_JS_DEFAULT);
}

#[test]
fn thaw_js_engine_default_applies_when_unset() {
    assert_eq!(thaw_js_engine_from(None, None, THAW_JS_DEFAULT), THAW_JS_DEFAULT);
    // unknown THAW_JS_ENGINE values fall through to the RUNTIME alias, then the default
    assert!(thaw_js_engine_from(Some("bogus"), Some("1"), false));
    assert_eq!(thaw_js_engine_from(Some("bogus"), None, THAW_JS_DEFAULT), THAW_JS_DEFAULT);
}

#[test]
fn thaw_js_routed_fallback_is_recorded_under_the_build_key() {
    clear_thaw_js_blockers();
    record_thaw_js_fallback_blocker("thaw-js routing needs QuickJS");
    assert_eq!(
        thaw_js_blockers_manifest(),
        serde_json::json!([{
            "package": "<build>",
            "why": "routed build fell back to QuickJS: thaw-js routing needs QuickJS",
        }])
    );
    clear_thaw_js_blockers();
}

#[test]
fn thaw_js_fallback_build_keeps_quickjs_and_reports_the_blockers_key() {
    // Same pinned scenario as `thaw_js_routing_falls_back_when_a_class_instance_needs_live_property_access`:
    // the routed build is rebuilt on QuickJS, and the artifact still carries the blockers key.
    let (dir, registry, entry) = thaw_js_class_fixture(
        "blockers-fallback",
        "import { Counter } from 'tjs-ctr';\n\
         function main(): void {\n\
           const c = new Counter(5);\n\
           c.n = 40;\n\
           console.log(c.add(2), c.n);\n\
         }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], true, "{manifest}");
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!([]));
    assert!(manifest["thaw_js_blockers"].is_array(), "{manifest}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn node_perf_hooks_thaw_js_bundle_scans_clean() {
    let bundle = thaw_registry::builtin_bundle_with("node:perf_hooks", &[]).unwrap();
    let report = thaw_js::scan::scan_bundle(&bundle).unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
    assert_eq!(report.entry, "node:perf_hooks");
}

#[test]
fn random_fill_sync_declaration_keeps_bytes_only_when_thaw_js_routes_node_crypto() {
    // The QuickJS-lane override declares randomFillSync as JsValue; thaw-js keeps the Bytes signature.
    let dir = std::env::temp_dir().join(format!("thaw-cli-random-fill-decl-{}", std::process::id()));
    let registry = dir.join("modules");
    let packages = ["node:crypto".to_string()];
    let active = packages.iter().cloned().collect::<std::collections::HashSet<_>>();
    let source = "import { randomFillSync } from \"node:crypto\";\nfunction main(): void {\n    const value = Buffer.alloc(8);\n    randomFillSync(value, 2, 3);\n}\n";
    let declaration = |routed: &ThawJsPackages| {
        let (shims, _) = generate_registry_shims_pass(
            &registry, &packages, &active, &active, &std::collections::HashSet::new(), source,
            &thaw_parser::common::FileName::Custom("random fill.ts".into()),
            false, &dir.join("app"), Some(&dir.join("staging")),
            &std::collections::HashSet::new(), true, routed,
        ).unwrap();
        shims.0.lines()
            .find(|line| thaw_js_declared_function(line, "node:crypto").as_deref() == Some("randomFillSync"))
            .unwrap_or_else(|| panic!("no randomFillSync declaration"))
            .to_string()
    };
    let unrouted = declaration(&ThawJsPackages::new());
    assert!(unrouted.contains("JsValue") && !unrouted.contains("Uint8Array"), "{unrouted}");
    let routed = declaration(&std::iter::once("node:crypto".to_string()).collect());
    assert!(routed.contains("Uint8Array") && !routed.contains("JsValue"), "{routed}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn imported_buffer_stays_native_when_thaw_js_routes_node_buffer() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-imported-buffer-{}", std::process::id()));
    let registry = dir.join("modules");
    let packages = ["node:buffer".to_string(), "node:crypto".to_string()];
    let active = packages.iter().cloned().collect::<std::collections::HashSet<_>>();
    let source = "import { Buffer } from \"node:buffer\";\nimport { randomFillSync } from \"node:crypto\";\nfunction main(): void {\n    const value = Buffer.alloc(8);\n    randomFillSync(value, 2, 3);\n}\n";
    let shim_text = |routed: &ThawJsPackages| {
        let (shims, _) = generate_registry_shims_pass(
            &registry, &packages, &active, &active, &std::collections::HashSet::new(), source,
            &thaw_parser::common::FileName::Custom("imported buffer.ts".into()),
            false, &dir.join("app"), Some(&dir.join("staging")),
            &std::collections::HashSet::new(), true, routed,
        ).unwrap();
        shims.0
    };
    let routed = shim_text(&packages.iter().cloned().collect());
    assert!(!routed.contains("__thaw_type_node_buffer_Buffer"), "{routed}");
    let unrouted = shim_text(&ThawJsPackages::new());
    assert!(unrouted.contains("__thaw_type_node_buffer_Buffer"), "{unrouted}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn node_http_create_server_is_native_fast_path_in_the_real_declarations() {
    // Premise gate: the real builtin d.ts (not a one-line excerpt) must classify createServer as a native call.
    let _guard = thaw_bridge::allow_native_bytes_type(true);
    let dts = thaw_registry::resolve_builtin("node:http").unwrap().dts_source;
    let functions = thaw_bridge::parse_dts(&dts).unwrap();
    let classes = thaw_bridge::effective_classifications(&functions, true);
    let (_, class) = classes
        .iter()
        .find(|(name, _)| name == "createServer")
        .expect("createServer is declared");
    assert!(
        matches!(class, thaw_bridge::Classification::FastPath(_)),
        "{class:?}"
    );
}

#[test]
fn thaw_js_routes_node_http_when_the_program_calls_native_create_server() {
    // Multi-line source: createServer is a native FastPath function with no typed target.
    let dir = std::env::temp_dir().join(format!("thaw-cli-http-eligible-{}", std::process::id()));
    let registry = dir.join("modules");
    let packages = ["node:http".to_string()];
    let active = packages.iter().cloned().collect::<std::collections::HashSet<_>>();
    let source = "import * as http from \"node:http\";\nfunction main(): void {\n    const s = http.createServer((req, res) => { res.end(\"x\"); });\n    s.close();\n}\n";
    let (_, eligible) = generate_registry_shims_pass(
        &registry, &packages, &active, &active, &std::collections::HashSet::new(), source,
        &thaw_parser::common::FileName::Custom("http eligible.ts".into()),
        false, &dir.join("app"), Some(&dir.join("staging")),
        &std::collections::HashSet::new(), true, &ThawJsPackages::new(),
    ).unwrap();
    assert!(eligible.contains("node:http"), "{eligible:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn routed_node_http_keeps_the_native_create_server_declaration() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-http-decl-{}", std::process::id()));
    let registry = dir.join("modules");
    let packages = ["node:http".to_string()];
    let active = packages.iter().cloned().collect::<std::collections::HashSet<_>>();
    let source = "import * as http from \"node:http\";\nfunction main(): void {\n    const s = http.createServer((req, res) => { res.end(\"x\"); });\n    s.close();\n}\n";
    let (shims, _) = generate_registry_shims_pass(
        &registry, &packages, &active, &active, &std::collections::HashSet::new(), source,
        &thaw_parser::common::FileName::Custom("http decl.ts".into()),
        false, &dir.join("app"), Some(&dir.join("staging")),
        &std::collections::HashSet::new(), true, &packages.iter().cloned().collect(),
    ).unwrap();
    let shim = shims.0;
    assert!(shim.contains("declare function createServer("), "{shim}");
    assert!(!shim.contains("callDynamic(\"createServer\""), "{shim}");
    assert!(!shim.contains("function get(argsArray"), "{shim}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routed_node_http_server_only_links_no_quickjs() {
    // Server-only: no thaw-js http client in the program. The listen callback reports the bound port and closes.
    let (dir, registry, entry) = thaw_js_fixture(
        "node-http-create-server",
        "import * as http from 'node:http';\n\
         function main(): void {\n\
           const srv = http.createServer((req, res) => { res.statusCode = 201; res.end('body'); });\n\
           srv.listen(0, '127.0.0.1', () => {\n\
             console.log('listening', typeof (srv.address() as any).port);\n\
             srv.close();\n\
           });\n\
         }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], true, "{manifest}");
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "listening number\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routed_node_http_get_refused_without_a_server() {
    let (dir, registry, entry) = thaw_js_fixture(
        "node-http-get-refused",
        "import * as http from 'node:http';\n\
         function main(): void {\n\
           const r = http.get('http://127.0.0.1:9/', () => {});\n\
           r.on('error', (e: any) => console.log('err', e.code));\n\
         }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "err ECONNREFUSED\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routed_node_http_fetch_from_listen_callback_refused() {
    let (dir, registry, entry) = thaw_js_fixture(
        "node-http-fetch-from-listen-callback",
        "import * as http from 'node:http';\n\
         function main(): void {\n\
           const srv = http.createServer((req, res) => { res.end('x'); });\n\
           srv.listen(0, '127.0.0.1', async () => {\n\
             try { await fetch('http://127.0.0.1:9/'); } catch (e: any) { console.log('err'); }\n\
             srv.close();\n\
           });\n\
         }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    // Narrow blocker: a native createServer next to the fetch client keeps node:http on QuickJS.
    assert_eq!(manifest["quickjs"], true, "{manifest}");
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "err\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routed_node_http_get_from_listen_callback_refused() {
    let (dir, registry, entry) = thaw_js_fixture(
        "node-http-get-from-listen-callback",
        "import * as http from 'node:http';\n\
         function main(): void {\n\
           const srv = http.createServer((req, res) => { res.end('x'); });\n\
           srv.listen(0, '127.0.0.1', () => {\n\
             const r = http.get('http://127.0.0.1:9/', () => {});\n\
             r.on('error', (e: any) => { console.log('err', e.code); srv.close(); });\n\
           });\n\
         }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "err ECONNREFUSED\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routed_node_http_get_from_listen_callback_does_not_segv() {
    // Rust-built Server methods are called through the graph adapter; a null this-entry must fall back to
    // the plain entry instead of jumping to a heap address (was SIGSEGV, exit 139).
    let (dir, registry, entry) = thaw_js_fixture(
        "node-http-get-from-listen-callback-v1",
        "import * as http from 'node:http';\n\
         function main(): void {\n\
           const srv = http.createServer((req, res) => { res.statusCode = 201; res.end('body'); });\n\
           srv.listen(0, '127.0.0.1', () => {\n\
             const addr: any = srv.address();\n\
             http.get('http://127.0.0.1:' + addr.port + '/', (res: any) => {\n\
               console.log('R', res.statusCode);\n\
               srv.close();\n\
             });\n\
           });\n\
         }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "R 201\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_ctor_option_callback_declaration_is_plain_only_for_constructors() {
    let ctor = "declare function __thaw_typed_tjs_246e6577246e6f64653a73747265616d3a3a5772697461626c6524617269747931(opts: { write: ((arg0: JsValue, arg1: string, arg2: () => void) => void) | undefined }): JsValue;";
    let method = "declare function __thaw_typed_tjs_246d6574686f64246e6f64653a73747265616d3a3a5772697461626c6524777269746524617269747931(opts: { write: ((arg0: JsValue, arg1: string, arg2: () => void) => void) | undefined }): JsValue;";
    // a constructor's option object may carry callbacks (`new Writable({ write })`)
    assert!(thaw_js_declaration_plain_with(ctor, true, true));
    // the same signature is still rejected by default, and for a method operation
    assert!(!thaw_js_declaration_plain_with(ctor, true, false));
    assert!(!thaw_js_declaration_plain(ctor));
    assert!(!thaw_js_declaration_plain_with(method, false, false));
}

#[test]
fn thaw_js_unready_addon_blocker_keeps_live_owner_and_subscription_addons_on_quickjs() {
    assert!(thaw_js_unready_addon_blocker("argon2").is_some());
    assert!(thaw_js_unready_addon_blocker("@parcel/watcher").is_some());
    assert!(thaw_js_unready_addon_blocker("bcrypt").is_none());
    assert!(thaw_js_unready_addon_blocker("sqlite3").is_none());
    assert!(thaw_js_unready_addon_blocker("some-unrelated-package").is_none());
}

/// node-compat `queueMicrotask uncaught exception` and `unhandled promise rejection` (sources from
/// tests/node-compat.json; the manifest has no expected-output field, so the expected text is Node's
/// listener output): native reports must reach the `process` listeners registered in thaw-js.
#[test]
fn routed_uncaught_and_rejection_reports_reach_thaw_js_process_listeners() {
    let cases = [
        (
            "uncaught",
            "async function main(): Promise<void> { process.once('uncaughtException', (error) => { console.log(error.message); }); queueMicrotask(() => { throw new Error('microtask boom'); }); await Promise.resolve(); }",
            "microtask boom\n",
        ),
        (
            "rejection",
            "async function main(): Promise<void> { process.once('unhandledRejection', (reason) => console.log(reason.message)); Promise.reject(new Error('reject boom')); await Promise.resolve(); }",
            "reject boom\n",
        ),
    ];
    for (name, body, expected) in cases {
        let _env = RouteAllEnv::set();
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-report-{name}"), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry, true);
        assert_eq!(thaw_js_run_guarded(&output), expected, "{name}");
    }
}

/// `typeof` of absent globals and statics read off imported classes must reach the live values, not an
/// `invalid JavaScript handle` error or a fresh native wrapper (r5b). Added unrun: no thaw-cli e2e here.
#[test]
fn routed_typeof_absent_global_and_imported_class_static_reads() {
    let body = "import EventEmitter from 'node:events';\nfunction main(): void { console.log(typeof navigator, typeof NopeNope, typeof Math); console.log(typeof EventEmitter.captureRejections, EventEmitter.defaultMaxListeners); }";
    let _env = RouteAllEnv::set();
    let (_dir, registry, entry) = thaw_js_fixture("route-typeof-static", body);
    let output = entry.with_file_name("app");
    thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(
        thaw_js_run_guarded(&output),
        "undefined undefined object\nboolean 10\n"
    );
}

/// Q9 route-all round 3 W4: node-compat `process hrtime bigint ids release and kill` (source from
/// tests/node-compat.json; expected text is Node's output). The `process.env` write case stays open: routed
/// `process.env.NAME` is a libc getenv read (thaw-llvm operators.rs compile_env_var), so JS writes are not seen.
#[test]
fn routed_process_ids_and_kill_match_node() {
    let cases = [
        (
            "hrtime bigint ids release and kill",
            "function main(): void {\n  const b = process.hrtime.bigint();\n  console.log(typeof b, b > 0n, process.hrtime.bigint() >= b);\n  console.log(typeof process.uptime(), typeof process.memoryUsage.rss(), typeof process.getuid(), typeof process.getgid(), typeof process.umask());\n  console.log(process.release.name, process.kill(process.pid, 0));\n  process.on(\"SIGUSR2\", () => { console.log(\"got sig\"); });\n  process.kill(process.pid, \"SIGUSR2\");\n  setTimeout(() => console.log(\"end\"), 20);\n}\n",
            "bigint true true\nnumber number number number number\nnode true\ngot sig\nend\n",
        ),
    ];
    for (name, body, expected) in cases {
        let _env = RouteAllEnv::set();
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-w4-{}", name.len()), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry, true);
        assert_eq!(thaw_js_run_guarded(&output), expected, "{name}");
    }
}

/// Turns route-all on for the calling thread only (the same effect as `THAW_JS_ROUTE_ALL=1`, without
/// touching the process env that parallel tests share). Off again on drop, even when the test panics.
struct RouteAllEnv;

impl RouteAllEnv {
    fn set() -> Self {
        set_thaw_js_route_all_override(true);
        RouteAllEnv
    }
}

impl Drop for RouteAllEnv {
    fn drop(&mut self) {
        set_thaw_js_route_all_override(false);
    }
}

#[test]
fn thaw_js_route_all_is_opt_in_on_exactly_one() {
    assert!(thaw_js_route_all_from(Some("1")));
    assert!(!thaw_js_route_all_from(None));
    assert!(!thaw_js_route_all_from(Some("0")));
    assert!(!thaw_js_route_all_from(Some("")));
    assert!(!thaw_js_route_all_from(Some("true")));
}

#[test]
fn thaw_js_route_all_override_is_thread_local() {
    if std::env::var("THAW_JS_ROUTE_ALL").is_ok() {
        return; // the isolation check needs the env unset by the outer shell
    }
    let _env = RouteAllEnv::set();
    assert!(thaw_js_route_all_enabled());
    let other_thread = std::thread::spawn(thaw_js_route_all_enabled).join().unwrap();
    assert!(!other_thread, "a route-all override must not leak to another thread");
}

#[test]
fn thaw_js_route_all_attempts_thaw_js_for_a_plain_any_program_and_names_the_fallback() {
    // Without THAW_JS_ROUTE_ALL this program has no routed package and no dynamic-code marker, so it
    // is never attempted on thaw-js. With it, the route is attempted, and a build that still needs
    // QuickJS says which lowering demanded it in the manifest's blockers.
    let dir = std::env::temp_dir().join(format!("thaw-cli-route-all-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let registry = dir.join("modules");
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "function main(): void {\n  const data: any = JSON.parse('{\"x\": 1}');\n  console.log(data.x);\n}\n",
    )
    .unwrap();
    let output = dir.join("app");
    let manifest = {
        let _env = RouteAllEnv::set();
        thaw_js_build(&entry, &output, &registry, true)
    };
    eprintln!("route-all manifest: {manifest}");
    if manifest["quickjs"] == true {
        let blockers = manifest["thaw_js_blockers"].to_string();
        assert!(
            blockers.contains("THAW_JS_MIXED")
                && (blockers.contains("site:") || blockers.contains("unported:thaw_js_")),
            "a QuickJS fallback must name its sites or symbols: {manifest}"
        );
    } else {
        assert_eq!(thaw_js_run_guarded(&output).trim(), "1");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// node-compat tick, timer and lifecycle cases (sources from tests/node-compat.json; the manifest has no
/// expected-output field, so each expected text is derived from Node's semantics for that source).
#[test]
fn routed_tick_timer_and_lifecycle_order_matches_node() {
    let cases: [(&str, &str, &str, Option<i32>); 6] = [
        (
            "nextTick precedes promise and microtask queues",
            "async function main(): Promise<void> { const order: string[] = []; process.nextTick(() => order.push('tick')); Promise.resolve().then(() => order.push('promise')); queueMicrotask(() => order.push('microtask')); await new Promise<void>((resolve) => setImmediate(resolve)); console.log(order.join(',')); }",
            "tick,promise,microtask\n",
            Some(0),
        ),
        (
            "commonjs program runs nextTick before promises",
            "function main(): void { console.log('sync'); process.nextTick((): void => console.log('tick')); Promise.resolve().then((): void => console.log('promise')); queueMicrotask((): void => console.log('microtask')); }",
            "sync\ntick\npromise\nmicrotask\n",
            Some(0),
        ),
        (
            "timers: each due timer gets a microtask checkpoint",
            "async function main(): Promise<void> {\n    const events: string[] = [];\n    setTimeout(() => {\n        events.push(\"timer-1\");\n        Promise.resolve().then(() => {\n            events.push(\"promise\");\n        });\n    }, 0);\n    setTimeout(() => {\n        events.push(\"timer-2\");\n    }, 0);\n    await new Promise<void>((resolve) => setTimeout(resolve, 20));\n    console.log(events.join(\",\"));\n}\n",
            "timer-1,promise,timer-2\n",
            Some(0),
        ),
        (
            "process beforeExit re-arm and exit event",
            "function main(): void { let n: number = 0; process.on('beforeExit', (code: number): void => { console.log('beforeExit', code, n); if (n++ < 1) setTimeout((): void => console.log('more'), 1); }); process.on('exit', (code: number): void => console.log('exit', code)); console.log('main'); }",
            "main\nbeforeExit 0 0\nmore\nbeforeExit 0 1\nexit 0\n",
            Some(0),
        ),
        (
            "sync throw in main skips registered timers and exit sees code 1",
            "function main(): void { process.on('exit', (code: number) => console.log('exit', code)); setTimeout(() => console.log('timer'), 5); console.log('a'); throw new Error('boom'); }",
            "a\nexit 1\n",
            Some(1),
        ),
        (
            "uncaught exception monitor precedes handler",
            "function main(): void { const order: string[] = []; process.on('uncaughtExceptionMonitor', () => order.push('monitor')); process.on('uncaughtException', () => { order.push('handler'); console.log(order.join(',')); }); queueMicrotask(() => { throw new Error('boom'); }); }",
            "monitor,handler\n",
            Some(0),
        ),
    ];
    for (name, body, expected, code) in cases {
        let _env = RouteAllEnv::set();
        let slug = name.replace(|c: char| !c.is_ascii_alphanumeric(), "-");
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-tick-{slug}"), body);
        let output = entry.with_file_name("app");
        let manifest = thaw_js_build(&entry, &output, &registry, true);
        assert_eq!(manifest["quickjs"], false, "{name} linked QuickJS: {manifest}");
        let (stdout, stderr, status) = thaw_js_run_guarded_status(&output);
        assert_eq!((stdout.as_str(), status), (expected, code), "{name}: {stderr}");
    }
}

/// node-compat `filesystem read and write streams`, `audit2e fs readdir ...` and `nd8 readline/promises ...` with
/// default routing (no THAW_JS_ROUTE_ALL): each program must exit on its own within the 20 s guard.
#[test]
fn routed_fs_and_readline_programs_exit_on_their_own() {
    let cases = [
        (
            "fs-streams",
            "import { createReadStream, createWriteStream, rmSync } from 'node:fs'; async function main(): Promise<void> { const path = '/tmp/thaw-node-fs-stream.txt'; rmSync(path, { force: true }); await new Promise<void>((resolve, reject) => { const output = createWriteStream(path, { highWaterMark: 2 }); output.on('error', reject); output.end('streamed', resolve); }); const chunks: string[] = []; await new Promise<void>((resolve, reject) => { const input = createReadStream(path, { highWaterMark: 3, encoding: 'utf8' }); input.on('data', (chunk: string): void => chunks.push(chunk)); input.on('end', resolve); input.on('error', reject); }); console.log(chunks.join(''), chunks.length); rmSync(path); }",
            "streamed 3\n",
        ),
        (
            "readline-promises",
            "import * as readline from \"node:readline\";\nimport { Readable } from \"node:stream\";\nasync function main(): Promise<void> {\n  const rl2 = readline.createInterface({ input: Readable.from([\"a\\nb\\n\"]) });\n  rl2.on(\"line\", (l: string) => console.log(\"line\", l));\n  await new Promise((r) => rl2.on(\"close\", r));\n  const rlp = await import(\"node:readline/promises\");\n  const rl3 = rlp.createInterface({ input: Readable.from([\"answer\\n\"]) });\n  console.log(await rl3.question(\"q? \"));\n  rl3.close();\n}\n",
            "line a\nline b\nanswer\n",
        ),
    ];
    for (name, body, expected) in cases {
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-exit-{name}"), body);
        let output = entry.with_file_name("app");
        let manifest = thaw_js_build(&entry, &output, &registry, true);
        eprintln!("{name} manifest: {manifest}");
        let (stdout, stderr, status) = thaw_js_run_guarded_status(&output);
        assert_eq!((stdout.as_str(), status), (expected, Some(0)), "{name}: {stderr}");
    }
}

/// node-compat `audit2e fs readdir ...` with default routing: exits on its own and prints its seven lines.
#[test]
fn routed_audit2e_fs_program_exits_on_its_own() {
    let body = r##"import * as fs from "node:fs"; import * as os from "node:os"; async function main(): Promise<void> { const d = fs.mkdtempSync(os.tmpdir() + "/fixe-"); for (const n of ["a", "B", "10", "9", "é"]) fs.writeFileSync(d + "/" + n, "1"); console.log(fs.readdirSync(d)); fs.writeFileSync(d + "/b.bin", Buffer.from([1, 2, 3, 4])); console.log(fs.readFileSync(d + "/b.bin", "hex"), fs.readFileSync(d + "/b.bin", { encoding: "base64" }), fs.statSync(d + "/zzz", { throwIfNoEntry: false })); console.log(fs.constants.R_OK, fs.F_OK, fs.R_OK, fs.W_OK, fs.X_OK); await fs.promises.writeFile(d + "/p.txt", "promised"); console.log(await fs.promises.readFile(d + "/p.txt", "utf8"), (await fs.promises.stat(d + "/p.txt")).size); await new Promise<void>((res) => fs.stat(d + "/p.txt", (err, st) => { console.log(err, st.size); res(); })); await new Promise<void>((res) => fs.readdir(d, (err, names) => { console.log(err, names.length); res(); })); await new Promise<void>((res) => fs.exists(d + "/p.txt", (ex) => { console.log(ex); res(); })); fs.accessSync(d + "/p.txt"); fs.rmSync(d, { recursive: true }); }"##;
    let (_dir, registry, entry) = thaw_js_fixture("route-exit-audit2e", body);
    let output = entry.with_file_name("app");
    thaw_js_build(&entry, &output, &registry, true);
    let (stdout, stderr, status) = thaw_js_run_guarded_status(&output);
    assert_eq!((stdout.lines().count(), status), (7, Some(0)), "{stderr}");
}

/// node-compat `process nextTick ordering` and `es module runs promises before nextTick` (ES module entries:
/// their promise jobs run before the module's ticks, as in Node), routed through thaw-js with RouteAllEnv.
#[test]
fn routed_esm_next_tick_orders_match_node() {
    let cases = [
        (
            "process nextTick ordering",
            r##"import process from 'node:process'; function main(): void { console.log('sync'); process.nextTick((): void => console.log('tick')); Promise.resolve().then((): void => console.log('promise')); }"##,
            "sync\npromise\ntick\n",
        ),
        (
            "es module runs promises before nextTick",
            r##"import { setImmediate as wait } from 'node:timers'; async function main(): Promise<void> { const order: string[] = []; process.nextTick(() => order.push('tick')); Promise.resolve().then(() => order.push('promise')); queueMicrotask(() => order.push('microtask')); await new Promise<void>((resolve) => wait(resolve)); console.log(order.join(',')); }"##,
            "promise,microtask,tick\n",
        ),
    ];
    for (name, body, expected) in cases {
        let _env = RouteAllEnv::set();
        let slug = name.replace(|c: char| !c.is_ascii_alphanumeric(), "-");
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-esm-{slug}"), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry, true);
        let (stdout, stderr, status) = thaw_js_run_guarded_status(&output);
        assert_eq!((stdout.as_str(), status), (expected, Some(0)), "{name}: {stderr}");
    }
}

/// Q9 cluster C1: the node-compat/builtins/ts-runtime case sources for Iterator, iterator helpers and
/// JSON.rawJSON, routed through thaw-js with RouteAllEnv. The manifests carry no expected output, so the
/// expected text is what Node 22 prints for the same program (checked with `node`).
#[test]
fn routed_iterator_and_raw_json_cases_match_node() {
    let cases = [
        (
            "Iterator.from",
            "function main(): void { const it = Iterator.from([1,2,3]); console.log(it.next().value, it.next().value); }",
            "1 2\n",
        ),
        (
            "iterator helpers",
            "function main(): void { const selected = Iterator.from([1, 2, 3, 4]).map(value => value * 2).filter(value => value > 2).drop(1).take(2).toArray(); const total = Iterator.from([1, 2, 3]).reduce((sum: number, value: number) => sum + value, 0); console.log(selected.join(','), total, Iterator.from(new Set<number>([3, 4])).toArray().join(',')); }",
            "6,8 6 3,4\n",
        ),
        (
            "JSON.rawJSON",
            "function main(): void { console.log(JSON.stringify({n: JSON.rawJSON(\"12345678901234567890\")})); }",
            "{\"n\":12345678901234567890}\n",
        ),
    ];
    for (name, body, expected) in cases {
        let _env = RouteAllEnv::set();
        let slug = name.replace(|c: char| !c.is_ascii_alphanumeric(), "-");
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-c1-{slug}"), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry, true);
        let (stdout, stderr, status) = thaw_js_run_guarded_status(&output);
        assert_eq!((stdout.as_str(), status), (expected, Some(0)), "{name}: {stderr}");
    }
}


/// Q9 cluster B1: a native Error caught as `any` or collected in an AggregateError, in a program that loads
/// no package, goes through the error factory, which must work with an empty package table. The sources are
/// tests/node-compat.json; the manifests carry no expected-output field, so the expected text is Node's output.
#[test]
fn routed_error_factory_runs_in_a_program_with_no_package() {
    let cases = [
        (
            "typinggaps catch any property assign",
            "function main(): void {\n  try { throw new TypeError(\"boom\"); } catch (e: any) { e.k = 2; e.code = \"E1\"; console.log(e.k, e.code, e.message, e.name); }\n  try { throw { a: 1 }; } catch (e: any) { e.k = 2; console.log(e.k, e.a); }\n  try { throw new Error(\"rethrow\"); } catch (e: any) { e.k = 1; try { throw e; } catch (e2: any) { console.log(e2.k, e2.message); } }\n}\n",
            "2 E1 boom TypeError\n2 1\n1 rethrow\n",
        ),
        (
            "caught Error keeps pointer identity under strict equality",
            "function main(): void {\n  const a = new Error(\"x\");\n  const b = new Error(\"x\");\n  try { throw a; } catch (e: any) { console.log(e === a, e === b, e !== a); }\n  try { throw a; } catch (e) { console.log(e === a, e === b); }\n}\n",
            "true false false\ntrue false\n",
        ),
        (
            "Promise.any rejection exposes the errors of every input",
            "async function f(n: number): Promise<number> { await Promise.resolve(); throw new Error('f' + n); }\nasync function main(): Promise<void> {\n  try {\n    await Promise.any([f(1), Promise.reject('b'), Promise.reject(7), Promise.reject(null)]);\n  } catch (e: any) {\n    console.log(e.name, e.message, e instanceof AggregateError);\n    console.log(e.errors.length, e.errors[0].message, e.errors[1], e.errors[2], e.errors[3]);\n  }\n  const v = await Promise.any([Promise.reject(new Error('z')), Promise.resolve(5)]);\n  console.log(v);\n  try { await Promise.any([]); } catch (e: any) { console.log(e.name, e.errors.length); }\n}",
            "AggregateError All promises were rejected true\n4 f1 b 7 null\n5\nAggregateError 0\n",
        ),
    ];
    for (name, body, expected) in cases {
        let _env = RouteAllEnv::set();
        let (_dir, registry, entry) = thaw_js_fixture(&format!("error-factory-{}", name.len()), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry, true);
        assert_eq!(thaw_js_run_guarded(&output), expected, "{name}");
    }
}


/// builtins / node-compat / typescript-runtime / text-encoding SharedArrayBuffer and Atomics cases
/// (sources from tests/*.json; the manifests have no expected-output field, so each expected text is
/// derived from Node 22 semantics for that source). Cross-worker SAB sharing is not covered here.
#[test]
fn routed_shared_array_buffer_and_atomics_match_node() {
    let cases: [(&str, &str, &str); 9] = [
        (
            "sab-byte-length",
            "function main(): void { const b: SharedArrayBuffer = new SharedArrayBuffer(16); console.log(b.byteLength, b.slice(4).byteLength); }",
            "16 12\n",
        ),
        (
            "atomics-add",
            "function main(): void { const a: Int32Array = new Int32Array(new SharedArrayBuffer(4)); console.log(Atomics.add(a,0,1)); }",
            "0\n",
        ),
        (
            "atomics-compare-exchange",
            "function main(): void { const a: Int32Array = new Int32Array(new SharedArrayBuffer(4)); Atomics.store(a,0,5); console.log(Atomics.compareExchange(a,0,5,9), Atomics.load(a,0)); }",
            "5 9\n",
        ),
        (
            "atomics-exchange",
            "function main(): void { const a: Int32Array = new Int32Array(new SharedArrayBuffer(4)); Atomics.store(a,0,5); console.log(Atomics.exchange(a,0,7), Atomics.load(a,0)); }",
            "5 7\n",
        ),
        (
            "atomics-load-store",
            "function main(): void { const a: Int32Array = new Int32Array(new SharedArrayBuffer(8)); console.log(Atomics.store(a,0,42), Atomics.load(a,0)); }",
            "42 42\n",
        ),
        (
            "atomics-sub",
            "function main(): void { const a: Int32Array = new Int32Array(new SharedArrayBuffer(4)); Atomics.store(a,0,10); console.log(Atomics.sub(a,0,3), Atomics.load(a,0)); }",
            "10 7\n",
        ),
        (
            "shared-array-buffer-atomics",
            "function main(): void { const buffer = new SharedArrayBuffer(8); const values = new Int32Array(buffer); Atomics.store(values, 0, 20); console.log(Atomics.add(values, 0, 22), Atomics.load(values, 0), buffer.byteLength); }",
            "20 42 8\n",
        ),
        (
            "atomics-wait-main-thread",
            "function main(): void {\n  const ia = new Int32Array(new SharedArrayBuffer(8));\n  console.log(Atomics.wait(ia, 0, 1, 0));\n  console.log(Atomics.wait(ia, 0, 0, 0));\n  console.log(Atomics.wait(ia, 0, 0, 5));\n}",
            "not-equal\ntimed-out\ntimed-out\n",
        ),
        (
            "text-decode-real-buffer-sources",
            "function main(): void { const bytes = new Uint8Array([65, 66, 67, 68]); const buffer = bytes.buffer; const view = new DataView(buffer, 1, 2); const shared = new SharedArrayBuffer(2); new Uint8Array(shared).set([88, 89]); let invalid = ''; try { new TextDecoder().decode({ buffer: buffer, byteOffset: 0, byteLength: 4 } as any); } catch (error: any) { invalid = error.name; } console.log(JSON.stringify([new TextDecoder().decode(buffer), new TextDecoder().decode(view), new TextDecoder().decode(shared), invalid])); }",
            "[\"ABCD\",\"BC\",\"XY\",\"TypeError\"]\n",
        ),
    ];
    for (name, body, expected) in cases {
        let _env = RouteAllEnv::set();
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-sab-{name}"), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry, true);
        let (stdout, stderr, status) = thaw_js_run_guarded_status(&output);
        assert_eq!((stdout.as_str(), status), (expected, Some(0)), "{name}: {stderr}");
    }
}
