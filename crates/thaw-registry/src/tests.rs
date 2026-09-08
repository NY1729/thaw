use super::*;

include!("tests/resolution.rs");

include!("tests/bundling.rs");

include!("tests/node_core.rs");

include!("tests/stream_web.rs");
include!("tests/stream_lifecycle.rs");
include!("tests/stream_pipeline.rs");
include!("tests/stream_integrations.rs");

include!("tests/platform.rs");

include!("tests/filesystem.rs");

include!("tests/network.rs");

include!("tests/web_platform.rs");

#[test]
fn os_builtin_reports_real_host_shapes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_os_info");
    fs::write(dir.join("index.js"), "var os = require('node:os'); module.exports = function () { var cpus = os.cpus(); var interfaces = os.networkInterfaces(); var user = os.userInfo(); return [typeof os.arch() === 'string' && os.arch().length > 0, typeof os.platform() === 'string' && os.platform().length > 0, typeof os.hostname() === 'string' && os.hostname().length > 0, typeof os.homedir() === 'string', typeof os.tmpdir() === 'string', cpus.length > 0, typeof cpus[0].model === 'string', typeof cpus[0].speed === 'number', os.totalmem() >= os.freemem(), os.totalmem() > 0, os.uptime() >= 0, os.loadavg().length === 3, interfaces.lo.length === 2, interfaces.lo[0].internal, typeof user.username === 'string', os.endianness() === 'LE' || os.endianness() === 'BE', os.devNull === '/dev/null', os.constants.signals.SIGTERM === 15, os.constants.errno.ENOENT === 2, os.EOL === '\\n']; };").unwrap();
    let empty_node_modules = temp_registry("builtin_os_info_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseOsInfo = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseOsInfo").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, format!("[{}]", vec!["true"; 20].join(",")));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

/// The exact real-world pattern that motivated `path`/`os`/`fs`: a real
/// native addon package (`bcrypt`, `utf-8-validate`, ...) depends on
/// `node-gyp-build`, whose real, unmodified source reads
/// `fs.readdirSync` (always wrapped in its own try/catch expecting
/// `[]` back on failure), `path.join`/`path.resolve`/`path.dirname`,
/// and `os.arch`/`os.platform` while hunting for a prebuilt `.node`
/// binary that this registry never bundles. Confirms all three
/// polyfills actually run together through real QuickJS-NG and that
/// `fs.readdirSync` failing is silently absorbed exactly the way real
/// Node's `ENOENT` would be, rather than crashing the whole load.
#[test]
fn path_os_fs_polyfills_actually_run_through_quickjs() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_addon_chase");
    fs::write(
            dir.join("index.js"),
            "var fs = require('fs');\n\
             var path = require('path');\n\
             var os = require('os');\n\
             function readdirSync(d) { try { return fs.readdirSync(d); } catch (err) { return []; } }\n\
             module.exports = function locate() {\n\
             \x20\x20var dir = path.resolve(__dirname);\n\
             \x20\x20var release = readdirSync(path.join(dir, 'build/Release'));\n\
             \x20\x20return os.platform() + '/' + os.arch() + '/' + path.dirname(dir) + '/' + release.length;\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_addon_chase_node_modules");

    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(
        file_count, 5,
        "pkg's index.js + fs/path/os/stream polyfills"
    );

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             globalThis.__dirname = '/thaw_modules/pkg';\n\
             {bundle}\n\
             globalThis.locate = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "bundle failed to load"
    );

    let func = CString::new("locate").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(result, "\"linux/x64//thaw_modules/0\"");

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn plain_commonjs_is_left_untouched() {
    assert!(rewrite_esm_to_commonjs("module.exports = function f() { return 1; };").is_none());
}

#[test]
fn rewrites_default_export_to_module_exports_default() {
    let rewritten =
        rewrite_esm_to_commonjs("export default function greet() { return 'hi'; }").unwrap();
    assert!(rewritten.contains("module.exports.default = function greet() { return 'hi'; }"));
    assert!(rewritten.contains("module.exports.__esModule = true;"));
}

#[test]
fn rewrites_named_export_and_binds_it_too() {
    let rewritten = rewrite_esm_to_commonjs("export function add(a, b) { return a + b; }").unwrap();
    assert!(rewritten.contains("function add(a, b) { return a + b; }"));
    assert!(rewritten.contains("get: function() { return add; }"));
}

#[test]
fn rewrites_named_import_to_a_require_call() {
    let rewritten =
        rewrite_esm_to_commonjs("import { add } from './math';\nconsole.log(add(1, 2));").unwrap();
    assert!(rewritten.contains("require(\"./math\")"));
    assert!(rewritten.contains("console.log(__thaw_esm_import_0[\"add\"](1, 2));"));
    assert!(!rewritten.contains("var add ="));
}

#[test]
fn rewrites_import_meta_url_without_touching_text() {
    let rewritten = rewrite_esm_to_commonjs(
        "export const url = import.meta.url; const text = 'import.meta.url';",
    )
    .unwrap();
    assert!(rewritten.contains("const url = ('file://' + __filename)"));
    assert!(rewritten.contains("const text = 'import.meta.url'"));
}

#[test]
fn live_import_rewrite_respects_shadowing_and_shorthand_properties() {
    let rewritten = rewrite_esm_to_commonjs(
        "import { value } from './state.js';\n\
             function read() {\n\
               const before = value;\n\
               { let value = 9; if (value !== 9) throw new Error('shadow'); }\n\
               return { value }.value + before;\n\
             }",
    )
    .unwrap();
    assert!(rewritten.contains("const before = __thaw_esm_import_0[\"value\"]"));
    assert!(rewritten.contains("let value = 9; if (value !== 9)"));
    assert!(rewritten.contains("return { value: __thaw_esm_import_0[\"value\"] }.value + before"));
}

#[test]
fn live_import_rewrite_respects_function_local_shadowing() {
    let rewritten = rewrite_esm_to_commonjs(
        "import { stringify } from './stringify.js';\n\
         function stringifyCollection(value) {\n\
           const stringify = value ? first : second;\n\
           return stringify(value);\n\
         }",
    )
    .unwrap();
    assert!(
        rewritten.contains("return stringify(value);"),
        "{rewritten}"
    );
}

/// The bundle isn't just plausible-looking text: an ESM main file
/// importing from an ESM sibling file must actually run correctly
/// through the real QuickJS-NG engine, exactly like the equivalent
/// CommonJS package already does (`bundle_actually_runs_through_quickjs`).
#[test]
fn esm_bundle_actually_runs_through_quickjs() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("esm_bundle_runs_through_quickjs");
    fs::write(
            dir.join("index.js"),
            "import { double } from './double.js';\nexport default function run(n) { return double(n); }",
        )
        .unwrap();
    fs::write(
        dir.join("double.js"),
        "export function double(n) { return n * 2; }",
    )
    .unwrap();

    let empty_node_modules = temp_registry("esm_bundle_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2, "index.js + double.js");

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.run = module.exports.default;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "ESM bundle failed to load"
    );

    let func = CString::new("run").unwrap();
    let args = CString::new("[21]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(result, "42");

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn mixed_esm_bundle_supports_live_exports_cycles_imports_json_and_dynamic_import() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("esm_mixed_graph");
    fs::write(
        dir.join("package.json"),
        r##"{"imports":{"#counter":{"node":"./counter.js","default":"./wrong.js"}}}"##,
    )
    .unwrap();
    fs::write(
        dir.join("index.js"),
        "import { increment, value } from '#counter';\n\
             import data from './data.json?payload' with { type: 'json' };\n\
             import { fromA } from './a.js';\n\
             export default async function run() {\n\
               increment();\n\
               const dynamic = await import('./dynamic.js');\n\
               return value + dynamic.extra + data.base + (fromA() === 'b' ? 10 : 0);\n\
             }",
    )
    .unwrap();
    fs::write(
        dir.join("counter.js"),
        "export let value = 1; export function increment() { value++; }",
    )
    .unwrap();
    fs::write(
            dir.join("a.js"),
            "import * as b from './b.js'; export function fromA() { return b.name; } export const name = 'a';",
        )
        .unwrap();
    fs::write(
            dir.join("b.js"),
            "import * as a from './a.js'; export const name = 'b'; export function fromB() { return a.name; }",
        )
        .unwrap();
    fs::write(dir.join("dynamic.js"), "export const extra = 10;").unwrap();
    fs::write(dir.join("data.json"), r#"{"base":20}"#).unwrap();

    let empty_node_modules = temp_registry("esm_mixed_graph_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 6, "{bundle}");
    let script = format!(
        "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(name); }};\n\
             {bundle}\n\
             globalThis.runMixed = module.exports.default;\n"
    );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("runMixed").unwrap();
    let args = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), args.as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn top_level_await_initializes_dependencies_before_export_binding() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("top_level_await_graph");
    fs::write(
        dir.join("index.js"),
        "import { value } from './value.js'; export default function run() { return value + 2; }",
    )
    .unwrap();
    fs::write(
        dir.join("value.js"),
        "export const value = await new Promise(resolve => setTimeout(() => resolve(40), 1));",
    )
    .unwrap();
    let empty_node_modules = temp_registry("top_level_await_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
        "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(name); }};\n\
             {bundle}\n\
             globalThis.runTopLevelAwait = function() {{ return module.exports.default(); }};\n"
    );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("runTopLevelAwait").unwrap();
    let args = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), args.as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn top_level_await_cycle_is_an_explicit_bundle_error() {
    let dir = temp_registry("top_level_await_cycle");
    fs::write(
        dir.join("a.js"),
        "import { b } from './b.js'; export const a = await Promise.resolve(b);",
    )
    .unwrap();
    fs::write(
        dir.join("b.js"),
        "import { a } from './a.js'; export const b = a;",
    )
    .unwrap();
    let empty_node_modules = temp_registry("top_level_await_cycle_modules");
    let error = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "a.js").unwrap_err();
    assert!(error.contains("top-level await module cycle"), "{error}");
    assert!(error.contains("pkg/a.js"), "{error}");
    assert!(error.contains("pkg/b.js"), "{error}");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn validates_json_import_attributes_and_rejects_unsupported_types() {
    let modern = analyze_module(
        "import data from './data.json' with { type: 'json' }; export default data;",
    );
    assert!(modern.attribute_error.is_none());

    let queried = analyze_module(
        "import data from './data.json?payload' with { type: 'json' }; export default data;",
    );
    assert!(queried.attribute_error.is_none());
    assert_eq!(queried.specs, vec!["./data.json?payload"]);

    let legacy = analyze_module(
        "import data from './data.json' assert { type: 'json' }; export default data;",
    );
    assert!(legacy.attribute_error.is_none());

    let unsupported = analyze_module(
        "import source from './code.js' with { type: 'javascript' }; export default source;",
    );
    assert!(unsupported
        .attribute_error
        .as_deref()
        .is_some_and(|error| error.contains("unsupported import attribute")));

    let dynamic = analyze_module("const data = import('./data.json', { with: { type: 'json' } });");
    assert!(dynamic.attribute_error.is_none());
    assert_eq!(dynamic.specs, vec!["./data.json"]);
    let legacy_dynamic =
        analyze_module("const data = import('./data.json', { assert: { type: 'json' } });");
    assert!(legacy_dynamic.attribute_error.is_none());
    let wrong_dynamic =
        analyze_module("const data = import('./data.js', { with: { type: 'json' } });");
    assert!(wrong_dynamic
        .attribute_error
        .as_deref()
        .is_some_and(|error| error.contains("only JSON modules")));
    let unknown_dynamic =
        analyze_module("const data = import('./data.json', { integrity: 'sha256-test' });");
    assert!(unknown_dynamic
        .attribute_error
        .as_deref()
        .is_some_and(|error| error.contains("unsupported dynamic import option")));
    let runtime_attributed =
        analyze_module("const data = import(name, { with: { type: 'json' } });");
    assert!(runtime_attributed
        .attribute_error
        .as_deref()
        .is_some_and(|error| error.contains("finite static specifier set")));

    let rewritten =
        rewrite_dynamic_imports("const data = import('./data.json', { with: { type: 'json' } });")
            .unwrap();
    assert!(rewritten.contains("requireAsync(String('./data.json'))"));
    assert!(!rewritten.contains("type: 'json'"));
}

#[test]
fn nonliteral_dynamic_import_resolves_candidates_and_reuses_namespace() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("runtime_dynamic_import");
    fs::write(
        dir.join("index.js"),
        "export default async function run() {\n\
               const name = 'feature';\n\
               const first = await import('./' + name + '.js');\n\
               const second = await import(`./${name}.js`);\n\
               return first === second ? first.value : 0;\n\
             }",
    )
    .unwrap();
    fs::write(dir.join("feature.js"), "export const value = 42;").unwrap();
    let empty_node_modules = temp_registry("runtime_dynamic_import_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
        "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(name); }};\n\
             {bundle}\n\
             globalThis.runRuntimeImport = module.exports.default;\n"
    );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("runRuntimeImport").unwrap();
    let args = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), args.as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn module_query_and_fragment_are_part_of_cache_identity() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("module_url_identity");
    fs::write(
            dir.join("index.js"),
            "export default async function run() {
               const first = await import('./feature.js?one');
               const again = await import('./feature.js?one');
               const second = await import('./feature.js#two');
               return first === again && first !== second && first.value === 1 && second.value === 2 ? 42 : 0;
             }",
        )
        .unwrap();
    fs::write(
            dir.join("feature.js"),
            "globalThis.__thawModuleIdentity = (globalThis.__thawModuleIdentity || 0) + 1; export const value = globalThis.__thawModuleIdentity;",
        )
        .unwrap();
    let empty_node_modules = temp_registry("module_url_identity_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!(
        "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(name); }};\n\
             {bundle}\n\
             globalThis.runModuleIdentity = module.exports.default;\n"
    );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("runModuleIdentity").unwrap();
    let args = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), args.as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn runtime_dynamic_import_resolves_declared_external_packages() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("constant_external_dynamic_import");
    fs::write(
            dir.join("index.js"),
            "export default async function run(name) { const dep = await import(name); return dep.value; }",
        )
        .unwrap();
    fs::write(
            dir.join("package.json"),
            r#"{"name":"pkg","version":"1.0.0","dependencies":{"dep-a":"1.0.0"},"optionalDependencies":{"dep-b":"1.0.0"}}"#,
        )
        .unwrap();
    let node_modules = temp_registry("constant_external_dynamic_modules");
    for (name, value) in [("dep-a", 41), ("dep-b", 42)] {
        let dependency = node_modules.join(name);
        fs::create_dir_all(&dependency).unwrap();
        let exports = if name == "dep-a" {
            r#", "exports":{".":"./index.js","./feature":"./feature.js","./features/*":"./features/*.js"}"#
        } else {
            ""
        };
        fs::write(
            dependency.join("package.json"),
            format!(r#"{{"name":"{name}","version":"1.0.0","main":"index.js"{exports}}}"#),
        )
        .unwrap();
        fs::write(
            dependency.join("index.js"),
            format!("exports.value = {value};"),
        )
        .unwrap();
        if name == "dep-a" {
            fs::create_dir_all(dependency.join("features")).unwrap();
            fs::write(dependency.join("feature.js"), "exports.value = 43;").unwrap();
            fs::write(dependency.join("features/math.js"), "exports.value = 44;").unwrap();
        } else {
            fs::create_dir_all(dependency.join("lib/tools")).unwrap();
            fs::write(
                    dependency.join("lib/tool.js"),
                    "globalThis.__thawDeepIdentity = (globalThis.__thawDeepIdentity || 44) + 1; exports.value = globalThis.__thawDeepIdentity;",
                )
                .unwrap();
            fs::write(dependency.join("lib/tools/index.js"), "exports.value = 46;").unwrap();
        }
    }
    let (bundle, _, file_count, versions) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 9);
    assert_eq!(versions.get("dep-a").map(String::as_str), Some("1.0.0"));
    assert_eq!(versions.get("dep-b").map(String::as_str), Some("1.0.0"));
    let script = format!(
        "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(name); }};\n\
             {bundle}\n\
             globalThis.runConstantExternalImport = module.exports.default;\n"
    );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("runConstantExternalImport").unwrap();
    for (args, expected) in [
        (r#"["dep-a"]"#, "41"),
        (r#"["dep-b"]"#, "42"),
        (r#"["dep-a/feature"]"#, "43"),
        (r#"["dep-a/features/math"]"#, "44"),
        (r#"["dep-b/lib/tool"]"#, "45"),
        (r#"["dep-b/lib/tool?raw"]"#, "46"),
        (r#"["dep-b/lib/tool?raw"]"#, "46"),
        (r#"["dep-b/lib/tool#part"]"#, "47"),
        (r#"["dep-b/lib/tools"]"#, "46"),
    ] {
        let args = CString::new(args).unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), args.as_ptr());
        assert_eq!(
            unsafe { CStr::from_ptr(result) }.to_string_lossy(),
            expected
        );
    }
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&node_modules);
}

#[test]
fn bare_subpaths_honor_exact_and_wildcard_export_conditions() {
    let node_modules = temp_registry("conditional_subpath_modules");
    let package = node_modules.join("conditional-pkg");
    fs::create_dir_all(package.join("dist/features")).unwrap();
    fs::write(
            package.join("package.json"),
            r#"{"exports":{"./feature":{"require":"./dist/feature.cjs","default":"./wrong.js"},"./features/*":{"require":"./dist/features/*.cjs"}}}"#,
        )
        .unwrap();
    fs::write(package.join("dist/feature.cjs"), "module.exports = 1;").unwrap();
    fs::write(
        package.join("dist/features/math.cjs"),
        "module.exports = 2;",
    )
    .unwrap();

    let (_, exact, ..) = resolve_bare_require(&node_modules, "conditional-pkg/feature").unwrap();
    let (_, wildcard, ..) =
        resolve_bare_require(&node_modules, "conditional-pkg/features/math").unwrap();
    assert_eq!(exact, "./dist/feature.cjs");
    assert_eq!(wildcard, "./dist/features/math.cjs");
    let _ = fs::remove_dir_all(&node_modules);
}
