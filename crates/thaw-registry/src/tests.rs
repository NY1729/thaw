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

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
fn os_priority_validates_arguments_and_reports_native_pid_errors() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_os_priority");
    fs::write(dir.join("index.js"), r#"
        var os = require('node:os');
        module.exports = function () {
            var priority = os.getPriority();
            var error = function (call, code, syscall) {
                try { call(); return false; } catch (failure) {
                    return failure.code === 'ERR_SYSTEM_ERROR' && failure.name === 'SystemError' &&
                        failure.info.code === code && failure.errno === failure.info.errno &&
                        failure.errno < 0 && failure.syscall === syscall;
                }
            };
            return [priority === os.getPriority(0),
                Number.isInteger(priority) && priority >= -20 && priority <= 19,
                os.constants.priority.PRIORITY_HIGHEST === -20,
                os.constants.priority.PRIORITY_LOW === 19,
                error(function () { os.getPriority(2147483647); }, 'ESRCH', 'uv_os_getpriority'),
                error(function () { os.setPriority(2147483647, 0); }, 'ESRCH', 'uv_os_setpriority'),
                (function () { try { os.setPriority(0, 20); } catch (failure) { return failure instanceof RangeError && failure.code === 'ERR_OUT_OF_RANGE'; } return false; })(),
                (function () { try { os.getPriority(null); } catch (failure) { return failure instanceof TypeError && failure.code === 'ERR_INVALID_ARG_TYPE'; } return false; })(),
                (function () { try { os.setPriority(1.5); } catch (failure) { return failure instanceof RangeError && failure.code === 'ERR_OUT_OF_RANGE'; } return false; })(),
                (function () { try { os.setPriority(); } catch (failure) { return failure instanceof TypeError && failure.code === 'ERR_INVALID_ARG_TYPE'; } return false; })()
            ];
        };
    "#).unwrap();
    let empty_node_modules = temp_registry("builtin_os_priority_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseOsPriority = module.exports;");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseOsPriority").unwrap().as_ptr(), CString::new("[]").unwrap().as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "[true,true,true,true,true,true,true,true,true,true]");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

/// `os.networkInterfaces()` used to be hardcoded to a loopback-only
/// literal, regardless of the host's real interfaces -- unlike every
/// other `os.*` accessor in the same module (`arch`/`platform`/`cpus`/
/// etc.), all of which already read real host data. Fixed via
/// `getifaddrs` (the standard POSIX interface-enumeration call,
/// already available through the existing `libc` dependency). This
/// machine always has at least loopback plus one more interface in CI
/// (the runner's own network device), so asserting more than one
/// interface name comes back is a real, non-hardcoded-loopback-only
/// check rather than a tautology.
#[test]
fn os_network_interfaces_reports_more_than_loopback_with_real_shapes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_os_network_interfaces");
    fs::write(
        dir.join("index.js"),
        "var os = require('node:os'); module.exports = function () {\n\
             var interfaces = os.networkInterfaces();\n\
             var names = Object.keys(interfaces);\n\
             var macPattern = /^([0-9a-f]{2}:){5}[0-9a-f]{2}$/;\n\
             var shapesOk = names.every(function (name) {\n\
                 return interfaces[name].every(function (entry) {\n\
                     return (entry.family === 'IPv4' || entry.family === 'IPv6')\n\
                         && typeof entry.address === 'string' && entry.address.length > 0\n\
                         && typeof entry.netmask === 'string' && entry.netmask.length > 0\n\
                         && macPattern.test(entry.mac)\n\
                         && typeof entry.internal === 'boolean'\n\
                         && entry.cidr === entry.address + '/' + entry.cidr.split('/')[1];\n\
                 });\n\
             });\n\
             return [names.length > 1, shapesOk, interfaces.lo[0].mac === '00:00:00:00:00:00'];\n\
         };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_os_network_interfaces_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetworkInterfaces = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNetworkInterfaces").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "[true,true,true]");
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
    assert!(rewritten.contains("function greet() { return 'hi'; }"));
    assert!(rewritten.contains("get: function() { return greet; }"));
    assert!(rewritten.contains("module.exports.__esModule = true;"));
}

#[test]
fn named_default_declarations_remain_local_bindings() {
    use std::ffi::{CStr, CString};

    // Unrun regression: both declaration kinds are visible to later code in
    // their own module; anonymous defaults retain the expression path.
    let anonymous = rewrite_esm_to_commonjs("export default function () { return 1; }").unwrap();
    assert!(anonymous.contains("module.exports.default = function () { return 1; }"));
    // Separate modules: ESM permits only one default export per module.
    let cases = [
        ("export default function greet() { return 'hi'; } export function call() { return greet(); }",
         "[\"hi\",\"hi\"]", "[module.exports.default(),module.exports.call()]"),
        ("export default class Greeting { value() { return 4; } } export function call() { return new Greeting().value(); }",
         "[4,4]", "[(new module.exports.default()).value(),module.exports.call()]"),
    ];
    for (source, expected, expression) in cases {
        let rewritten = rewrite_esm_to_commonjs(source).unwrap();
        let script = format!("globalThis.__thaw_named_default = (function() {{ \
            var module = {{ exports: {{}} }}, exports = module.exports; \
            {rewritten} return function() {{ return {expression}; }}; }})();");
        assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
        let result = thaw_quickjs::thaw_js_call(
            CString::new("__thaw_named_default").unwrap().as_ptr(),
            CString::new("[]").unwrap().as_ptr(),
        );
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_str().unwrap(), expected);
    }
}

#[test]
fn export_star_does_not_redefine_explicit_local_or_reexported_names() {
    use std::ffi::{CStr, CString};

    // Unrun regression: explicit bindings take priority even when they are
    // declared after the star. Destructuring and named reexports are explicit.
    let source = "export * from './a'; \
        export { remote as chosen } from './c'; export const local = 7; \
        export const { value: destructured } = { value: 11 };";
    for await_imports in [false, true] {
        let rewritten = rewrite_esm_to_commonjs_mode(source, await_imports).unwrap();
        assert!(rewritten.contains(".track(exports)"), "{rewritten}");
        assert_eq!(rewritten.contains("await requireAsync"), await_imports);
    }
    let dir = temp_registry("star_explicit_priority");
    let node_modules = temp_registry("star_explicit_priority_modules");
    fs::write(dir.join("index.js"), source).unwrap();
    fs::write(dir.join("a.js"), "module.exports = { local: 1, chosen: 2, destructured: 3, starOnly: 4 };").unwrap();
    fs::write(dir.join("c.js"), "module.exports = { remote: 10 };").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.__thaw_star_explicit_case = function() {{ return [module.exports.local, module.exports.destructured, module.exports.chosen, module.exports.starOnly]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(
        CString::new("__thaw_star_explicit_case").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_str().unwrap(), "[7,11,10,4]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}


#[test]
fn star_rewrite_uses_the_bundle_origin_graph() {
    use std::ffi::{CStr, CString};
    let source = "import { x } from './dep.js'; export * from './dep.js'; export const y = x + 1;";
    // A standalone source string cannot distinguish an export diamond from
    // two different bindings with the same value.
    assert!(rewrite_esm_to_commonjs(source).is_none());
    let dir = temp_registry("star_import_origin");
    let node_modules = temp_registry("star_import_origin_modules");
    fs::write(dir.join("index.js"), source).unwrap();
    fs::write(dir.join("dep.js"), "export const x = 4;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.__thaw_star_import = function() {{ return [module.exports.x, module.exports.y]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"__thaw_star_import".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_str().unwrap(), "[4,5]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
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
fn esm_synthetic_names_avoid_source_bindings_in_sync_and_async_rewrites() {
    // Unrun regression: the reference and declaration passes must choose the
    // same unused range, including identifiers spelled with Unicode escapes.
    let source = "import { value } from './dep'; \
        const __thaw_esm_import_0 = 7, __thaw_esm_import_\\u0031 = 9; \
        export { value as again } from './dep'; \
        const __thaw_esm_reexport_0 = 3; \
        export * from './other'; \
        const __thaw_esm_reexport_all_0 = 4, __thaw_esm_key_0 = 5; \
        export const answer = value + __thaw_esm_import_0 + __thaw_esm_import_\\u0031;";
    for await_imports in [false, true] {
        let rewritten = rewrite_esm_to_commonjs_mode(source, await_imports).unwrap();
        assert!(rewritten.contains("var __thaw_esm_import_2 = "), "{rewritten}");
        assert!(rewritten.contains("__thaw_esm_import_2[\"value\"]"), "{rewritten}");
        assert!(rewritten.contains("var __thaw_esm_reexport_3 = "), "{rewritten}");
        assert!(rewritten.contains("var __thaw_esm_reexport_all_4 = "), "{rewritten}");
        assert!(rewritten.contains("__thaw_esm_origin_2.track(exports)"), "{rewritten}");
        assert!(rewritten.contains("const __thaw_esm_import_0 = 7"), "{rewritten}");
        assert_eq!(rewritten.contains("await requireAsync"), await_imports, "{rewritten}");
    }
}

#[test]
fn rewrites_import_meta_url_without_touching_text() {
    let rewritten = rewrite_esm_to_commonjs(
        "export const url = import.meta.url; const text = 'import.meta.url';",
    )
    .unwrap();
    assert!(
        rewritten.contains("const url = (__thaw_import_meta_0 ||"),
        "{rewritten}"
    );
    assert!(
        rewritten.contains("const text = 'import.meta.url'"),
        "{rewritten}"
    );
}

/// A common ESM/CJS dual-package shim (`const __dirname =
/// fileURLToPath(dirname(import.meta.url));`, `const require =
/// createRequire(import.meta.url);`) redeclares the exact same names
/// this bundler's own per-module wrapper function already provides as
/// real parameters (`module`/`exports`/`require`/`requireAsync`/
/// `__filename`/`__dirname`, see `bundle/render.rs`). Real trigger:
/// yargs's own `platform-shims/esm.mjs`. QuickJS (like real JS) rejects
/// a lexical redeclaration of a parameter name outright ("invalid
/// redefinition of parameter name"); the fix drops the `const`/`let`
/// keyword, turning it into a plain reassignment of the existing
/// parameter instead of a colliding new binding.
#[test]
fn strips_a_top_level_const_that_redeclares_a_reserved_wrapper_parameter() {
    let rewritten = rewrite_esm_to_commonjs(
        "const __dirname = 'computed'; const require = 'also computed'; export const kept = 1;",
    )
    .unwrap();
    assert!(!rewritten.contains("const __dirname ="), "{rewritten}");
    assert!(!rewritten.contains("const require ="), "{rewritten}");
    assert!(rewritten.contains("__dirname = 'computed';"), "{rewritten}");
    assert!(
        rewritten.contains("require = 'also computed';"),
        "{rewritten}"
    );
    assert!(rewritten.contains("const kept = 1;"), "{rewritten}");
}

/// `import.meta.resolve(...)` (or any `import.meta.*` access besides
/// `.url`) used to survive the CJS rewrite untouched -- QuickJS's
/// script-mode parser (this bundler always evaluates as a script, not
/// real ESM) rejects a literal `import.meta` outright with "import.meta
/// only valid in module code", even on a code path the program never
/// actually reaches (real trigger: yargs's own unused `.config()`
/// "extends" feature). Every `import.meta` expression, not just
/// `.url`, must be replaced with something QuickJS can parse.
#[test]
fn rewrites_import_meta_resolve_so_the_bundle_still_parses() {
    let rewritten = rewrite_esm_to_commonjs(
        "export function extend(spec) { return import.meta.resolve(spec); }",
    )
    .unwrap();
    assert!(
        !rewritten.contains("import.meta.resolve(spec)"),
        "{rewritten}"
    );
    assert!(rewritten.contains("resolve: function()"), "{rewritten}");
    assert!(thaw_parser::parse_javascript(&rewritten).is_ok(), "{rewritten}");
}

#[test]
fn rewrites_import_meta_as_an_expression_in_every_position() {
    // Unrun regression: an unparenthesized object after `=>` is parsed
    // as a block, not the arrow function's returned object value.
    let rewritten = rewrite_esm_to_commonjs(
        "export const meta = () => import.meta; export const url = import.meta.url; \
         export const resolved = import.meta.resolve('x'); import.meta.url;",
    )
    .unwrap();
    assert!(rewritten.contains("const meta = () => (__thaw_import_meta_0 ||"), "{rewritten}");
    assert!(rewritten.contains("})).url"), "{rewritten}");
    assert!(rewritten.contains("})).resolve('x')"), "{rewritten}");
    assert!(thaw_parser::parse_javascript(&rewritten).is_ok(), "{rewritten}");
}

#[test]
fn rewrites_bare_import_meta_expression_statement_without_export_syntax() {
    // Unrun regression: ESM can use import.meta without an import/export
    // declaration, and the CJS wrapper still evaluates it as script code.
    let rewritten = rewrite_esm_to_commonjs("import.meta.url;").unwrap();
    assert!(rewritten.starts_with("(__thaw_import_meta_0 ||"), "{rewritten}");
    assert!(rewritten.contains("})).url;"), "{rewritten}");
    assert!(rewritten.ends_with("var __thaw_import_meta_0;"), "{rewritten}");
    assert!(thaw_parser::parse_javascript(&rewritten).is_ok(), "{rewritten}");
}

#[test]
fn import_meta_is_one_mutable_object_without_a_generated_name_collision() {
    use std::ffi::{CStr, CString};

    // Unrun regression: the source owns one plain and one escaped
    // candidate name. Both must be distinct from the generated slot.
    let rewritten = rewrite_esm_to_commonjs(
        "const __thaw_import_meta_0 = 17, __thaw_import_meta_\\u0031 = 19; \
         export function check() { const first = import.meta; first.extra = 1; \
         return [first === import.meta, import.meta.extra, __thaw_import_meta_0, __thaw_import_meta_\\u0031]; }",
    )
    .unwrap();
    assert!(rewritten.contains("var __thaw_import_meta_2;"), "{rewritten}");
    let script = format!(
        "globalThis.__thaw_import_meta_identity_case = (function() {{ \
         var module = {{ exports: {{}} }}, __filename = '/module.js'; \
         {rewritten} return module.exports.check; }})();"
    );
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(
        CString::new("__thaw_import_meta_identity_case").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_str().unwrap(), "[true,1,17,19]");
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

#[test]
fn live_import_rewrite_keeps_named_function_expression_self_binding() {
    // Unrun regression: the function-expression name exists in its
    // parameters and body, but the same spelling outside is the import.
    let rewritten = rewrite_esm_to_commonjs(
        "import { x } from './dep.js'; \
         const fn = function x(value = x) { return value === x; }; \
         export const outside = x;",
    )
    .unwrap();
    assert!(rewritten.contains("function x(value = x) { return value === x; }"), "{rewritten}");
    assert!(rewritten.contains("const outside = __thaw_esm_import_0[\"x\"]"), "{rewritten}");
}

#[test]
fn live_import_rewrite_respects_hoisted_var_across_nested_statements() {
    // Unrun regression: var belongs to the whole function, including text
    // before its declaration, but not to an enclosing function.
    let rewritten = rewrite_esm_to_commonjs(
        "import { x } from './dep.js'; \
         function read() { const before = x; if (false) { var x; } return before; } \
         const arrow = () => { const before = x; for (var x of []) {} return before; }; \
         const lexicalArrow = () => { const before = x; let x = 1; return before; }; \
         function outer() { function inner() { var x; return x; } return x; }",
    )
    .unwrap();
    assert!(rewritten.contains("const before = x; if (false) { var x; }"), "{rewritten}");
    assert!(rewritten.contains("const before = x; for (var x of [])"), "{rewritten}");
    assert!(rewritten.contains("const before = x; let x = 1;"), "{rewritten}");
    assert!(rewritten.contains("function inner() { var x; return x; }"), "{rewritten}");
    assert!(rewritten.contains("return __thaw_esm_import_0[\"x\"]; }"), "{rewritten}");
}

#[test]
fn live_import_rewrite_respects_later_parameter_bindings() {
    // Unrun regression: all parameter names exist before defaults evaluate.
    let rewritten = rewrite_esm_to_commonjs(
        "import { x } from './dep.js'; \
         function regular(before = x, x = 1) { return before; } \
         const arrow = (before = x, x = 1) => before; \
         const outside = x;",
    )
    .unwrap();
    assert!(rewritten.contains("function regular(before = x, x = 1)"), "{rewritten}");
    assert!(rewritten.contains("(before = x, x = 1) => before"), "{rewritten}");
    assert!(rewritten.contains("const outside = __thaw_esm_import_0[\"x\"]"), "{rewritten}");
}

#[test]
fn live_import_rewrite_respects_loop_header_lexical_bindings() {
    // Unrun regression: let/const hide the import within the loop only.
    let rewritten = rewrite_esm_to_commonjs(
        "import { x } from './dep.js'; \
         for (let x = 0; x < 1; x++) { console.log(x); } \
         for (const x of [1]) console.log(x); \
         for (const x in { a: 1 }) console.log(x); \
         console.log(x);",
    )
    .unwrap();
    assert!(rewritten.contains("let x = 0; x < 1; x++"), "{rewritten}");
    assert!(rewritten.contains("for (const x of [1]) console.log(x)"), "{rewritten}");
    assert!(rewritten.contains("for (const x in { a: 1 }) console.log(x)"), "{rewritten}");
    assert!(rewritten.contains("console.log(__thaw_esm_import_0[\"x\"]);"), "{rewritten}");
}

#[test]
fn live_import_rewrite_respects_switch_and_static_block_scopes() {
    // Unrun regression: switch lexical names and static-block var names do
    // not leak to the surrounding module.
    let rewritten = rewrite_esm_to_commonjs(
        "import { x } from './dep.js'; \
         switch (x) { case 1: let x = 2; console.log(x); break; } \
         class Holder { static { const before = x; if (false) { var x; } } } \
         console.log(x);",
    )
    .unwrap();
    assert!(rewritten.contains("switch (__thaw_esm_import_0[\"x\"])"), "{rewritten}");
    assert!(rewritten.contains("let x = 2; console.log(x)"), "{rewritten}");
    assert!(rewritten.contains("const before = x; if (false) { var x; }"), "{rewritten}");
    assert!(rewritten.contains("console.log(__thaw_esm_import_0[\"x\"]);"), "{rewritten}");
}

#[test]
fn live_import_rewrite_updates_destructured_parameter_defaults() {
    let rewritten = rewrite_esm_to_commonjs(
        "import process from 'node:process';\n\
         import pathKey from 'path-key';\n\
         export const run = ({ path = process.env[pathKey()], execPath = process.execPath } = {}) => [path, execPath];",
    )
    .unwrap();
    assert!(
        rewritten.contains("__thaw_esm_import_0") && rewritten.contains("__thaw_esm_import_1"),
        "{rewritten}"
    );
    assert!(!rewritten.contains("process.env[pathKey()]"), "{rewritten}");
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
fn top_level_for_await_initializes_dependencies_before_export_binding() {
    use std::ffi::{CStr, CString};

    // Unrun regression: the loop has no AwaitExpr node, but its module
    // and a static importer still require async initialization.
    assert!(!analyze_module(
        "export async function later() { for await (const item of values) {} }"
    ).has_top_level_await);
    assert!(!analyze_module(
        "export const later = async () => { for await (const item of values) {} };"
    ).has_top_level_await);
    assert!(analyze_module(
        "export const values = []; for (const item of await source()) {}"
    ).has_top_level_await);
    assert!(analyze_module(
        "export let value = 0; for await (const item of values) { value = item; }"
    ).has_top_level_await);
    let dir = temp_registry("top_level_for_await_graph");
    fs::write(dir.join("index.js"),
        "import { value } from './value.js'; export default function run() { return value + 2; }"
    ).unwrap();
    fs::write(dir.join("value.js"),
        "export let value = 0; for await (const item of [Promise.resolve(40)]) { value = item; }"
    ).unwrap();
    let empty_node_modules = temp_registry("top_level_for_await_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
        "globalThis.module = {{ exports: {{}} }}; \
         globalThis.exports = globalThis.module.exports; \
         globalThis.require = function(name) {{ throw new Error(name); }}; \
         {bundle} globalThis.runTopLevelForAwait = function() {{ return module.exports.default(); }};"
    );
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(
        CString::new("runTopLevelForAwait").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
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
fn nested_dynamic_import_rewrites_without_overlapping_ranges() {
    // Unrun regression for an outer import whose argument contains another import.
    let source = "async function choose() { return import((await import('./flag.js')) ? './a.js' : './b.js'); }";
    let rewritten = rewrite_dynamic_imports(source).unwrap();
    assert!(thaw_parser::parse_javascript_with_source_map(&rewritten).is_ok());
    assert_eq!(rewritten.matches("requireAsync(").count(), 2);
    assert!(!rewritten.contains("import("));
    assert!(rewritten.contains("'./flag.js'"));
    assert!(rewritten.contains("'./a.js' : './b.js'"));
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

    let (_, exact, ..) =
        resolve_bare_require(&node_modules, &node_modules, "conditional-pkg/feature").unwrap();
    let (_, wildcard, ..) = resolve_bare_require(
        &node_modules,
        &node_modules,
        "conditional-pkg/features/math",
    )
    .unwrap();
    assert_eq!(exact, "./dist/feature.cjs");
    assert_eq!(wildcard, "./dist/features/math.cjs");
    let _ = fs::remove_dir_all(&node_modules);
}

#[test]
fn registry_bundle_javascript_source_maps_keep_exact_or_generated_names() {
    use thaw_parser::common::{FileName, Spanned};

    let node_modules = temp_registry("bundle_js_source_names");
    let package = node_modules.join("pkg");
    fs::create_dir_all(&package).unwrap();
    let raw = package.join("raw.js");
    let shebang = package.join("shebang.js");
    let json = package.join("data.json");
    let worker_entry = package.join("worker-entry.js");
    let worker = package.join("worker.js");
    fs::write(&raw, "exports.value = 1;").unwrap();
    fs::write(&shebang, "#!/usr/bin/env node\nexports.value = 2;").unwrap();
    fs::write(&json, r#"{"value":3}"#).unwrap();
    fs::write(&worker, "export const value = 4;").unwrap();
    fs::write(&worker_entry, "var Worker = require('node:worker_threads').Worker; new Worker(new URL('./worker.js', import.meta.url));").unwrap();
    let mut cache = SourceCache::default();
    for entry in ["raw.js", "shebang.js", "data.json", "worker-entry.js"] {
        bundle_commonjs_package_cached(&node_modules, "pkg", &package, entry, &mut cache).unwrap();
    }
    for (path, expected) in [
        (&raw, FileName::Real(raw.clone())),
        (&shebang, FileName::Custom(format!("{} (after shebang removal)", shebang.display()).into())),
        (&json, FileName::Custom(format!("{} (generated JSON module)", json.display()).into())),
        (&worker_entry, FileName::Custom(format!("{} (Worker URL rewrite)", worker_entry.display()).into())),
    ] {
        let (source, _, name) = cache.modules.get(&(path.clone(), "pkg".to_string())).unwrap();
        assert_eq!(name, &expected);
        let (module, map) = thaw_parser::parse_javascript_with_source_map_named(source, name.clone()).unwrap();
        assert_eq!(map.lookup_char_pos(module.body[0].span().lo).file.name.as_ref(), &expected);
    }
    let worker_name = FileName::Real(worker.clone());
    assert!(esm_origin_parameter_named(&fs::read_to_string(&worker).unwrap(), &worker_name).is_some());
    let (module, map) = thaw_parser::parse_javascript_with_source_map_named(&fs::read_to_string(&worker).unwrap(), worker_name.clone()).unwrap();
    assert_eq!(map.lookup_char_pos(module.body[0].span().lo).file.name.as_ref(), &worker_name);
    let malformed = "export const =;";
    let malformed_name = FileName::Real(raw.clone());
    assert!(analyze_module_named(malformed, &malformed_name).specs.is_empty());
    assert!(rewrite_esm_to_commonjs_mode_named(malformed, false, &malformed_name).is_none());
    let mut builtins = Vec::new();
    add_builtin_module("path", "node:path".to_string(), &mut Vec::new(), &mut builtins);
    let path_builtin = builtins.iter().find(|module| module.key == "node:path").unwrap();
    assert_eq!(path_builtin.source_name, FileName::Custom("node:path (generated builtin module)".into()));
    let _ = fs::remove_dir_all(node_modules);
}
