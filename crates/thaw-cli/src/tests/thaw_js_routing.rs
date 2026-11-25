// Experimental thaw-js routing (thaw-js). These call `build_with_thaw_js_routing`
// directly so the tests never touch the process environment.

#[test]
fn thaw_js_declaration_bytes_result_is_plain_for_any_program() {
    let bytes = "declare function __thaw_typed_tjs_6d73(path: string): Uint8Array;";
    assert!(thaw_js_declaration_plain_with(bytes, false));
    assert!(thaw_js_declaration_plain(bytes));
    // a Uint8Array result under a Promise is plain too
    let async_bytes = "declare function __thaw_typed_tjs_6d73(path: string): Promise<Uint8Array>;";
    assert!(thaw_js_async_declaration_plain_with(async_bytes));
    // the relaxation only covers the bare bytes result, never a rejected parameter
    let buffer_param = "declare function __thaw_typed_tjs_6d73(v: Buffer): Uint8Array;";
    assert!(!thaw_js_declaration_plain_with(buffer_param, false));
}

#[test]
fn thaw_js_tls_fs_https_bytes_result_is_plain_for_any_program() {
    // the r2c shape (node:tls + node:fs + node:child_process) and the ky shape (node:https) get the same line
    let bytes = "declare function __thaw_typed_tjs_6d73(path: string): Uint8Array;";
    assert!(thaw_js_declaration_plain_with(bytes, false));
    let async_bytes = "declare function __thaw_typed_tjs_6d73(path: string): Promise<Uint8Array>;";
    assert!(thaw_js_async_declaration_plain_with(async_bytes));
}

#[test]
fn thaw_js_declaration_scan_accepts_plain_data_and_rejects_everything_else() {
    let plain = |line: &str| thaw_js_declaration_plain(line);
    assert!(plain(
        "declare function __thaw_typed_tjs_6d73(value: number, options: { long: boolean }): string;"
    ));
    assert!(plain(
        "declare function __thaw_typed_tjs_6d73__arity_1(args: string[]): JsValue;"
    ));
    assert!(plain(
        "declare function __thaw_typed_tjs_6d73(a: string | number, b?: Json): null | undefined;"
    ));
    // type parameters declared by the function itself are fine (inferred from plain arguments)
    assert!(plain(
        "declare function __thaw_typed_tjs_6d73__generic_rest__arity_1<T>(array: T[]): T[];"
    ));
    assert!(plain("declare function __thaw_typed_tjs_6d73<TObject, TPath extends string>(data: TObject, path: TPath): JsValue;"));
    // callbacks, classes and other named types cannot cross the JSON boundary
    assert!(!plain(
        "declare function __thaw_typed_tjs_6d73(f: (x: number) => number): number;"
    ));
    assert!(!plain(
        "declare function __thaw_typed_tjs_6d73(v: Buffer): number;"
    ));
    assert!(!plain(
        "declare function __thaw_typed_tjs_6d73(v: SemVer): number;"
    ));
    assert!(!plain(
        "declare function __thaw_typed_tjs_6d73(): Promise<number>;"
    ));
    assert!(!plain("function __thaw_typed_wrapper_tjs_6d73(): number {"));
    // a bare `Uint8Array` result crosses as bytes; in params, unions and nested positions it stays out
    assert!(plain(
        "declare function __thaw_typed_tjs_6d73(path: string): Uint8Array;"
    ));
    assert!(!plain(
        "declare function __thaw_typed_tjs_6d73(v: Uint8Array): number;"
    ));
    assert!(!plain(
        "declare function __thaw_typed_tjs_6d73(): Uint8Array | string;"
    ));
}

#[test]
fn thaw_js_declaration_callable_result_is_plain_and_callable_parameter_is_not() {
    // a callable RESULT is a live handle (ejs.compile, nanoid.customAlphabet): plain when its
    // parameters and return are plain, including a union of such callables (sync | Promise return)
    let compile = "declare function __thaw_typed_tjs_6d73(template: string): ((arg0?: Json | undefined) => string) | ((arg0?: Json | undefined) => Promise<string>);";
    assert!(thaw_js_declaration_plain(compile));
    let factory = "declare function __thaw_typed_tjs_6d73(prefix: string): (x: string) => string;";
    assert!(thaw_js_declaration_plain(factory));
    // a callable PARAMETER (shape S1) stays rejected, also next to a callable result
    assert!(!thaw_js_declaration_plain(
        "declare function __thaw_typed_tjs_6d73(f: (x: number) => number): number;"
    ));
    assert!(!thaw_js_declaration_plain(
        "declare function __thaw_typed_tjs_6d73(f: (x: number) => number): (y: string) => string;"
    ));
    // a callable result whose parameter or return is not plain data stays rejected
    assert!(!thaw_js_declaration_plain(
        "declare function __thaw_typed_tjs_6d73(prefix: string): (x: string) => Buffer;"
    ));
    assert!(!thaw_js_declaration_plain(
        "declare function __thaw_typed_tjs_6d73(prefix: string): (x: Buffer) => string;"
    ));
    assert!(!thaw_js_declaration_plain(
        "declare function __thaw_typed_tjs_6d73(prefix: string): ((x: string) => string) | number;"
    ));
}

#[test]
fn thaw_js_symbols_decode_to_their_package_function() {
    let line = "declare function __thaw_typed_tjs_6d733a3a6d73__overload_0__arity_2(value: number): string;";
    assert_eq!(thaw_js_declared_function(line, "ms").as_deref(), Some("ms"));
    assert_eq!(thaw_js_declared_function(line, "other"), None);
    let generic = "declare function __thaw_typed_tjs_6c6f646173683a3a676574__generic_rest__arity_1<T>(a: T[]): T[];";
    assert_eq!(
        thaw_js_declared_function(generic, "lodash").as_deref(),
        Some("get")
    );
    assert_eq!(
        thaw_js_declared_function("declare function __thaw_typed_tjs_zz(): number;", "ms"),
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
/// thaw-js path is the baseline and thaw-js is the replacement.
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
    // the evaluation tail that makes the same text runnable as a plain thaw-js script.
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

fn thaw_js_build(entry: &Path, output: &Path, registry: &Path) -> serde_json::Value {
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

fn builtin_routing_package(name: &str) -> ResolvedPackage {
    ResolvedPackage {
        name: name.into(),
        commonjs_export_name: None,
        commonjs_export_assignment: None,
        called_commonjs_namespace_properties: Default::default(),
        functions: Vec::new(),
        values: Vec::new(),
        classes: Vec::new(),
        classifications: Vec::new(),
        native_lib: None,
        native_addon: None,
        native_dependencies: Vec::new(),
        platform_executable: None,
        bundle_js: None,
        factory_class_returns: Default::default(),
        namespace_self_aliases: Default::default(),
        type_only_exports: Default::default(),
        internal_support_classes: Default::default(),
        type_only_value_names: Default::default(),
        nested_namespaces: Default::default(),
    }
}

#[test]
fn thaw_js_newly_allowlisted_builtins_route_to_thaw_js() {
    // the module added to THAW_JS_BUILTIN_MODULES: a metadata-only program must be eligible for thaw-js
    // old: node:cluster, node:console, node:wasi; new: + node:v8, node:test/reporters
    let cases = [
        ("node:cluster", "import cluster from 'node:cluster';\nconsole.log(cluster.isPrimary, cluster.settings);\n"),
        ("node:console", "import { Console } from 'node:console';\nconsole.log(typeof Console);\n"),
        ("node:wasi", "import { WASI } from 'node:wasi';\nconsole.log(typeof WASI);\n"),
        ("node:v8", "import v8 from 'node:v8';\nconsole.log(typeof v8);\n"),
        ("node:test/reporters", "import reporters from 'node:test/reporters';\nconsole.log(typeof reporters);\n"),
    ];
    for (module, user) in cases {
        let bundle = thaw_registry::builtin_bundle_with(module, &[]).unwrap();
        assert!(
            thaw_js_package_eligible(&builtin_routing_package(module), &bundle, user, &Default::default(), ""),
            "{module}"
        );
    }
}

#[test]
fn package_embedding_legacy_engine_worker_threads_routes_to_thaw_js() {
    // The registry's real thaw-js `node:worker_threads` factory (builtins/system/workers.rs), embedded in a package
    // bundle the way bundle/render.rs embeds a builtin the package requires: an entry `pkg/index.js` requires it.
    let qjs = thaw_registry::builtin_bundle_with("node:worker_threads", &[]).unwrap();
    let entry = "var __thaw_bundle_entry_key = \"node:worker_threads\";";
    let maps = "var __thaw_bundle_require_maps = {\n";
    let factories = "var __thaw_bundle_factories = {\n";
    assert!(qjs.contains(entry) && qjs.contains(maps) && qjs.contains(factories));
    let package_factory = "\"pkg/index.js\": function(module, exports, require, requireAsync, __filename) {\n  exports.isMain = require('node:worker_threads').isMainThread;\n},\n";
    let bundle = qjs
        .replacen(entry, "var __thaw_bundle_entry_key = \"pkg/index.js\";", 1)
        .replacen(maps, &format!("{maps}\"pkg/index.js\": {{\"node:worker_threads\": \"node:worker_threads\"}},\n"), 1)
        .replacen(factories, &format!("{factories}{package_factory}"), 1);
    let user = "import pkg from 'pkg';\nconsole.log(pkg.isMain);\n";
    assert!(thaw_js_package_eligible(&builtin_routing_package("pkg"), &bundle, user, &Default::default(), ""));
}

#[test]
fn node_test_mock_timers_route_to_thaw_js() {
    // mock.timers is served by the thaw-js prelude now (no router blocker): the program stays on thaw-js
    let bundle = thaw_registry::builtin_bundle_with("node:test", &[]).unwrap();
    let user = "import { mock } from 'node:test';\nmock.timers.enable({ now: 1000 });\nmock.timers.tick(10);\n";
    assert!(thaw_js_package_eligible(&builtin_routing_package("node:test"), &bundle, user, &Default::default(), ""));
}

#[test]
fn thaw_js_wasi_bundle_routes_without_a_wasm_mention_in_the_program() {
    // the wasi bundle uses WebAssembly: it routes whatever the program says (the build selects the `wasm` feature from the routed bundle)
    let bundle = thaw_registry::builtin_bundle_with("node:wasi", &[]).unwrap();
    let wasi = "import { WASI } from 'node:wasi';\nconsole.log(typeof WASI);\n";
    let pkg = builtin_routing_package("node:wasi");
    assert!(thaw_js_package_eligible(&pkg, &bundle, wasi, &Default::default(), ""));
    // old: `assert!(!thaw_js_package_eligible(&pkg, &bundle, no_wasm, ...))`; new: routes (the wasm blocker is deleted)
    let no_wasm = "import { WASI } from 'node:path';\nconsole.log(typeof WASI);\n";
    assert!(thaw_js_package_eligible(&pkg, &bundle, no_wasm, &Default::default(), ""));
}

#[test]
fn thaw_js_bundle_only_webassembly_routes_without_a_user_mention() {
    // a package bundle that uses WebAssembly routes although the user source never names it.
    // old: rejected by `thaw_js_wasm_blocker` (stayed on thaw-js); new: routed, the build adds the `wasm` feature
    let bundle = "module.exports = { check: function () { return String(new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])) instanceof WebAssembly.Module); } };";
    let pkg = builtin_routing_package("wasm-kit");
    let user = "import { check } from 'wasm-kit';\nconsole.log(check());\n";
    // routing sees the registry form of a plain script (`thaw_js_bundle_form` -> `script_as_bundle`)
    let form = thaw_js::scan::script_as_bundle(bundle);
    assert!(thaw_js_package_eligible(&pkg, &form, user, &Default::default(), ""));
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
fn thaw_js_routing_serves_a_plain_data_package_without_linking_legacy_engine() {
    let (dir, registry, entry) = thaw_js_fixture(
        "plain",
        "import { describe, add } from 'tjs-kit';\n\
         function main(): void { console.log(describe(['a', 'b', 'a'], 'x')); console.log(add(2, 3)); console.log(describe([], 'none')); }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry);
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-kit"]));
    // `apply` (a callback parameter) is declared but never mentioned, so it does not block routing.
    std::fs::remove_dir_all(&registry).unwrap();
    // Same text real Node prints for the equivalent JavaScript.
    assert_eq!(
        thaw_js_run(&output),
        "{ label: 'X', counts: { a: 2, b: 1 } }\n5\n{ label: 'NONE', counts: {} }\n"
    );
    // The linked binary carries no thaw-js engine at all.
    let bytes = std::fs::read(&output).unwrap();
    assert!(
        !bytes.windows(8).any(|window| window == b"rquickjs"),
        "the binary links the retired QuickJS engine (rquickjs symbols present)"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_carries_dates_both_ways_without_linking_legacy_engine() {
    let (dir, registry, entry) = thaw_js_fixture(
        "dates",
        "import { shift, iso } from 'tjs-kit';\n\
         function main(): void { const d = new Date(Date.UTC(2024, 2, 5, 10)); console.log(iso(shift(d, 2))); console.log(iso(shift(d, 1))); console.log(iso(d)); }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry);
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-kit"]));
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(
        thaw_js_run(&output),
        "2024-03-07T10:00:00.000Z\n2024-03-06T10:00:00.000Z\n2024-03-05T10:00:00.000Z\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_decodes_a_nested_array_result_without_linking_legacy_engine() {
    // A typed `string[][]` result is decoded from plain JSON: no interned-native-owner query (which
    // would need the thaw-js realm) may be compiled into a thaw-js build.
    let main = "import { pairs } from 'tjs-kit';\nfunction main(): void { const p = pairs(['a', 'b', 'c']); console.log(p.length, p[0][1], p[1][0]); }\n";
    let (dir, registry, entry) = thaw_js_fixture("nested-array", main);
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry);
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-kit"]));
    assert_eq!(thaw_js_run(&output), "2 b c\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_prints_a_plain_object_result_like_node() {
    // The package's `any` result is a live JS object; `console.log` must inspect it like Node
    // (not `String(value)`, which printed `[object Object]`).
    let main = "import { describe } from 'tjs-kit';\nfunction main(): void { console.log(describe(['a', 'b', 'a'], 'x')); }\n";
    let (dir, registry, entry) = thaw_js_fixture("engine-print", main);
    let output = dir.join("plain");
    thaw_js_build(&entry, &output, &registry);
    assert_eq!(
        thaw_js_run(&output),
        "{ label: 'X', counts: { a: 2, b: 1 } }\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_serves_a_callback_through_thaw_js_without_legacy_engine() {
    let (dir, registry, entry) = thaw_js_fixture(
        "callback",
        "import { apply } from 'tjs-kit';\nfunction main(): void { console.log(apply([1, 2, 3], (x: number): number => x * 2).join(',')); }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry);
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
        )
        .unwrap();
        let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
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
        "declare function __thaw_typed_tjs_6d73(t: string, d: Json): Promise<string>;"
    ));
    assert!(thaw_js_async_declaration_plain(
        "declare function __thaw_typed_tjs_6d73(t: string): string | Promise<string>;"
    ));
    assert!(!thaw_js_async_declaration_plain(
        "declare function __thaw_typed_tjs_6d73(t: string): Promise<Buffer>;"
    ));
    assert!(!thaw_js_async_declaration_plain(
        "declare function __thaw_typed_tjs_6d73(t: Promise<string>): string;"
    ));
    assert!(thaw_js_async_declaration_plain(
        "declare function __thaw_typed_tjs_6d73(path: string): Promise<Uint8Array>;"
    ));
}

#[test]
fn thaw_js_path_family_bundles_are_the_root_bundle_with_a_realm_marker() {
    let root = thaw_registry::resolve_builtin("node:path")
        .unwrap()
        .bundle_js
        .unwrap();
    // (specifier the user writes, export path of the shared `node:path` realm it views)
    for (specifier, member) in [
        ("path", ""),
        ("node:path", ""),
        ("path/posix", "posix"),
        ("node:path/posix", "posix"),
        ("path/win32", "win32"),
        ("node:path/win32", "win32"),
    ] {
        let builtin = path_builtin_name(specifier).unwrap();
        let form = thaw_js_path_bundle(&root, builtin);
        let marker = format!(
            "var __thaw_bundle_realm = {{\"key\":\"node:path\",\"member\":\"{member}\"}};"
        );
        assert!(form.contains(&marker), "{specifier}: {marker} missing");
        assert_eq!(form.matches("__thaw_bundle_realm").count(), 1, "{specifier}");
        let report = thaw_js::scan::scan_bundle(&form).unwrap();
        assert!(report.is_clean(), "{specifier}: {:?}", report.blockers);
        assert_eq!(report.entry, "node:path", "{specifier}");
        // the form is still the root bundle: nothing else is rewritten
        assert_eq!(form.replace(&format!("\n{marker}"), ""), root, "{specifier}");
    }
}

#[test]
fn thaw_js_ordinary_bundles_do_not_carry_the_realm_marker() {
    for module in ["node:url", "node:util", "node:events"] {
        let bundle = thaw_registry::resolve_builtin(module).unwrap().bundle_js.unwrap();
        assert!(!bundle.contains("__thaw_bundle_realm"), "{module}");
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
        "node:v8",
        "node:async_hooks",
        "node:querystring",
        "node:timers",
        "node:worker_threads",
        "node:wasi",
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
        "node:test/reporters",
        "node:console",
    ] {
        assert!(THAW_JS_BUILTIN_MODULES.contains(&module), "{module}");
    }
    // old: `assert!(!THAW_JS_BUILTIN_MODULES.contains(&"node:cluster"), "node:cluster")`; new: allowlisted (fork is blocked separately)
    assert!(THAW_JS_BUILTIN_MODULES.contains(&"node:cluster"), "node:cluster");
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

/// Node 22.23.1 `require('module').builtinModules` with the `_`-prefixed internals removed (54 names).
/// Regenerate with: `node -e "console.log(require('module').builtinModules.filter(n => !n.startsWith('_')))"`.
const NODE22_BUILTIN_MODULES: &[&str] = &[
    "assert", "assert/strict", "async_hooks", "buffer", "child_process", "cluster", "console",
    "constants", "crypto", "dgram", "diagnostics_channel", "dns", "dns/promises", "domain", "events",
    "fs", "fs/promises", "http", "http2", "https", "inspector", "inspector/promises", "module", "net",
    "os", "path", "path/posix", "path/win32", "perf_hooks", "process", "punycode", "querystring",
    "readline", "readline/promises", "repl", "stream", "stream/consumers", "stream/promises",
    "stream/web", "string_decoder", "sys", "timers", "timers/promises", "tls", "trace_events", "tty",
    "url", "util", "util/types", "v8", "vm", "wasi", "worker_threads", "zlib",
];

#[test]
fn thaw_js_builtin_allowlist_matches_node22_builtin_modules() {
    // Prefix-only builtins Node 22 loads but does not list in builtinModules.
    const PREFIX_ONLY: &[&str] = &["node:test", "node:test/reporters", "node:sea", "node:sqlite"];
    for name in NODE22_BUILTIN_MODULES {
        let module = format!("node:{name}");
        assert!(THAW_JS_BUILTIN_MODULES.contains(&module.as_str()), "{module}");
    }
    for module in THAW_JS_BUILTIN_MODULES {
        assert!(
            NODE22_BUILTIN_MODULES.contains(&module.trim_start_matches("node:"))
                || PREFIX_ONLY.contains(module),
            "{module} is allowlisted but is not a Node 22 builtin"
        );
    }
    assert!(THAW_JS_BUILTIN_MODULES.contains(&"node:sqlite"));
}

#[test]
fn thaw_js_sys_is_util_and_sea_reports_not_a_single_executable_app() {
    // Node 22: `require('sys') === require('util')` (DEP0025 warning on stderr only), `isSea()` is false
    // and the asset readers throw ERR_NOT_IN_SINGLE_EXECUTABLE_APPLICATION.
    assert!(THAW_JS_BUILTIN_MODULES.contains(&"node:sys"));
    assert!(THAW_JS_BUILTIN_MODULES.contains(&"node:sea"));
    let sys = thaw_registry::resolve_builtin("node:sys").unwrap();
    assert_eq!(sys.name, "node:sys");
    let mut sys_package =
        thaw_js::host::Package::load(&thaw_registry::builtin_bundle_with("node:sys", &[]).unwrap())
            .unwrap();
    let format_args = [
        thaw_js::host::Native::Str("%s-%d".into()),
        thaw_js::host::Native::Str("a".into()),
        thaw_js::host::Native::Number(2.0),
    ];
    assert_eq!(
        sys_package.call("format", &format_args).ok(),
        Some(thaw_js::host::Native::Str("a-2".into()))
    );
    let sea = thaw_registry::resolve_builtin("node:sea").unwrap();
    assert!(sea.dts_source.contains("isSea"));
    let mut sea_package =
        thaw_js::host::Package::load(&thaw_registry::builtin_bundle_with("node:sea", &[]).unwrap())
            .unwrap();
    assert_eq!(
        sea_package.call("isSea", &[]).ok(),
        Some(thaw_js::host::Native::Bool(false))
    );
    let error = sea_package
        .call("getAsset", &[thaw_js::host::Native::Str("key".into())])
        .unwrap_err()
        .to_string();
    // the host renders a thrown error as `Name: message`, so the code is not part of the text
    assert_eq!(
        error,
        "Error: Operation cannot be invoked when not in a single-executable application"
    );
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
fn thaw_js_refused_package_fails_the_build_and_records_the_blockers_key() {
    // A package whose bundle calls `process.dlopen` is refused: the build fails (no thaw-js rebuild) and the
    // blockers of the failed build name the refused package.
    let (dir, registry, entry) = thaw_js_package_fixture("blockers-fallback", TJS_DLOPEN_MAIN, &[TJS_DLOPEN_PACKAGE]);
    let error = build_on_thaw_js(&entry, &dir.join("app"), &registry).unwrap_err();
    assert!(error.starts_with("unsupported on thaw-js: "), "{error}");
    let blockers = thaw_js_blockers_manifest();
    assert!(
        blockers.as_array().unwrap().iter().any(|blocker| blocker["package"] == "tjs-dl"),
        "{blockers}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A package thaw-js refuses (its bundle calls `process.dlopen`) and the program that calls it: the stable way to
/// make a routed build fail with `unsupported on thaw-js`.
const TJS_DLOPEN_MAIN: &str = "import { load } from 'tjs-dl';\nfunction main(): void {\n  console.log(load('x'));\n}\n";
const TJS_DLOPEN_PACKAGE: (&str, &str, &str) = (
    "tjs-dl",
    "export declare function load(m: string): number;\n",
    "exports.load = function (m) { return process.dlopen(m, 'x.node'); };\n",
);

/// A `createHook` program that uses `finally` and the Promise combinators stays on thaw-js: its hook traces are
/// pinned against Node 22 in thaw-js (`async_hooks_promise_*`), so the module is no longer refused.
#[test]
fn thaw_js_create_hook_with_finally_and_promise_combinators_stays_on_thaw_js() {
    let (dir, registry, entry) = thaw_js_class_fixture(
        "async-hooks-finally",
        "import { Counter } from 'tjs-ctr';\n\
         import { createHook } from 'node:async_hooks';\n\
         function main(): void {\n\
           const c = new Counter(5);\n\
           createHook({ init(): void {} }).enable();\n\
           Promise.resolve(1).finally((): void => {});\n\
           Promise.all([Promise.resolve(1)]);\n\
           console.log(c.add(2));\n\
         }\n",
    );
    let output = dir.join("app");
    thaw_js_build(&entry, &output, &registry);
    assert_eq!(thaw_js_run_guarded(&output), "7\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_fallback_error_mode_fails_the_build_with_every_refusal() {
    let (dir, registry, entry) = thaw_js_package_fixture("fallback-modes", TJS_DLOPEN_MAIN, &[TJS_DLOPEN_PACKAGE]);
    // the refused package fails the build, named with its reason, and no thaw-js rebuild runs
    let error = build_on_thaw_js(&entry, &dir.join("app-error"), &registry).unwrap_err();
    assert!(error.starts_with("unsupported on thaw-js: "), "{error}");
    assert!(error.contains("package `tjs-dl` refused"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_fallback_error_mode_fails_only_when_the_build_links_legacy_engine() {
    // node:crypto's typed `randomFillSync` declaration is refused per function, yet the program still ends up
    // on thaw-js: an `error` build must succeed, since nothing links thaw-js.
    let (dir, registry, entry) = thaw_js_class_fixture(
        "fallback-final-pass",
        "import { randomFillSync } from 'node:crypto';\n\
         function main(): void {\n\
           const b = Buffer.alloc(8);\n\
           randomFillSync(b, 2, 3);\n\
           console.log(b.length);\n\
         }\n",
    );
    let result = build_with_thaw_js_routing(
        &entry, &dir.join("app"), &[], &[], &[], &registry, &[], false, None, true, false, false,
    );
    result.unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

/// Builds `main` on thaw-js; returns the build dir and the output path.
/// Asserts the native-timer pass succeeded (no retry was recorded) unless `expect_retry`.
fn native_timer_program_builds(name: &str, main: &str, expect_retry: bool) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("thaw-cli-native-timer-{name}-{}", std::process::id()));
    let registry = dir.join("modules");
    std::fs::create_dir_all(&registry).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, main).unwrap();
    let output = dir.join("app");
    NATIVE_TIMER_RETRIES.with(|retries| retries.borrow_mut().clear());
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &[], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    let retries = NATIVE_TIMER_RETRIES.with(|retries| retries.borrow().clone());
    assert_eq!(retries.is_empty(), !expect_retry, "native timer retries: {retries:?}");
    (dir, output)
}

#[test]
fn thaw_js_native_timer_resolve_callback_builds_natively_under_error_mode() {
    let (dir, output) = native_timer_program_builds(
        "native-timer-resolve",
        "async function main(){ await new Promise<void>(r => setTimeout(r, 1)); console.log(\"ok\"); }\n",
        false,
    );
    assert_eq!(thaw_js_run(&output), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_native_timer_microtask_and_immediate_resolve_callbacks_build_natively() {
    let (dir, output) = native_timer_program_builds(
        "native-timer-microtask",
        "async function main(): Promise<void> { await new Promise<void>(r => queueMicrotask(r)); console.log(\"ok\"); }\n",
        false,
    );
    assert_eq!(thaw_js_run(&output), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
    let (dir, output) = native_timer_program_builds(
        "native-timer-immediate",
        "async function main(): Promise<void> { await new Promise<void>(r => setImmediate(r)); console.log(\"ok\"); }\n",
        false,
    );
    assert_eq!(thaw_js_run(&output), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_native_timer_sleep_helper_builds_natively() {
    let (dir, output) = native_timer_program_builds(
        "native-timer-sleep",
        "const sleep = (ms: number): Promise<void> => new Promise<void>(r => setTimeout(r, ms));\n\
         async function main(): Promise<void> { await sleep(5); console.log(\"ok\"); }\n",
        false,
    );
    assert_eq!(thaw_js_run(&output), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_native_timer_race_with_rejecting_timeout_builds_and_runs() {
    // `setTimeout(rej, 5)` keeps a required reason parameter, so the native pass still retries.
    let (dir, output) = native_timer_program_builds(
        "native-timer-race",
        "async function main(): Promise<void> {\n\
           const p = new Promise<number>(r => setTimeout(() => r(1), 50));\n\
           const x = await Promise.race([p, new Promise<number>((_, rej) => setTimeout(rej, 5))]).catch(() => -1);\n\
           console.log(\"ok\", x);\n\
         }\n",
        true,
    );
    assert_eq!(thaw_js_run(&output), "ok -1\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// `tjs-factory` (a function returning a callable, like nanoid's `customAlphabet`) and `tjs-template`
/// (a function returning a union of two callables, like ejs's `compile`), both kept off the JIT by `Map`.
fn thaw_js_callable_result_fixture(name: &str, main: &str) -> (PathBuf, PathBuf, PathBuf) {
    thaw_js_package_fixture(
        name,
        main,
        &[
            (
                "tjs-factory",
                "export declare function make(prefix: string): (x: string) => string;\n",
                "exports.make = function (prefix) { var m = new Map(); m.set('p', prefix); return function (x) { return m.get('p') + x; }; };\n",
            ),
            (
                "tjs-template",
                "export declare function compile(template: string): ((arg0?: any) => string) | ((arg0?: any) => Promise<string>);\n",
                "exports.compile = function (template) { var m = new Map(); m.set('t', template); return function (data) { return m.get('t').replace('NAME', data.name); }; };\n",
            ),
        ],
    )
}

/// The registry fixture of `thaw_js_callable_result_fixture` for any `(package, d.ts, module body)` list.
fn thaw_js_package_fixture(
    name: &str,
    main: &str,
    packages: &[(&str, &str, &str)],
) -> (PathBuf, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("thaw-cli-thaw-js-{name}-{}", std::process::id()));
    let registry = dir.join("modules");
    for &(package, declarations, module) in packages {
        let dir_pkg = registry.join(package);
        std::fs::create_dir_all(&dir_pkg).unwrap();
        std::fs::write(dir_pkg.join("package.d.ts"), declarations).unwrap();
        std::fs::write(
            dir_pkg.join("bundle.js"),
            format!(
                "module.exports = (function(require){{\n\
                 var __thaw_bundle_factories = {{\n\
                 \"{package}/index.js\": function(module, exports, require) {{\n{module}}},\n\
                 }};\n\
                 var __thaw_bundle_require_maps = {{ \"{package}/index.js\": {{}}, }};\n\
                 var __thaw_bundle_entry_key = \"{package}/index.js\";\n\
                 var __thaw_entry = {{ exports: {{}} }};\n\
                 __thaw_bundle_factories[__thaw_bundle_entry_key](__thaw_entry, __thaw_entry.exports, require);\n\
                 return __thaw_entry.exports;\n\
                 }})(globalThis.require);\n"
            ),
        )
        .unwrap();
    }
    let entry = dir.join("main.ts");
    std::fs::write(&entry, main).unwrap();
    (dir.clone(), registry, entry)
}

/// The ejs `compile` shape: a union of two callables over `any` parameters, plus an object-typed `opts`.
#[test]
fn thaw_js_union_callable_result_ejs_shape_builds_under_error_mode() {
    let (dir, registry, entry) = thaw_js_package_fixture(
        "ejs-union",
        "import * as ejs from 'tjs-ejs';\n\
         function main(): void {\n\
           const fn = ejs.compile('hello NAME!', { async: false });\n\
           console.log(`${fn({ name: 'ada' })}`);\n\
         }\n",
        &[(
            "tjs-ejs",
            "export declare function compile(template: string, opts?: { async?: boolean; delimiter?: string }): ((arg0?: any) => string) | ((arg0?: any) => Promise<string>);\n",
            "exports.compile = function (template, opts) { var m = new Map(); m.set('t', template); return function (data) { return m.get('t').replace('NAME', data.name); }; };\n",
        )],
    );
    let output = dir.join("app");
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &[], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    assert_eq!(thaw_js_run(&output), "hello ada!\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// An event-emitter `on(event, cb)` whose listener is typed `(...args: any[]) => any` (minipass's
/// `ReadStream.on`): the callback's declared return is `any` (a JsValue), so the class operation must
/// still cross the thaw-js boundary instead of failing the build (`fs-minipass` under error fallback).
#[test]
fn thaw_js_class_callback_returning_any_builds_under_error_mode() {
    let (dir, registry, entry) = thaw_js_package_fixture(
        "class-callback-any",
        "import { ReadStream } from \"tjs-stream\";\n\
         function run(): void {\n\
           const rs = new ReadStream();\n\
           let total = \"\";\n\
           rs.on(\"data\", function (chunk: any) {\n\
             total += chunk.toString();\n\
           });\n\
           rs.on(\"end\", function () {\n\
             console.log(\"end:\", total);\n\
           });\n\
           rs.fire(\"data\", \"hello\");\n\
           rs.fire(\"end\", \"\");\n\
         }\n\
         run();\n",
        &[(
            "tjs-stream",
            "export declare class ReadStream {\n\
               constructor();\n\
               on(event: string, cb: (...args: any[]) => any): this;\n\
               fire(event: string, value: string): void;\n\
             }\n",
            "class ReadStream {\n\
               constructor() { this.listeners = {}; }\n\
               on(event, cb) { (this.listeners[event] ||= []).push(cb); return this; }\n\
               fire(event, value) { for (const cb of this.listeners[event] || []) cb(value); }\n\
             }\n\
             exports.ReadStream = ReadStream;\n",
        )],
    );
    let output = dir.join("app");
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &["tjs-stream".to_string()], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    // Same text real Node prints for the equivalent JavaScript.
    assert_eq!(thaw_js_run(&output), "end: hello\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A single-argument bare callable with no import (`utf-8-validate`'s `isValidUTF8(buffer: Buffer): boolean`
/// via `--use`): thaw-js must still emit a typed bare-name wrapper, since it has no untyped `callDynamic` shim.
#[test]
fn thaw_js_single_arg_bare_callable_builds_under_error_mode() {
    let (dir, registry, entry) = thaw_js_package_fixture(
        "single-arg-bare",
        "function main(): void {\n\
           console.log(isValid(Buffer.from(\"a\")));\n\
         }\n",
        &[(
            "tjs-utf8",
            "export declare function isValid(buf: Buffer): boolean;\n",
            "module.exports.isValid = function isValid(buf) { return buf.length > 0; };\n",
        )],
    );
    let output = dir.join("app");
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &["tjs-utf8".to_string()], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    // Same text real Node prints for the equivalent JavaScript.
    assert_eq!(thaw_js_run(&output), "true\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A builtin `node:` module routed to thaw-js must not get a typed bare alias for its single-argument
/// functions: a module function named `createServer` makes thaw-llvm treat the program as a native http
/// server (`thaw_http_run_servers`), which ignores the latched unhandled-rejection exit.
#[test]
fn thaw_js_node_net_single_arg_functions_get_no_bare_alias() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-node-net-alias-{}", std::process::id()));
    let registry = dir.join("modules");
    let packages = ["node:net".to_string()];
    let active = packages.iter().cloned().collect::<std::collections::HashSet<_>>();
    let source = "import { createServer } from \"node:net\";\nfunction main(): void {\n    const server = createServer();\n    server.close();\n}\n";
    let routed: ThawJsPackages = std::iter::once("node:net".to_string()).collect();
    let (shims, _) = generate_registry_shims_pass(
        &registry, &packages, &active, &active, &std::collections::HashSet::new(), source,
        &thaw_parser::common::FileName::Custom("node net.ts".into()),
        false, &dir.join("app"), Some(&dir.join("staging")),
        &std::collections::HashSet::new(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), &routed,
        &std::collections::HashSet::new(),
    ).unwrap();
    assert!(
        !shims.0.lines().any(|line| line.starts_with("function createServer(")),
        "node:net got a bare createServer alias:\n{}",
        shims.0
    );
}

/// The verbatim ejs 7.0.2 `package.d.ts` (`ejs_package.d.ts`): `compile` has sync, async and union
/// overloads whose function results are adapters. Called as the ejs repro does (`ejs.compile(..)`
/// then `compiled({ name })`). The module body is a stand-in; the shape comes from the d.ts.
#[test]
fn thaw_js_union_callable_result_real_ejs_overloads_build_under_error_mode() {
    let (dir, registry, entry) = thaw_js_package_fixture(
        "ejs-real-overloads",
        "import ejs from 'tjs-ejs';\n\
         function main(): void {\n\
           const compiled = ejs.compile('Hello NAME');\n\
           console.log(compiled({ name: 'Alice' }));\n\
         }\n",
        &[(
            "tjs-ejs",
            include_str!("ejs_package.d.ts"),
            "exports.compile = function (template, opts) { return function (data) { return template.replace('NAME', data.name); }; };\n",
        )],
    );
    let output = dir.join("app");
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &[], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    assert_eq!(thaw_js_run(&output), "Hello Alice\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// The hapi shape: a factory returns a `Server` whose `route` takes an options object with a
/// handler callback that is unannotated in the program (`(request, h) => ..`). The `.d.ts` gives the
/// handler a union type with callback members, so the callback's parameters come from the literal's
/// dynamic receiver, not from a typed signature.
#[test]
fn thaw_js_object_literal_route_handler_on_dynamic_server_builds_under_error_mode() {
    let (dir, registry, entry) = thaw_js_package_fixture(
        "hapi-route",
        "import * as Hapi from 'tjs-hapi';\n\
         async function main(): Promise<void> {\n\
           const server = Hapi.server({ port: 0 });\n\
           server.route({ method: 'GET', path: '/', handler: (request, h) => h.response(request.path) });\n\
           const res = await server.inject('/');\n\
           console.log(res.statusCode);\n\
           console.log(res.payload);\n\
         }\n",
        &[(
            "tjs-hapi",
            "export interface Request { path: string; }\n\
             export interface ResponseToolkit { response(value: any): any; }\n\
             export interface ServerRoute { method: string; path: string; handler: ((request: Request, h: ResponseToolkit) => any) | ((request: Request, h: ResponseToolkit, err: Error) => any) | object; }\n\
             export declare class Server { route(options: ServerRoute | ServerRoute[]): void; inject(path: string): Promise<{ statusCode: number; payload: string }>; }\n\
             export declare function server(options: { port: number }): Server;\n",
            "function Server() { this.routes = {}; }\n\
             Server.prototype.route = function (options) { this.routes[options.method + ' ' + options.path] = options.handler; };\n\
             Server.prototype.inject = function (path) {\n\
               var handler = this.routes['GET ' + path];\n\
               var h = { response: function (value) { return { statusCode: 200, payload: value }; } };\n\
               return Promise.resolve(handler({ path: path }, h));\n\
             };\n\
             exports.server = function (options) { return new Server(options); };\n",
        )],
    );
    let output = dir.join("app");
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &[], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    // Same text real Node prints for the equivalent JavaScript.
    assert_eq!(thaw_js_run(&output), "200\n/\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_callable_results_are_live_handles_and_build_under_error_mode() {
    let (dir, registry, entry) = thaw_js_callable_result_fixture(
        "callable-results",
        "import { make } from 'tjs-factory';\n\
         import { compile } from 'tjs-template';\n\
         function main(): void {\n\
           const shout = make('>> ');\n\
           console.log(shout('hi'));\n\
           const fn = compile('hello NAME!');\n\
           console.log(fn({ name: 'ada' }));\n\
         }\n",
    );
    let output = dir.join("app");
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &[], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    // Same text real Node prints for the equivalent JavaScript.
    assert_eq!(thaw_js_run(&output), ">> hi\nhello ada!\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// The picocolors shape: `export =` of an object whose members are callables, used through a default
/// import (`pc.dim(..)`). The shim must serve the package's value under its `export =` name as a handle.
#[test]
fn thaw_js_export_equals_object_default_import_builds_under_error_mode() {
    let (dir, registry, entry) = thaw_js_package_fixture(
        "export-equals-object",
        "import pc from 'tjs-colors';\n\
         function main(): void {\n\
           console.log(pc.dim('x') + pc.green('y'));\n\
         }\n",
        &[(
            "tjs-colors",
            "interface Colors { dim(s: string): string; green(s: string): string; }\n\
             declare const colors: Colors & { createColors: (enabled?: boolean) => Colors };\n\
             export = colors;\n",
            "module.exports = { dim: function (s) { return '<' + s + '>'; }, green: function (s) { return '[' + s + ']'; } };\n",
        )],
    );
    let output = dir.join("app");
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &[], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    assert_eq!(thaw_js_run(&output), "<x>[y]\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// An ES package's `import.meta.url` (rewritten by `module_transform` to `'file://' + __filename`) must be an
/// absolute `file:///thaw_modules/<pkg>/...` URL, so its directory is `/thaw_modules/<pkg>`, as in the thaw-js
/// lane. Plain string ops keep the package inside the scanner's accepted shape (`node:path` method calls are
/// refused); `fileURLToPath` itself is covered by the real yargs test.
#[test]
fn thaw_js_import_meta_url_dirname_is_absolute_under_error_mode() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-thaw-js-import-meta-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("tjs-url");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare const dir: string;\nexport declare const abs: boolean;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = (function(require){\n\
         var __thaw_bundle_factories = {\n\
         \"tjs-url/index.js\": function(module, exports, require, requireAsync, __filename, __dirname, __thaw_require) {\n\
           var __thaw_import_meta_0;\n\
           var meta = (__thaw_import_meta_0 || (__thaw_import_meta_0 = {url: ('file://' + __filename), resolve: function() { throw new Error('import.meta.resolve is not supported'); }}));\n\
           var url = meta.url;\n\
           exports.dir = url.slice('file://'.length, url.lastIndexOf('/'));\n\
           exports.abs = url.indexOf('file:///') === 0;\n\
         },\n\
         };\n\
         var __thaw_bundle_require_maps = { \"tjs-url/index.js\": {}, };\n\
         var __thaw_bundle_entry_key = \"tjs-url/index.js\";\n\
         })(globalThis.require);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import * as pkg from 'tjs-url';\n\
         function main(): void {\n\
           console.log(pkg.dir + ' ' + pkg.abs);\n\
         }\n",
    )
    .unwrap();
    let output = dir.join("app");
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &[], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    assert_eq!(thaw_js_run(&output), "/thaw_modules/tjs-url true\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// The debug shape: `export =` of a callable with static members, used through a default import
/// (`createDebug.enable(..)`). The callable must stay a handle, not bind to the typed function symbol.
#[test]
fn thaw_js_export_equals_callable_default_import_builds_under_error_mode() {
    let (dir, registry, entry) = thaw_js_package_fixture(
        "export-equals-callable",
        "import createDebug from 'tjs-debug';\n\
         function main(): void {\n\
           createDebug.enable('app:db');\n\
           console.log(createDebug.enabled('app:db'));\n\
           console.log(createDebug.enabled('other'));\n\
           const log = createDebug('app:db');\n\
           log('query');\n\
           console.log('done');\n\
         }\n",
        &[(
            "tjs-debug",
            "declare var debug: debug.Debug & { debug: debug.Debug; default: debug.Debug };\n\
             export = debug;\n\
             export as namespace debug;\n\
             declare namespace debug {\n\
               interface Debug { (namespace: string): (...args: any[]) => void; enable: (namespaces: string) => void; enabled: (namespaces: string) => boolean; }\n\
             }\n",
            "function debug(ns) { return function () { return ns; }; }\n\
             var enabledNames = [];\n\
             debug.enable = function (ns) { enabledNames.push(ns); };\n\
             debug.enabled = function (name) { return enabledNames.indexOf(name) >= 0; };\n\
             module.exports = debug;\n",
        )],
    );
    let output = dir.join("app");
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &[], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    assert_eq!(thaw_js_run(&output), "true\nfalse\ndone\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// The luxon shape: a class with a private constructor used as a bare value for its static constants
/// (`DateTime.DATE_FULL`), while its static method stays typed. The shim must serve the class as a `$value$`
/// handle; without it the program's `DateTime` is the type alias and the build fails (`unknown variable`).
#[test]
fn thaw_js_class_value_for_static_constant_builds_under_error_mode() {
    let (dir, registry, entry) = thaw_js_package_fixture(
        "class-value",
        "import { DateTime } from 'tjs-luxon';\n\
         function main(): void {\n\
           console.log(DateTime.now().toLocaleString(DateTime.DATE_FULL));\n\
           console.log(JSON.stringify(DateTime.DATE_FULL));\n\
         }\n",
        &[(
            "tjs-luxon",
            "export declare class DateTime {\n\
               private constructor(config: unknown);\n\
               static get DATE_FULL(): Intl.DateTimeFormatOptions;\n\
               static now(): DateTime;\n\
               toLocaleString(fmt?: Intl.DateTimeFormatOptions): string;\n\
             }\n",
            "function DateTime() {}\n\
             DateTime.DATE_FULL = { year: 'numeric', month: 'long', day: 'numeric' };\n\
             DateTime.now = function () { return new DateTime(); };\n\
             DateTime.prototype.toLocaleString = function (fmt) { return JSON.stringify(fmt); };\n\
             module.exports.DateTime = DateTime;\n",
        )],
    );
    let output = dir.join("app");
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &[], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    // Same text real Node prints for the equivalent JavaScript.
    assert_eq!(
        thaw_js_run(&output),
        "{\"year\":\"numeric\",\"month\":\"long\",\"day\":\"numeric\"}\n{\"year\":\"numeric\",\"month\":\"long\",\"day\":\"numeric\"}\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A class the program only constructs or calls statically (`new DateTime()`, `DateTime.now()`) gets no value
/// handle: those stay typed operations, and only a bare value use (`DateTime.DATE_FULL`) serves the class.
#[test]
fn thaw_js_class_value_handle_only_for_bare_value_uses() {
    let (dir, registry, _entry) = thaw_js_package_fixture(
        "class-value-negative",
        "",
        &[(
            "tjs-luxon",
            "export declare class DateTime {\n\
               static get DATE_FULL(): Intl.DateTimeFormatOptions;\n\
               static now(): DateTime;\n\
               constructor(ms?: number);\n\
             }\n",
            "function DateTime(ms) { this.ms = ms; }\n\
             DateTime.DATE_FULL = { year: 'numeric' };\n\
             DateTime.now = function () { return new DateTime(0); };\n\
             module.exports.DateTime = DateTime;\n",
        )],
    );
    let packages = ["tjs-luxon".to_string()];
    let active = packages.iter().cloned().collect::<std::collections::HashSet<_>>();
    let routed = active.clone();
    let shim_for = |source: &str| {
        let (shims, _) = generate_registry_shims_pass(
            &registry, &packages, &active, &active, &std::collections::HashSet::new(), source,
            &thaw_parser::common::FileName::Custom("class value.ts".into()),
            false, &dir.join("app"), Some(&dir.join("staging")),
            &std::collections::HashSet::new(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), &routed,
            &std::collections::HashSet::new(),
        ).unwrap();
        shims.0
    };
    let value_handle = "__thaw_value_tjs_luxon_DateTime";
    let typed = shim_for(
        "import { DateTime } from 'tjs-luxon';\nfunction main(): void {\n  console.log(DateTime.now() !== null);\n  console.log(new DateTime(5) !== null);\n}\n",
    );
    assert!(!typed.contains(value_handle), "{typed}");
    let bare = shim_for(
        "import { DateTime } from 'tjs-luxon';\nfunction main(): void {\n  console.log(JSON.stringify(DateTime.DATE_FULL));\n}\n",
    );
    assert!(bare.contains(value_handle), "{bare}");
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
    // The thaw-js-lane override declares randomFillSync as JsValue; thaw-js keeps the Bytes signature.
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
            &std::collections::HashSet::new(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), routed,
            &std::collections::HashSet::new(),
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
            &std::collections::HashSet::new(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), routed,
            &std::collections::HashSet::new(),
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
        &std::collections::HashSet::new(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), &ThawJsPackages::new(),
        &std::collections::HashSet::new(),
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
        &std::collections::HashSet::new(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), &packages.iter().cloned().collect(),
        &std::collections::HashSet::new(),
    ).unwrap();
    let shim = shims.0;
    assert!(shim.contains("declare function createServer("), "{shim}");
    assert!(!shim.contains("callDynamic(\"createServer\""), "{shim}");
    assert!(!shim.contains("function get(argsArray"), "{shim}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routed_node_http_server_only_links_no_legacy_engine() {
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
    thaw_js_build(&entry, &output, &registry);
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
    thaw_js_build(&entry, &output, &registry);
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "err ECONNREFUSED\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routed_node_http_fetch_from_listen_callback_runs_on_thaw_js() {
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
    thaw_js_build(&entry, &output, &registry);
    // A native createServer next to the fetch client runs on thaw-js (the blocker is lifted).
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "err\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routed_fetch_without_node_http_import_runs_on_thaw_js() {
    // Q9 PR 2b: a program that mentions `fetch` but never imports node:http still routes through thaw-js
    // (the forced node:http / node:https bundles give the realm fetch its transport). No server: fetch to a
    // closed port (59123: closed and not on undici's bad-port list) rejects, as under `node`. The shape of
    // the caught error is not asserted: the compiled catch sees the transport's error text, not Node's
    // TypeError('fetch failed'); a real round trip is covered by the createServer + own fetch test.
    let (dir, registry, entry) = thaw_js_fixture(
        "fetch-without-node-http",
        "async function main(): Promise<void> {\n\
           try {\n\
             await fetch('http://127.0.0.1:59123/');\n\
             console.log('unexpected');\n\
           } catch (e: any) {\n\
             console.log('rejected');\n\
           }\n\
         }\n",
    );
    let output = dir.join("app");
    thaw_js_build(&entry, &output, &registry);
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "rejected\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routed_native_server_with_own_fetch_builds_and_matches_node() {
    // Q9 PR 2b: the forced realm-fetch bundles must not add typed declarations, so the program's own
    // `createServer` does not clash with node:https, so the program stays on thaw-js with the native server
    // and the global fetch. Expected stdout is from a real `node` run.
    let (dir, registry, entry) = thaw_js_fixture(
        "native-server-with-own-fetch",
        "import * as http from 'node:http';\n\
         function main(): void {\n\
           const srv = http.createServer((req, res) => { res.end('ok'); });\n\
           srv.listen(0, '127.0.0.1', async () => {\n\
             const addr: any = srv.address();\n\
             const r: Response = await fetch('http://127.0.0.1:' + addr.port + '/');\n\
             console.log(r.status, await r.text());\n\
             srv.close();\n\
           });\n\
         }\n",
    );
    let output = dir.join("app");
    thaw_js_build(&entry, &output, &registry);
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "200 ok\n");
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
    thaw_js_build(&entry, &output, &registry);
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
    thaw_js_build(&entry, &output, &registry);
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "R 201\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_ctor_option_callback_declaration_is_plain_only_for_constructors() {
    let ctor = "declare function __thaw_typed_tjs_246e6577246e6f64653a73747265616d3a3a5772697461626c6524617269747931(opts: { write: ((arg0: JsValue, arg1: string, arg2: () => void) => void) | undefined }): JsValue;";
    let method = "declare function __thaw_typed_tjs_246d6574686f64246e6f64653a73747265616d3a3a5772697461626c6524777269746524617269747931(opts: { write: ((arg0: JsValue, arg1: string, arg2: () => void) => void) | undefined }): JsValue;";
    // a constructor's option object may carry callbacks (`new Writable({ write })`)
    assert!(thaw_js_declaration_plain_with(ctor, true));
    // the same signature is still rejected by default, and for a method operation
    assert!(!thaw_js_declaration_plain_with(ctor, false));
    assert!(!thaw_js_declaration_plain(ctor));
    assert!(!thaw_js_declaration_plain_with(method, false));
}

// old: `@parcel/watcher` was blocked by `thaw_js_unready_addon_blocker` (kept on thaw-js); new: routes to thaw-js like argon2.
#[test]
fn parcel_watcher_and_argon2_native_addon_programs_route_to_thaw_js() {
    let bundle = "module.exports = (function(require){ var __thaw_bundle_factories = { \"pkg/index.js\": function(module, exports, require) { exports.subscribe = function () {}; } }; var __thaw_bundle_require_maps = { \"pkg/index.js\": {} }; var __thaw_bundle_entry_key = \"pkg/index.js\"; })(globalThis.require);";
    let user = "import { subscribe } from 'pkg';\nfunction main(): void { subscribe(); }\n";
    for name in ["@parcel/watcher", "argon2"] {
        let mut pkg = builtin_routing_package(name);
        pkg.native_addon = Some(std::path::PathBuf::from("native.node"));
        assert!(thaw_js_package_eligible(&pkg, bundle, user, &Default::default(), ""), "{name}");
    }
}

#[test]
fn argon2_native_addon_program_routes_to_thaw_js_with_no_blocker() {
    let mut pkg = builtin_routing_package("argon2");
    pkg.native_addon = Some(std::path::PathBuf::from("native.node"));
    let bundle = "module.exports = (function(require){ var __thaw_bundle_factories = { \"argon2/index.js\": function(module, exports, require) { exports.verify = function (h, p) { return h === p; }; } }; var __thaw_bundle_require_maps = { \"argon2/index.js\": {} }; var __thaw_bundle_entry_key = \"argon2/index.js\"; })(globalThis.require);";
    let user = "import { verify } from 'argon2';\nfunction main(): void { console.log(verify('a', 'a')); }\n";
    assert!(thaw_js_package_eligible(&pkg, bundle, user, &Default::default(), ""));
    let blockers = thaw_js_blockers_manifest();
    assert!(
        !blockers
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["package"] == "argon2"),
        "{blockers}"
    );
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
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-report-{name}"), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry);
        assert_eq!(thaw_js_run_guarded(&output), expected, "{name}");
    }
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
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-w4-{}", name.len()), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry);
        assert_eq!(thaw_js_run_guarded(&output), expected, "{name}");
    }
}

#[test]
fn thaw_js_route_all_builds_a_plain_any_program_on_thaw_js() {
    // Route-all is the only mode: this program has no routed package and no dynamic-code marker, yet it
    // builds on thaw-js without linking thaw-js (a program that needed thaw-js would fail the build).
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
    thaw_js_build(&entry, &output, &registry);
    assert_eq!(thaw_js_run_guarded(&output).trim(), "1");
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
        let slug = name.replace(|c: char| !c.is_ascii_alphanumeric(), "-");
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-tick-{slug}"), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry);
        let (stdout, stderr, status) = thaw_js_run_guarded_status(&output);
        assert_eq!((stdout.as_str(), status), (expected, code), "{name}: {stderr}");
    }
}

/// node-compat `filesystem read and write streams`, `audit2e fs readdir ...` and `nd8 readline/promises ...` with
/// default routing: each program must exit on its own within the 20 s guard.
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
        let manifest = thaw_js_build(&entry, &output, &registry);
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
    thaw_js_build(&entry, &output, &registry);
    let (stdout, stderr, status) = thaw_js_run_guarded_status(&output);
    assert_eq!((stdout.lines().count(), status), (7, Some(0)), "{stderr}");
}

/// node-compat `process nextTick ordering` and `es module runs promises before nextTick` (ES module entries:
/// their promise jobs run before the module's ticks, as in Node), routed through thaw-js.
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
        let slug = name.replace(|c: char| !c.is_ascii_alphanumeric(), "-");
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-esm-{slug}"), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry);
        let (stdout, stderr, status) = thaw_js_run_guarded_status(&output);
        assert_eq!((stdout.as_str(), status), (expected, Some(0)), "{name}: {stderr}");
    }
}

/// Q9 cluster C1: the node-compat/builtins/ts-runtime case sources for Iterator, iterator helpers and
/// JSON.rawJSON, routed through thaw-js. The manifests carry no expected output, so the
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
        let slug = name.replace(|c: char| !c.is_ascii_alphanumeric(), "-");
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-c1-{slug}"), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry);
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
        let (_dir, registry, entry) = thaw_js_fixture(&format!("error-factory-{}", name.len()), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry);
        assert_eq!(thaw_js_run_guarded(&output), expected, "{name}");
    }
}


/// builtins / node-compat / typescript-runtime / text-encoding SharedArrayBuffer and Atomics cases
/// (sources from tests/*.json; the manifests have no expected-output field, so each expected text is
/// derived from Node 22 semantics for that source). Cross-worker SAB sharing is not covered here.
#[test]
fn routed_shared_array_buffer_and_atomics_match_node() {
    let cases: [(&str, &str, &str); 11] = [
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
            "atomics-wait-async-shapes",
            "function main(): void {\n  const ia = new Int32Array(new SharedArrayBuffer(8));\n  const r1: any = Atomics.waitAsync(ia, 0, 1);\n  console.log(r1.async, r1.value, Object.keys(r1).join(','));\n  const r2: any = Atomics.waitAsync(ia, 0, 0, 0);\n  console.log(r2.async, r2.value);\n  console.log(Atomics.waitAsync.length, Atomics.waitAsync.name);\n  try { Atomics.waitAsync(new Int32Array(4), 0, 0, 0); } catch (e: any) { console.log('nonshared', e.constructor.name); }\n  const p: any = Atomics.waitAsync(ia, 0, 0, 10);\n  console.log(p.async, typeof p.value.then);\n}",
            "false not-equal async,value\nfalse timed-out\n4 waitAsync\nnonshared TypeError\ntrue function\n",
        ),
        (
            "atomics-wait-async-does-not-keep-the-process-alive",
            "function main(): void {\n  const ia = new Int32Array(new SharedArrayBuffer(8));\n  Atomics.waitAsync(ia, 0, 0, 200).value.then((v: any) => console.log('fired', v));\n  Atomics.waitAsync(ia, 0, 0).value.then((v: any) => console.log('fired inf', v));\n  console.log('end');\n}",
            "end\n",
        ),
        (
            "text-decode-real-buffer-sources",
            "function main(): void { const bytes = new Uint8Array([65, 66, 67, 68]); const buffer = bytes.buffer; const view = new DataView(buffer, 1, 2); const shared = new SharedArrayBuffer(2); new Uint8Array(shared).set([88, 89]); let invalid = ''; try { new TextDecoder().decode({ buffer: buffer, byteOffset: 0, byteLength: 4 } as any); } catch (error: any) { invalid = error.name; } console.log(JSON.stringify([new TextDecoder().decode(buffer), new TextDecoder().decode(view), new TextDecoder().decode(shared), invalid])); }",
            "[\"ABCD\",\"BC\",\"XY\",\"TypeError\"]\n",
        ),
    ];
    for (name, body, expected) in cases {
        let (_dir, registry, entry) = thaw_js_fixture(&format!("route-sab-{name}"), body);
        let output = entry.with_file_name("app");
        thaw_js_build(&entry, &output, &registry);
        let (stdout, stderr, status) = thaw_js_run_guarded_status(&output);
        assert_eq!((stdout.as_str(), status), (expected, Some(0)), "{name}: {stderr}");
    }
}

/// A routed `node:path` package whose `join` is declared (the typed function the shim resolves `path.join(..)` to).
fn path_routing_package_with_join() -> ResolvedPackage {
    let mut pkg = builtin_routing_package("node:path");
    pkg.functions = thaw_bridge::parse_dts("export declare function join(...paths: string[]): string;").unwrap();
    pkg
}

#[test]
fn thaw_js_default_import_value_keeps_its_handle_export() {
    let pkg = path_routing_package_with_join();
    let needs = |user: &str| thaw_js_value_export_needs_handle(&pkg, user, "default");
    // the path-contracts case: the default binding is compared, so it is a value
    assert!(needs("import path, { posix, win32 } from 'node:path'; const ok = [path === posix];"));
    // a bare alias of the default binding is a value use too
    assert!(needs("import p from 'node:path'; const q = p;"));
    // a value passed as an argument
    assert!(needs("import p from 'node:path'; use(p);"));
    // `import { default as p }` binds the same default export
    assert!(needs("import { default as p } from 'node:path'; const q = p;"));
    // `import p = require('node:path')` binds it as well
    assert!(needs("import p = require('node:path'); const q = p;"));
    // the bare `node:`-less specifier names the same builtin
    assert!(needs("import p from 'path'; const q = p;"));
}

#[test]
fn thaw_js_default_import_used_only_as_namespace_calls_stays_typed() {
    let pkg = path_routing_package_with_join();
    let needs = |user: &str| thaw_js_value_export_needs_handle(&pkg, user, "default");
    assert!(!needs("import path from 'node:path'; const s = path.join('a', 'b');"));
    // an unused default import must not force the export
    assert!(!needs("import path from 'node:path'; const s = 1;"));
    // a namespace import binds the module object, not the default export
    assert!(!needs("import * as path from 'node:path'; const s = path.join('a');"));
    // a namespace used as a value is served as its own namespace object (`NAMESPACE_VALUE`), not as `default`
    assert!(!needs("import * as path from 'node:path'; const s = [path];"));
    assert!(thaw_js_value_export_needs_handle(
        &pkg,
        "import * as path from 'node:path'; const s = [path];",
        thaw_js::host::NAMESPACE_VALUE,
    ));
    assert!(!thaw_js_value_export_needs_handle(
        &pkg,
        "import * as path from 'node:path'; const s = path.join('a');",
        thaw_js::host::NAMESPACE_VALUE,
    ));
}

#[test]
fn thaw_js_export_equals_value_is_served_for_its_default_import_local() {
    // `export = picocolors`: the value is named after the export, the program binds it under another local
    let mut pkg = builtin_routing_package("picocolors");
    pkg.commonjs_export_name = Some("picocolors".into());
    let needs = |user: &str| thaw_js_value_export_needs_handle(&pkg, user, "picocolors");
    assert!(needs("import pc from 'picocolors'; const s = pc.dim('x') + 'y';"));
    // a default import of another module does not bind this export
    assert!(!needs("import pc from 'other'; const s = pc.dim('x') + 'y';"));
}

#[test]
fn thaw_js_default_import_of_another_module_does_not_match() {
    let pkg = path_routing_package_with_join();
    let needs = |user: &str| thaw_js_value_export_needs_handle(&pkg, user, "default");
    assert!(!needs("import p from 'node:url'; const q = [p];"));
    assert!(!needs("import p from 'node:url'; import { join } from 'node:path'; const q = [p, join];"));
    // a default import of another module with the same local name as an unused binding is not a use
    assert!(!needs("import path from 'other-pkg'; const q = [path];"));
}

#[test]
fn thaw_js_named_value_export_is_unchanged_by_default_import_bindings() {
    let pkg = path_routing_package_with_join();
    // `posix` is a named value export: its own members differ from the module's, so `posix.join(..)` needs it
    assert!(thaw_js_value_export_needs_handle(&pkg, "import { posix } from 'node:path'; posix.join('a');", "posix"));
    assert!(thaw_js_value_export_needs_handle(&pkg, "import path, { posix } from 'node:path'; path.join('a'); posix.join('a');", "posix"));
}

#[test]
fn path_family_two_specifiers_with_values_stay_on_thaw_js_with_one_realm() {
    // the marker makes `path` and `path/posix` views of one realm, so their values are identical objects
    let bundle = "module.exports = (function(require){ var __thaw_bundle_factories = { \"node:path\": function(module, exports, require) { exports.sep = '/'; } }; var __thaw_bundle_require_maps = { \"node:path\": {} }; var __thaw_bundle_entry_key = \"node:path\"; })(globalThis.require);";
    let split = "import path from 'path'; import { posix } from 'path/posix'; const s = [path === posix];";
    assert!(thaw_js_package_eligible(&builtin_routing_package("path"), bundle, split, &Default::default(), ""));
    assert!(thaw_js_package_eligible(&builtin_routing_package("path/posix"), bundle, split, &Default::default(), ""));
}

#[test]
fn routed_worker_resource_limits_stop_a_runaway_worker() {
    // Node 22.23.1 prints `error ERR_WORKER_OUT_OF_MEMORY` and the program ends with exit 0 (heap_limit, plan B3)
    let body = "import { Worker } from 'node:worker_threads';\n\
        async function main(): Promise<void> {\n\
          const worker = new Worker(`const keep = []; while (true) { keep.push(new Array(100000).fill(1)); }`, { eval: true, resourceLimits: { maxOldGenerationSizeMb: 16 } });\n\
          const result: string = await new Promise<string>((resolve) => { worker.on('error', (e: any) => { resolve('error ' + e.code); }); worker.on('exit', () => { resolve('exit'); }); });\n\
          console.log(result);\n\
        }";
    let (_dir, registry, entry) = thaw_js_fixture("route-worker-limits", body);
    let output = entry.with_file_name("app");
    thaw_js_build(&entry, &output, &registry);
    let (stdout, stderr, status) = thaw_js_run_guarded_status(&output);
    assert_eq!((stdout.as_str(), status), ("error ERR_WORKER_OUT_OF_MEMORY\n", Some(0)), "{stderr}");
}

/// A registry package whose bundle uses WebAssembly through a hand-written 8-byte module header; the user
/// source never mentions WebAssembly, so only the routed bundle can select the `wasm` runtime feature.
#[test]
fn wasm_in_bundle_only_routes_to_thaw_js_and_runs() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-wasm-bundle-only-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("wasm-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), "export declare function check(): string;\n").unwrap();
    // The Map keeps the function on the runtime path, as in `thaw_js_fixture`.
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { check: function () {\n\
           var seen = new Map();\n\
           var module = new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));\n\
           seen.set('ok', module instanceof WebAssembly.Module);\n\
           return String(seen.get('ok'));\n\
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { check } from 'wasm-kit';\nfunction main(): void { console.log(check()); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry);
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["wasm-kit"]));
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "true\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_nested_namespace_class_is_not_a_routing_blocker() {
    // `kdf.OpenSSL` is a dotted class path: thaw-js resolves it through its own class operations (exports_member),
    // so a program naming it routes to thaw-js. Old: the nested-alias blocker kept the package on thaw-js.
    let mut pkg = builtin_routing_package("tjs-nested");
    pkg.classes.push(thaw_bridge::DtsClass {
        name: "kdf.OpenSSL".into(),
        extends: None,
        constructible: true,
        constructors: Vec::new(),
        methods: Vec::new(),
        properties: Vec::new(),
    });
    let bundle = "module.exports = (function(require){ var __thaw_bundle_factories = { \"tjs-nested/index.js\": function(module, exports, require) { function OpenSSL(pw) { this.pw = pw; } OpenSSL.prototype.derive = function (n) { return this.pw + n; }; exports.kdf = { OpenSSL: OpenSSL }; } }; var __thaw_bundle_require_maps = { \"tjs-nested/index.js\": {} }; var __thaw_bundle_entry_key = \"tjs-nested/index.js\"; })(globalThis.require);";
    let user = "import { kdf } from 'tjs-nested';\nfunction main(): void { console.log(new kdf.OpenSSL('x').derive(3)); }\n";
    assert!(thaw_js_package_eligible(&pkg, bundle, user, &Default::default(), ""));
}

#[test]
fn nested_namespace_alias_class_routes_to_thaw_js_without_linking_legacy_engine() {
    // A `.d.ts` `export declare namespace kdf { class OpenSSL }`: the program builds a dotted class path on thaw-js.
    let dir = std::env::temp_dir().join(format!("thaw-cli-thaw-js-nested-alias-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("tjs-nested");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare namespace kdf {\n\
         \x20 class OpenSSL {\n\
         \x20   constructor(password: string);\n\
         \x20   derive(length: number): string;\n\
         \x20 }\n\
         }\n",
    )
    .unwrap();
    // Same bundle layout and evaluation tail as `thaw_js_fixture`'s package.
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = (function(require){\n\
         var __thaw_bundle_factories = {\n\
         \"tjs-nested/index.js\": function(module, exports, require) {\n\
           function OpenSSL(pw) { this.pw = pw; }\n\
           OpenSSL.prototype.derive = function (n) { return this.pw + n; };\n\
           exports.kdf = { OpenSSL: OpenSSL };\n\
         },\n\
         };\n\
         var __thaw_bundle_require_maps = { \"tjs-nested/index.js\": {}, };\n\
         var __thaw_bundle_entry_key = \"tjs-nested/index.js\";\n\
         var __thaw_entry = { exports: {} };\n\
         __thaw_bundle_factories[__thaw_bundle_entry_key](__thaw_entry, __thaw_entry.exports, require);\n\
         return __thaw_entry.exports;\n\
         })(globalThis.require);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { kdf } from 'tjs-nested';\nfunction main(): void { console.log(new kdf.OpenSSL('x').derive(3)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry);
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-nested"]));
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(thaw_js_run(&output), "x3\n");
    let bytes = std::fs::read(&output).unwrap();
    assert!(
        !bytes.windows(8).any(|window| window == b"rquickjs"),
        "the binary links the retired QuickJS engine (rquickjs symbols present)"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A one-module registry bundle (`pkg/index.js`) whose body is `code`.
fn one_package_bundle(code: &str) -> String {
    format!(
        "module.exports = (function(require) {{\nvar __thaw_bundle_factories = {{\n\"pkg/index.js\": function(module, exports, require, requireAsync, __filename, __dirname, __thaw_require) {{\n{code}\n}},\n}};\nvar __thaw_bundle_require_maps = {{ \"pkg/index.js\": {{}}, }};\nvar __thaw_bundle_entry_key = \"pkg/index.js\";\n}})(globalThis.require);"
    )
}

#[test]
fn builtin_defined_methods_make_graphql_and_socket_io_shaped_packages_eligible() {
    // R6: `runStores`/`publish`/`traceSync` (graphql's diagnostics_channel calls) and `unsubscribe` (socket.io's
    // adapter) are methods the registry's JS builtins define, so the packages are served by thaw-js. A Node method
    // no builtin defines, and `process.dlopen`, stay blockers.
    let pkg = builtin_routing_package("pkg");
    let graphql = one_package_bundle(
        "var dc = process.getBuiltinModule('node:diagnostics_channel');\n\
         exports.p = dc?.tracingChannel('graphql:parse');\n\
         exports.run = (ch, ctx, fn) => ch.start.runStores(ctx, () => { ch.error.publish(ctx); return fn(); });\n\
         exports.trace = (ch, src) => ch.traceSync(() => src, {});",
    );
    let socket = one_package_bundle(
        "exports.del = (websocket, room) => websocket.unsubscribe(room);",
    );
    assert!(thaw_js_package_eligible(&pkg, &graphql, "", &Default::default(), ""));
    assert!(thaw_js_package_eligible(&pkg, &socket, "", &Default::default(), ""));
    // a method that no builtin defines is still refused once the bundle can reach a module that owns the name
    // (`writeHeapSnapshot` is a `node:v8` export; a bundle that loads no owner cannot have made a Node call: the
    // owner-module rule)
    let heap = one_package_bundle(
        "exports.h = (p) => (process.getBuiltinModule('node:v8'), p.writeHeapSnapshot());",
    );
    assert!(!thaw_js_package_eligible(&pkg, &heap, "", &Default::default(), ""));
    let dlopen = one_package_bundle("exports.d = (m) => process.dlopen(m, 'x.node');");
    assert!(!thaw_js_package_eligible(&pkg, &dlopen, "", &Default::default(), ""));
    // execa's IPC `channel.refCounted()`: the child_process builtin defines it since #842, so it is served now
    let ipc = one_package_bundle("exports.r = (child) => child.channel.refCounted();");
    assert!(thaw_js_package_eligible(&pkg, &ipc, "", &Default::default(), ""));
}

#[test]
fn unmapped_require_of_an_installed_peer_keeps_a_package_on_legacy_engine() {
    // drizzle-orm's neon driver requires its peer `@neondatabase/serverless`, which the bundle does not map. While the
    // peer is not installed the require is a MODULE_NOT_FOUND at run time, as on Node: the package is served. Once the
    // peer is installed (a bundler gap or a peer added after the bundle) thaw-js could not serve it: thaw-js keeps it.
    let bundle = one_package_bundle(
        "exports.connect = function () { return require('@neondatabase/serverless'); };",
    );
    let pkg = builtin_routing_package("drizzle-orm");
    let (dir, registry, _entry) = thaw_js_package_fixture(
        "drizzle-unmapped-peer",
        "export {};\n",
        &[("drizzle-orm", "export {};\n", "exports.x = 1;\n")],
    );
    let absent = registry_installed_packages(&registry);
    assert!(!absent.contains("@neondatabase/serverless"));
    assert!(thaw_js_package_eligible_in(&pkg, &bundle, "", &Default::default(), "", &absent, &Default::default()));
    let peer = registry.join("@neondatabase").join("serverless");
    std::fs::create_dir_all(&peer).unwrap();
    std::fs::write(peer.join("package.d.ts"), "export {};\n").unwrap();
    let installed = registry_installed_packages(&registry);
    assert!(installed.contains("@neondatabase/serverless") && installed.contains("drizzle-orm"));
    assert!(!thaw_js_package_eligible_in(&pkg, &bundle, "", &Default::default(), "", &installed, &Default::default()));
    let _ = std::fs::remove_dir_all(dir);
}

/// `tjs-peer-a` reaches its peer `tjs-peer-b` by name, lazily (only when `make` runs), as drizzle reaches `pg`.
const TJS_PEER_A: (&str, &str, &str) = (
    "tjs-peer-a",
    "export declare function make(): any;\n",
    "exports.make = function () { return new (require('tjs-peer-b').B)(); };\n",
);

/// `tjs-peer-b`: a class `B`, and `isB` (its own identity check).
const TJS_PEER_B: (&str, &str, &str) = (
    "tjs-peer-b",
    "export declare function isB(x: any): boolean;\n",
    "class B {}\nexports.B = B;\nexports.isB = function (x) { return x instanceof B; };\n",
);

/// Builds `entry` over `registry` on thaw-js (a thaw-js link is a build error).
fn thaw_js_peer_build(entry: &Path, output: &Path, registry: &Path) -> Result<(), String> {
    build_with_thaw_js_routing(
        entry, output, &[], &[], &[], registry, &[], false, None, true, false, false,
    )
}

#[test]
fn thaw_js_peer_package_builds_under_error_mode_and_reaches_its_peer() {
    // a require of an installed, unmapped peer is served once the peer is compiled in as thaw-js (`served_peers`)
    let (dir, registry, entry) = thaw_js_package_fixture(
        "peer-served",
        "import { make } from 'tjs-peer-a';\n\
         import { isB } from 'tjs-peer-b';\n\
         function main(): void {\n\
           console.log(String(isB(make())));\n\
         }\n",
        &[TJS_PEER_A, TJS_PEER_B],
    );
    let output = dir.join("app");
    thaw_js_peer_build(&entry, &output, &registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "true\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_implicit_peer_is_compiled_in_for_a_program_that_only_imports_its_user() {
    // the program never imports `tjs-peer-b`: the build adds the installed peer its bundle requires
    let (dir, registry, entry) = thaw_js_package_fixture(
        "peer-implicit",
        "import { make } from 'tjs-peer-a';\n\
         function main(): void {\n\
           make();\n\
           console.log('made');\n\
         }\n",
        &[TJS_PEER_A, TJS_PEER_B],
    );
    let output = dir.join("app");
    thaw_js_peer_build(&entry, &output, &registry).unwrap();
    assert_eq!(thaw_js_run_guarded(&output), "made\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_refused_peer_refuses_its_dependent_and_the_error_names_both() {
    // `tjs-peer-b` calls process.dlopen (refused): `tjs-peer-a` cannot be served either, so the build fails
    let peer_b_refused = (
        "tjs-peer-b",
        "export declare function isB(x: any): boolean;\n",
        "class B {}\nexports.B = B;\nexports.isB = function (x) { return x instanceof B; };\nexports.load = function (m) { return process.dlopen(m, 'x.node'); };\n",
    );
    let (dir, registry, entry) = thaw_js_package_fixture(
        "peer-refused",
        "import { make } from 'tjs-peer-a';\n\
         function main(): void {\n\
           make();\n\
         }\n",
        &[TJS_PEER_A, peer_b_refused],
    );
    let output = dir.join("app");
    let error = thaw_js_peer_build(&entry, &output, &registry).unwrap_err();
    assert!(error.contains("tjs-peer-a") && error.contains("tjs-peer-b"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_subpath_of_a_served_peer_stays_refused() {
    // `tjs-peer-b/lib/x` is no package the registry serves: only the exact name of a served peer is resolved
    let peer_a_subpath = (
        "tjs-peer-a",
        "export declare function make(): any;\n",
        "exports.make = function () { return new (require('tjs-peer-b/lib/x').B)(); };\n",
    );
    let (dir, registry, entry) = thaw_js_package_fixture(
        "peer-subpath",
        "import { make } from 'tjs-peer-a';\n\
         function main(): void {\n\
           make();\n\
         }\n",
        &[peer_a_subpath, TJS_PEER_B],
    );
    let output = dir.join("app");
    let error = thaw_js_peer_build(&entry, &output, &registry).unwrap_err();
    assert!(error.contains("tjs-peer-a") && error.contains("tjs-peer-b/lib/x"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_peers_register_before_the_packages_that_require_them() {
    // the registration order: a requirer after its peer, the rest in the given order, a cycle as given
    assert_eq!(peers_first_order(&[vec![1], vec![]]), vec![1, 0]);
    assert_eq!(peers_first_order(&[vec![2], vec![], vec![]]), vec![2, 0, 1]);
    assert_eq!(peers_first_order(&[vec![1], vec![2], vec![]]), vec![2, 1, 0]);
    assert_eq!(peers_first_order(&[vec![], vec![], vec![]]), vec![0, 1, 2]);
    assert_eq!(peers_first_order(&[vec![1], vec![0], vec![]]), vec![0, 1, 2]);
}

#[test]
fn builtin_defined_method_names_hold_the_builtin_methods_and_no_helpers() {
    let names = thaw_registry::builtin_defined_names();
    for name in ["publish", "runStores", "traceSync", "unsubscribe", "refCounted"] {
        assert!(names.contains(name), "{name} is not defined by a served builtin");
    }
    // a Node method no builtin defines is not in the set (`getHeapSnapshot` is: the Worker builtin defines it)
    assert!(!names.contains("writeHeapSnapshot"));
    // too generic to excuse a call (`GENERIC_METHOD_NAMES`)
    assert!(!names.contains("get") && !names.contains("write") && !names.contains("on"));
    // module-private `__thaw_*` helpers never are
    assert!(names.iter().all(|name| !name.starts_with("__thaw")));
}

#[test]
fn node_sqlite_bundle_scans_clean() {
    let bundle = thaw_registry::builtin_bundle_with("node:sqlite", &[]).unwrap();
    let report = thaw_js::scan::scan_bundle(&bundle).unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
    assert_eq!(report.entry, "node:sqlite");
}

#[test]
fn node_sqlite_program_builds_and_runs() {
    // Expected stdout is what Node 22.23.1 prints for the same program (its warning goes to stderr).
    const EXPECTED: &str = "1 1\n2\n2 ann 1,2,3 null\ntwo\n9007199254740993\nundefined\nfalse\n";
    let dir = std::env::temp_dir().join(format!("thaw-cli-node-sqlite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { DatabaseSync } from 'node:sqlite';
function main(): void {
  const db = new DatabaseSync(':memory:');
  db.exec('create table t(id integer primary key, name text, data blob)');
  const insert = db.prepare('insert into t (name, data) values (?, ?)');
  const first = insert.run('ann', new Uint8Array([1, 2, 3]));
  console.log(first.changes, first.lastInsertRowid);
  console.log(insert.run('bob', null).lastInsertRowid);
  const rows: any[] = db.prepare('select id, name, data from t order by id').all();
  console.log(rows.length, rows[0].name, Array.from(rows[0].data).join(','), rows[1].data);
  console.log(db.prepare('select :x as x, $y as y').get({ x: 1, y: 'two' }).y);
  const big = db.prepare('select 9007199254740993 as n');
  big.setReadBigInts(true);
  console.log(String(big.get().n));
  console.log(db.prepare('select 1 as a where 0').get());
  db.close();
  console.log(db.isOpen);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    thaw_js_build(&entry, &output, &dir.join("registry"));
    // SQLite is linked because the program imports node:sqlite (its file-header string is in the archive)
    let bytes = std::fs::read(&output).unwrap();
    assert!(bytes.windows(15).any(|window| window == b"SQLite format 3"), "SQLite not linked");
    let result = Command::new(&output).output().unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");
    assert_eq!(String::from_utf8_lossy(&result.stdout), EXPECTED);
    assert_eq!(stderr.matches("ExperimentalWarning: SQLite is an experimental feature").count(), 1, "{stderr}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A tagged template on an imported thaw-js function with a rest parameter (`tag(strings, ...params: any[])`,
/// drizzle's `sql` shape) must pass every interpolated value, in order, not just the strings array.
/// Builds the program with the named values and checks the output against what Node prints.
fn thaw_js_tagged_rest_fixture(name: &str, main_body: &str) -> String {
    let (dir, registry, entry) = thaw_js_package_fixture(
        name,
        &format!(
            "import {{ tag }} from 'tjs-tag';\n\
             function main(): void {{\n\
               {main_body}\n\
             }}\n"
        ),
        &[(
            "tjs-tag",
            "export declare function tag(strings: TemplateStringsArray, ...params: any[]): string;\n",
            "exports.tag = function tag(strings, ...vals) { return JSON.stringify({ s: Array.from(strings), v: vals }); };\n",
        )],
    );
    let output = dir.join("app");
    let result = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &["tjs-tag".to_string()], false, None, true, false, false,
    );
    if let Err(error) = &result {
        eprintln!("build error: {error}");
    }
    result.unwrap();
    let stdout = thaw_js_run(&output);
    let _ = std::fs::remove_dir_all(dir);
    stdout
}

#[test]
fn thaw_js_tagged_template_rest_passes_every_interpolated_value_in_order() {
    let stdout = thaw_js_tagged_rest_fixture(
        "tagged-rest-many",
        "console.log(tag`a${1}b${\"x\"}c`);",
    );
    // Same text real Node prints for the equivalent JavaScript.
    assert_eq!(stdout, "{\"s\":[\"a\",\"b\",\"c\"],\"v\":[1,\"x\"]}\n");
}

#[test]
fn thaw_js_tagged_template_rest_with_one_value() {
    let stdout = thaw_js_tagged_rest_fixture("tagged-rest-one", "console.log(tag`a${1}b`);");
    assert_eq!(stdout, "{\"s\":[\"a\",\"b\"],\"v\":[1]}\n");
}

#[test]
fn thaw_js_tagged_template_rest_with_no_values() {
    let stdout = thaw_js_tagged_rest_fixture("tagged-rest-zero", "console.log(tag`abc`);");
    assert_eq!(stdout, "{\"s\":[\"abc\"],\"v\":[]}\n");
}

#[test]
fn thaw_js_peer_pair_bundles_share_one_group_marker_and_an_unrelated_package_has_none() {
    // `tjs-peer-a` and `tjs-peer-b` are one peer component: both bundles carry the same group key (the least
    // member name); `tjs-unrelated` has no peer edge, so its bundle is a singleton with no marker
    let unrelated = (
        "tjs-unrelated",
        "export declare function ping(): number;\n",
        "exports.ping = function () { return 1; };\n",
    );
    let (dir, registry, entry) = thaw_js_package_fixture(
        "peer-group",
        "import { make } from 'tjs-peer-a';\n\
         import { isB } from 'tjs-peer-b';\n\
         import { ping } from 'tjs-unrelated';\n\
         function main(): void {\n\
           console.log(String(isB(make())), String(ping()));\n\
         }\n",
        &[TJS_PEER_A, TJS_PEER_B, unrelated],
    );
    let output = dir.join("app");
    thaw_js_peer_build(&entry, &output, &registry).unwrap();
    let bytes = std::fs::read(&output).unwrap();
    let count = |text: &str| bytes.windows(text.len()).filter(|w| *w == text.as_bytes()).count();
    assert_eq!(count("var __thaw_bundle_group = \"tjs-peer-a\";"), 2);
    assert_eq!(count("var __thaw_bundle_group = \"tjs-unrelated\";"), 0);
    assert_eq!(count("var __thaw_bundle_group = "), 2);
    assert_eq!(thaw_js_run_guarded(&output), "true 1\n");
    let _ = std::fs::remove_dir_all(dir);
}
