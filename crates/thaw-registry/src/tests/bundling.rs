#[test]
fn bundled_json_preserves_proto_keys_and_other_json_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("bundle_json_proto_keys");
    fs::write(dir.join("index.js"), "module.exports = [require('./top.json'), require('./nested.json'), require('./scalar.json'), require('./array.json')];").unwrap();
    fs::write(dir.join("top.json"), r#"{"__proto__":{"marker":7},"plain":1}"#).unwrap();
    fs::write(dir.join("nested.json"), r#"{"outer":{"__proto__":"inner","value":2}}"#).unwrap();
    fs::write(dir.join("scalar.json"), "42").unwrap();
    fs::write(dir.join("array.json"), r#"[1,{"__proto__":"item"}]"#).unwrap();
    let modules = temp_registry("bundle_json_proto_keys_modules");
    let (bundle, _, count, _) = bundle_commonjs_package(&modules, "json-pkg", &dir, "index.js").unwrap();
    assert_eq!(count, 5);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.jsonModuleCheck = function() {{ var values = module.exports, top = values[0], nested = values[1].outer, array = values[3]; return [Object.prototype.hasOwnProperty.call(top, '__proto__'), Object.getPrototypeOf(top) === Object.prototype, Object.keys(top).indexOf('__proto__') >= 0, top['__proto__'].marker, Object.prototype.hasOwnProperty.call(nested, '__proto__'), Object.getPrototypeOf(nested) === Object.prototype, nested.value, nested['__proto__'], values[2], Array.isArray(array), array[0], Object.prototype.hasOwnProperty.call(array[1], '__proto__'), array[1]['__proto__']]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"jsonModuleCheck".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[true,true,true,7,true,true,2,\"inner\",42,true,1,true,\"item\"]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(modules);
}

/// The exact shape found in a real npm package (`qs`): `main` requires
/// two sibling files by relative path, each with no further requires
/// of their own.
#[test]
fn bundles_a_multi_file_package_reachable_from_main() {
    let dir = temp_registry("bundle_multi_file");
    fs::create_dir_all(dir.join("lib")).unwrap();
    fs::write(
        dir.join("lib/index.js"),
        "var parse = require('./parse');\nvar stringify = require('./stringify');\n\
             module.exports = { parse: parse, stringify: stringify };",
    )
    .unwrap();
    fs::write(
        dir.join("lib/parse.js"),
        "module.exports = function parse(s) { return s; };",
    )
    .unwrap();
    fs::write(
        dir.join("lib/stringify.js"),
        "module.exports = function stringify(s) { return s; };",
    )
    .unwrap();

    let empty_node_modules = temp_registry("bundle_multi_file_node_modules");
    let (bundle, main_key, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "lib/index.js").unwrap();
    assert_eq!(main_key, "pkg/lib/index.js");
    assert_eq!(file_count, 3, "main + parse.js + stringify.js");
    assert!(bundle.contains("pkg/lib/parse.js"));
    assert!(bundle.contains("pkg/lib/stringify.js"));

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn bundles_package_imports_from_a_generated_nested_package_scope() {
    let root = temp_registry("bundle_generated_scope");
    let node_modules = root.join("node_modules");
    let package = node_modules.join("@scope/client");
    let generated = node_modules.join(".generated/client");
    fs::create_dir_all(&package).unwrap();
    fs::create_dir_all(&generated).unwrap();
    fs::write(
        package.join("default.js"),
        "module.exports = require('.generated/client/default');",
    )
    .unwrap();
    fs::write(
        generated.join("package.json"),
        r##"{"imports":{"#main":{"require":{"node":"./index.js"}}}}"##,
    )
    .unwrap();
    fs::write(
        generated.join("default.js"),
        "module.exports = require('#main');",
    )
    .unwrap();
    fs::write(generated.join("index.js"), "module.exports = 42;").unwrap();

    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "@scope/client", &package, "default.js").unwrap();
    assert_eq!(file_count, 3);
    assert!(bundle.contains(".generated/client/index.js"), "{bundle}");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn bundled_modules_receive_node_filename_and_dirname() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("bundle_module_paths");
    fs::create_dir_all(dir.join("lib")).unwrap();
    fs::write(
            dir.join("lib/index.js"),
            "var child = require('./child'); module.exports = function() { return [__filename, __dirname, child]; };",
        )
        .unwrap();
    fs::write(
        dir.join("lib/child.js"),
        "module.exports = [__filename, __dirname];",
    )
    .unwrap();
    let node_modules = temp_registry("bundle_module_paths_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "lib/index.js").unwrap();
    let script = CString::new(format!(
            "globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.readModulePaths = module.exports;"
        ))
        .unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("readModulePaths").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["/thaw_modules/pkg/lib/index.js","/thaw_modules/pkg/lib",["/thaw_modules/pkg/lib/child.js","/thaw_modules/pkg/lib"]]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

/// `bundle_commonjs_package`'s version-recording half (the other half
/// of `add`'s 17章 `version.txt` work, extended to cover every
/// package the bundle actually reaches, not just the root one --
/// `AddedPackage::dependency_versions`/`lock.json`). Both the root
/// package (`pkg`) and its one real dependency (`left-pad-ish`) have
/// their own `package.json` with a `version` field here, mirroring
/// what `npm install` actually leaves on disk.
#[test]
fn bundle_commonjs_package_records_every_reached_packages_version() {
    let dir = temp_registry("bundle_versions_root");
    fs::write(
        dir.join("package.json"),
        r#"{"name": "pkg", "version": "2.5.0", "main": "index.js"}"#,
    )
    .unwrap();
    fs::write(
        dir.join("index.js"),
        "var dep = require('left-pad-ish');\nmodule.exports = dep;",
    )
    .unwrap();

    let node_modules = temp_registry("bundle_versions_node_modules");
    fs::create_dir_all(node_modules.join("left-pad-ish")).unwrap();
    fs::write(
        node_modules.join("left-pad-ish/package.json"),
        r#"{"name": "left-pad-ish", "version": "1.3.0", "main": "index.js"}"#,
    )
    .unwrap();
    fs::write(
        node_modules.join("left-pad-ish/index.js"),
        "module.exports = function () { return 'padded'; };",
    )
    .unwrap();

    let (_, _, _, dependency_versions) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();

    assert_eq!(
        dependency_versions.get("pkg").map(String::as_str),
        Some("2.5.0")
    );
    assert_eq!(
        dependency_versions.get("left-pad-ish").map(String::as_str),
        Some("1.3.0")
    );
    assert_eq!(dependency_versions.len(), 2, "no extra/missing entries");

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&node_modules);
}

/// A package with no dependencies still gets exactly one entry (its
/// own) -- `fetch_and_copy` uses this len-1 case to decide *not* to
/// write a redundant `lock.json` next to `version.txt`.
#[test]
fn bundle_commonjs_package_with_no_dependencies_records_only_itself() {
    let dir = temp_registry("bundle_versions_solo");
    fs::write(
        dir.join("package.json"),
        r#"{"name": "solo-pkg", "version": "0.1.0", "main": "index.js"}"#,
    )
    .unwrap();
    fs::write(
        dir.join("index.js"),
        "module.exports = function () { return 1; };",
    )
    .unwrap();

    let empty_node_modules = temp_registry("bundle_versions_solo_node_modules");
    let (_, _, _, dependency_versions) =
        bundle_commonjs_package(&empty_node_modules, "solo-pkg", &dir, "index.js").unwrap();

    assert_eq!(dependency_versions.len(), 1);
    assert_eq!(
        dependency_versions.get("solo-pkg").map(String::as_str),
        Some("0.1.0")
    );

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn bundle_of_a_single_file_package_still_has_one_module() {
    let dir = temp_registry("bundle_single_file");
    fs::write(
        dir.join("index.js"),
        "module.exports = function f() { return 1; };",
    )
    .unwrap();

    let empty_node_modules = temp_registry("bundle_single_file_node_modules");
    let (_, main_key, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(main_key, "pkg/index.js");
    assert_eq!(file_count, 1);

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

/// An unresolvable relative require (here: a `.json` target, which
/// `resolve_module_path`'s candidates don't cover) must not abort
/// bundling the rest of the package -- it's just left for the
/// external-require stub to report clearly if actually called.
#[test]
fn unresolvable_relative_require_does_not_abort_bundling() {
    let dir = temp_registry("bundle_unresolvable_require");
    fs::write(
        dir.join("index.js"),
        "var pkg = require('./package.json');\nmodule.exports = function f() { return 1; };",
    )
    .unwrap();
    // Deliberately no package.json written -- this require can never
    // resolve via resolve_module_path's .js/index.js candidates.

    let empty_node_modules = temp_registry("bundle_unresolvable_require_node_modules");
    let (_, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

/// The bundle isn't just plausible-looking text -- it must actually
/// run correctly through the real QuickJS-NG engine, including a
/// same-package relative require resolving to a sibling module *and*
/// a bare-specifier require resolving to a real dependency package
/// under `node_modules` (the actual new capability: `qs`'s own
/// dependency on `side-channel`, reproduced in miniature). A third,
/// genuinely external require (something not present under
/// `node_modules` at all, mirroring a Node core builtin or a
/// dependency `npm install` didn't fetch) is left inside a function
/// that's never called -- were it eager and reached, it would throw
/// immediately, same as `is-odd`'s real `require('is-number')`
/// (already covered by this session's end-to-end verification); this
/// test is specifically about what *does* resolve.
#[test]
fn bundle_actually_runs_through_quickjs() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("bundle_runs_through_quickjs");
    fs::create_dir_all(dir.join("lib")).unwrap();
    fs::write(
        dir.join("lib/index.js"),
        "var double = require('./double');\n\
             var triple = require('triple-dep');\n\
             function unused() { return require('a-package-that-was-never-installed'); }\n\
             module.exports = function run(n) { return double(triple(n)); };",
    )
    .unwrap();
    fs::write(
        dir.join("lib/double.js"),
        "module.exports = function (n) { return n * 2; };",
    )
    .unwrap();

    let node_modules_dir = temp_registry("bundle_runs_through_quickjs_node_modules");
    fs::create_dir_all(node_modules_dir.join("triple-dep")).unwrap();
    fs::write(
        node_modules_dir.join("triple-dep/package.json"),
        r#"{"main": "index.js"}"#,
    )
    .unwrap();
    fs::write(
        node_modules_dir.join("triple-dep/index.js"),
        "module.exports = function (n) { return n * 3; };",
    )
    .unwrap();

    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules_dir, "pkg", &dir, "lib/index.js").unwrap();
    assert_eq!(
        file_count, 3,
        "pkg's index.js + double.js + triple-dep's index.js"
    );

    // Same environment thaw-bridge's `wrap_as_commonjs_module` sets
    // up: global `module`/`exports`/`require` before running the
    // source, then bind the default export by name afterward.
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.run = module.exports;\n"
        );

    let source = CString::new(script).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "bundle failed to load"
    );

    let func = CString::new("run").unwrap();
    let args = CString::new("[7]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(
            result, "42",
            "relative require (double) and cross-package bare require (triple-dep) must both resolve correctly"
        );

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&node_modules_dir);
}

#[test]
fn bare_parent_directory_require_stays_inside_its_package() {
    let dir = temp_registry("bundle_parent_directory_require");
    fs::create_dir_all(dir.join("lib/nested")).unwrap();
    fs::write(dir.join("lib/index.js"), "module.exports = 42;").unwrap();
    fs::write(
        dir.join("lib/nested/child.js"),
        "module.exports = require('..');",
    )
    .unwrap();

    let node_modules_dir = temp_registry("bundle_parent_directory_node_modules");
    let (bundle, _, file_count, _) = bundle_commonjs_package(
        &node_modules_dir,
        "pkg",
        &dir,
        "lib/nested/child.js",
    )
    .unwrap();

    assert_eq!(file_count, 2);
    assert!(bundle.contains("\"..\": \"pkg/lib/index.js\""));
    assert!(!bundle.contains("\"../"));

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&node_modules_dir);
}

/// Real `--use pkg-a --use pkg-b` loads each package's bundle into the
/// *same* shared thread-local QuickJS-NG context, one `loadScript`
/// call per package (`wrap_as_commonjs_module`, called once per
/// package). Before wrapping each bundle's module-system helpers in
/// an IIFE, they were plain globals (`__thaw_bundle_cache` etc.), so
/// loading package B would silently overwrite package A's -- invisible
/// for a require resolved eagerly at load time (already finished and
/// cached by then), but package A's *lazy* internal require (deferred
/// inside a function body, only actually called after B has loaded)
/// would then resolve against B's module map instead of its own.
#[test]
fn multiple_bundled_packages_dont_stomp_each_others_module_state() {
    use std::ffi::{CStr, CString};

    let node_modules_dir = temp_registry("multi_pkg_node_modules");

    let dir_a = temp_registry("multi_pkg_a");
    fs::write(
        dir_a.join("index.js"),
        "module.exports = function getLazy() { return require('./lazy')(); };",
    )
    .unwrap();
    fs::write(
        dir_a.join("lazy.js"),
        "module.exports = function () { return 'from lazy'; };",
    )
    .unwrap();
    let (bundle_a, _, _, _) =
        bundle_commonjs_package(&node_modules_dir, "pkg-a", &dir_a, "index.js").unwrap();

    let dir_b = temp_registry("multi_pkg_b");
    fs::write(
        dir_b.join("index.js"),
        "module.exports = function () { return 'b'; };",
    )
    .unwrap();
    let (bundle_b, _, _, _) =
        bundle_commonjs_package(&node_modules_dir, "pkg-b", &dir_b, "index.js").unwrap();

    let wrap = |bundle: &str, bind_as: &str| {
        format!(
                "globalThis.module = {{ exports: {{}} }};\n\
                 globalThis.exports = globalThis.module.exports;\n\
                 globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
                 {bundle}\n\
                 globalThis.{bind_as} = module.exports;\n"
            )
    };

    let source_a = CString::new(wrap(&bundle_a, "getLazy")).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source_a.as_ptr()),
        1,
        "package A failed to load"
    );

    // Loaded into the same shared global context *after* A.
    let source_b = CString::new(wrap(&bundle_b, "pkgB")).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source_b.as_ptr()),
        1,
        "package B failed to load"
    );

    // Call A's lazily-requiring function *after* B has loaded.
    let func = CString::new("getLazy").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        result, "\"from lazy\"",
        "package A's lazy internal require must still resolve against its own module map, \
             not package B's, after B has loaded into the shared global context"
    );

    let result = thaw_quickjs::eval_json(
        "globalThis.__thaw_bundle_create_require('/tmp/main.js')('pkg-a/index.js')()",
    )
    .unwrap();
    assert_eq!(result.as_deref(), Some("\"from lazy\""));

    let _ = fs::remove_dir_all(&dir_a);
    let _ = fs::remove_dir_all(&dir_b);
    let _ = fs::remove_dir_all(&node_modules_dir);
}

#[test]
fn bundle_exports_self_contained_worker_runtime_source() {
    use std::ffi::CString;

    let node_modules_dir = temp_registry("worker_runtime_source_node_modules");
    let dir = temp_registry("worker_runtime_source");
    fs::write(
        dir.join("index.js"),
        "module.exports = function () { return require('./value').answer; };",
    )
    .unwrap();
    fs::write(
        dir.join("value.js"),
        "globalThis.workerResolveValueRuns = (globalThis.workerResolveValueRuns || 0) + 1; this.answer = 42;",
    )
    .unwrap();
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules_dir, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let source = CString::new(format!(
            "globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; globalThis.workerResolveValueRuns = 0; {bundle}"
        ))
        .unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);

    let resolved = thaw_quickjs::eval_json(
            "Function('require', __thaw_worker_bundle_source + \"return [__thaw_bundle_create_require('pkg/index.js').resolve('./value'), globalThis.workerResolveValueRuns || 0];\")(function(name) { throw new Error(name); })",
        )
        .unwrap();
    assert_eq!(resolved.as_deref(), Some("[\"pkg/value.js\",0]"));

    let result = thaw_quickjs::eval_json(
            "Function('require', __thaw_worker_bundle_source + \"return __thaw_bundle_create_require('pkg/index.js')('./value').answer;\")(function(name) { throw new Error(name); })",
        )
        .unwrap();
    assert_eq!(result.as_deref(), Some("42"));

    // The serialized origin resolver must carry its factory-key classifier.
    // This lookup runs inside the detached Worker source, where outer bundle
    // helper declarations are not available.
    let origin = thaw_quickjs::eval_json(
            "Function('require', __thaw_worker_bundle_source + \"return __thaw_bundle_worker_origin('pkg/index.js').resolve('missing') === null;\")(function(name) { throw new Error(name); })",
        )
        .unwrap();
    assert_eq!(origin.as_deref(), Some("true"));

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules_dir);
}

/// The exact shape found in `qs`'s real dependency chain:
/// `object-inspect` (pulled in transitively) does
/// `require('util').inspect.custom` unconditionally at load time,
/// with no matching `node_modules/util` -- must resolve via the
/// `util` builtin polyfill instead of falling through to the
/// external-require stub.
#[test]
fn bundles_the_util_builtin_polyfill_when_required() {
    let dir = temp_registry("builtin_util");
    fs::write(
        dir.join("index.js"),
        "var inspect = require('util').inspect;\n\
             module.exports = function () { return typeof inspect.custom; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_util_node_modules");

    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2, "pkg's index.js + the util polyfill");
    assert!(bundle.contains("node:util"));

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}
#[test]
fn bundled_bindings_require_uses_the_loaded_addon() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("bundle_bindings_addon");
    fs::write(
        dir.join("index.js"),
        "module.exports = require('bindings')('native.node');",
    )
    .unwrap();
    let empty_node_modules = temp_registry("bundle_bindings_addon_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!(
        "globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; globalThis.require.addon = function() {{ return {{ answer: 42 }}; }}; {bundle} globalThis.readAddon = function() {{ return module.exports.answer; }};"
    );
    assert_eq!(
        thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()),
        1
    );
    let result = thaw_quickjs::thaw_js_call(
        CString::new("readAddon").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(empty_node_modules);
}

#[test]
fn delayed_native_require_keeps_each_bundles_addon() {
    use std::ffi::{CStr, CString};

    let first = temp_registry("bundle_delayed_first");
    let second = temp_registry("bundle_delayed_second");
    let empty_node_modules = temp_registry("bundle_delayed_node_modules");
    let source = "var nodeProcess = require('node:process'), nodeModule = require('node:module'); module.exports = function() { var target = { exports: {}, filename: __filename }, builtinTarget = { exports: {}, filename: __filename }; process.dlopen(target, __dirname + '/native.node'); nodeProcess.dlopen(builtinTarget, __dirname + '/native.node'); process.exitCode = target.exports.answer; process.title = 'pkg' + target.exports.answer; process.argv = [String(target.exports.answer)]; Object.defineProperty(process, 'phase', {value: target.exports.answer, configurable: true}); delete process.marker; return [require('bindings')('native.node').answer, target.exports.answer, builtinTarget.exports.answer, nodeModule.createRequire(__filename)('bindings')('native.node').answer, nodeProcess === process, process === globalThis.process]; };";
    fs::write(first.join("index.js"), source).unwrap();
    fs::write(second.join("index.js"), source).unwrap();
    let (first_bundle, _, first_count, _) =
        bundle_commonjs_package(&empty_node_modules, "first-pkg", &first, "index.js").unwrap();
    let (second_bundle, _, second_count, _) =
        bundle_commonjs_package(&empty_node_modules, "second-pkg", &second, "index.js").unwrap();
    assert_eq!(first_count, 3, "entry plus process and module builtins");
    assert_eq!(second_count, 3, "entry plus process and module builtins");
    let script = format!(
        "globalThis.__thaw_saved_process = globalThis.process; \
         globalThis.process = {{ marker: true, dlopen: function(target) {{ target.exports = {{ answer: target.filename.indexOf('/first-pkg/') >= 0 ? 11 : 22 }}; return target.exports; }} }}; \
         globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; \
         globalThis.require = function(name) {{ throw new Error(name); }}; \
         globalThis.require.addon = function() {{ return {{ answer: 11 }}; }}; \
         {first_bundle} globalThis.firstDelayed = module.exports; \
         \
         globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; \
         globalThis.require = function(name) {{ throw new Error(name); }}; \
         globalThis.require.addon = function() {{ return {{ answer: 22 }}; }}; \
         {second_bundle} globalThis.secondDelayed = module.exports; \
         globalThis.delayedAddonValues = function() {{ var first = firstDelayed(), firstState = [process.exitCode, process.title, process.argv[0], process.phase, 'marker' in process], second = secondDelayed(), secondState = [process.exitCode, process.title, process.argv[0], process.phase, 'marker' in process]; globalThis.process = globalThis.__thaw_saved_process; delete globalThis.__thaw_saved_process; return [first, firstState, second, secondState]; }};"
    );
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"delayedAddonValues".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[[11,11,11,11,true,true],[11,\"pkg11\",\"11\",11,false],[22,22,22,22,true,true],[22,\"pkg22\",\"22\",22,false]]");
    let _ = fs::remove_dir_all(first);
    let _ = fs::remove_dir_all(second);
    let _ = fs::remove_dir_all(empty_node_modules);
}

#[test]
fn bundled_node_gyp_build_uses_the_loaded_addon() {
    use std::ffi::{CStr, CString};

    let node_modules = temp_registry("bundle_node_gyp_addon_node_modules");
    let package = node_modules.join("pkg");
    let locator = node_modules.join("node-gyp-build");
    fs::create_dir_all(&package).unwrap();
    fs::create_dir_all(&locator).unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = require('node-gyp-build')(__dirname);",
    )
    .unwrap();
    fs::write(
        locator.join("package.json"),
        r#"{"name":"node-gyp-build","main":"index.js"}"#,
    )
    .unwrap();
    fs::write(locator.join("index.js"), "throw new Error('locator ran');").unwrap();
    let (bundle, _, _, _) =
        bundle_commonjs_package(&node_modules, "pkg", &package, "index.js").unwrap();
    let script = format!(
        "globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; globalThis.require.addon = function() {{ return {{ answer: 42 }}; }}; {bundle} globalThis.readAddon = function() {{ return module.exports.answer; }};"
    );
    assert_eq!(
        thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()),
        1
    );
    let result = thaw_quickjs::thaw_js_call(
        CString::new("readAddon").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn bundled_direct_node_require_uses_the_loaded_addon() {
    use std::ffi::{CStr, CString};

    let node_modules = temp_registry("bundle_direct_addon_node_modules");
    let package = node_modules.join("pkg");
    let native = node_modules.join("@vendor/native");
    fs::create_dir_all(&package).unwrap();
    fs::create_dir_all(&native).unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = require('@vendor/native/addon.node');",
    )
    .unwrap();
    fs::write(
        native.join("package.json"),
        r#"{"name":"@vendor/native","main":"addon.node"}"#,
    )
    .unwrap();
    fs::write(native.join("addon.node"), b"not JavaScript").unwrap();

    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &package, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!(
        "globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; globalThis.require.addon = function() {{ return {{ answer: 42 }}; }}; {bundle} globalThis.readAddon = function() {{ return module.exports.answer; }};"
    );
    assert_eq!(
        thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()),
        1
    );
    let result = thaw_quickjs::thaw_js_call(
        CString::new("readAddon").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");
    let _ = fs::remove_dir_all(node_modules);
}

/// A dependency subpath whose `exports` offers `{ default, node, import }`
/// but no `require` -- real-world example: `@babel/runtime/helpers/extends`
/// (`{"node":"./helpers/extends.js","import":"./helpers/esm/extends.js",
/// "default":"./helpers/extends.js"}`). The bundler is CommonJS, so it must
/// pick the CJS `node`/`default` file; preferring `import` bundled the ESM
/// source as if it were commonjs, and mathjs died at load time with
/// "not a function" (`_interopRequireDefault(esm).default`).
#[test]
fn a_subpath_export_prefers_commonjs_over_an_esm_import_condition() {
    let root = temp_registry("bundle_subpath_condition");
    let node_modules = root.join("node_modules");
    let package = node_modules.join("uses-runtime");
    let dependency = node_modules.join("dual-runtime");
    fs::create_dir_all(&package).unwrap();
    fs::create_dir_all(&dependency).unwrap();
    fs::write(package.join("package.json"), r#"{"main":"index.js"}"#).unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = require('dual-runtime/helper');",
    )
    .unwrap();
    fs::write(
        dependency.join("package.json"),
        r#"{"exports":{"./helper":{"default":"./default.cjs","node":"./node.cjs","import":"./esm.mjs"}}}"#,
    )
    .unwrap();
    fs::write(dependency.join("default.cjs"), "module.exports = 7;").unwrap();
    fs::write(dependency.join("node.cjs"), "module.exports = 8;").unwrap();
    fs::write(dependency.join("esm.mjs"), "export default 99;").unwrap();

    let (bundle, _, _, _) =
        bundle_commonjs_package(&node_modules, "uses-runtime", &package, "index.js").unwrap();
    assert!(bundle.contains("default.cjs"), "{bundle}");
    assert!(!bundle.contains("node.cjs"), "{bundle}");
    assert!(
        !bundle.contains("esm.mjs"),
        "the ESM condition must not be bundled for a CommonJS require:\n{bundle}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn bundles_transitive_bare_esm_imports() {
    let root = temp_registry("bundle_transitive_bare_esm_imports");
    let node_modules = root.join("node_modules");
    let package = node_modules.join("runner");
    let middle = node_modules.join("npm-run-path");
    let leaf = node_modules.join("unicorn-magic");
    for directory in [&package, &middle, &leaf] {
        fs::create_dir_all(directory).unwrap();
        fs::write(
            directory.join("package.json"),
            r#"{"type":"module","exports":{"default":"./index.js"}}"#,
        )
        .unwrap();
    }
    fs::write(
        leaf.join("package.json"),
        r#"{"type":"module","exports":{"node":{"import":"./index.js"}}}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "import { npmRunPath } from 'npm-run-path'; export default npmRunPath();",
    )
    .unwrap();
    fs::write(
        middle.join("index.js"),
        "import { toPath } from 'unicorn-magic'; export const npmRunPath = () => toPath('ok');",
    )
    .unwrap();
    fs::write(
        leaf.join("index.js"),
        "export const toPath = value => value;",
    )
    .unwrap();

    let (bundle, _, _, _) =
        bundle_commonjs_package(&node_modules, "runner", &package, "index.js").unwrap();
    assert!(bundle.contains("unicorn-magic/./index.js"), "{bundle}");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn package_imports_use_import_conditions_for_static_and_dynamic_calls() {
    let root = temp_registry("imports-conditions");
    let modules = root.join("node_modules");
    let package = modules.join("condition-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"), r##"{"name":"condition-kit","imports":{"#exact":{"import":"./esm.js","require":"./cjs.js"},"#feature/*":{"import":"./esm/*.js","require":"./cjs/*.js"}}}"##).unwrap();
    fs::create_dir_all(package.join("esm")).unwrap();
    fs::create_dir_all(package.join("cjs")).unwrap();
    fs::write(package.join("entry.js"), "import value from '#exact'; export default import('#feature/child');").unwrap();
    fs::write(package.join("esm.js"), "export default 1;").unwrap();
    fs::write(package.join("cjs.js"), "module.exports = 2;").unwrap();
    fs::write(package.join("esm/child.js"), "export default 3;").unwrap();
    fs::write(package.join("cjs/child.js"), "module.exports = 4;").unwrap();
    let (bundle, _, file_count, _) = bundle_commonjs_package(&modules, "condition-kit", &package, "entry.js").unwrap();
    assert_eq!(file_count, 3, "entry plus import-condition exact and wildcard files");
    assert!(bundle.contains("condition-kit/esm.js"));
    assert!(bundle.contains("condition-kit/esm/child.js"));
    assert!(!bundle.contains("condition-kit/cjs.js"));
    assert!(!bundle.contains("condition-kit/cjs/child.js"));
    fs::write(package.join("entry.js"), "module.exports = [require('#exact'), require('#feature/child')];").unwrap();
    let (bundle, _, file_count, _) = bundle_commonjs_package(&modules, "condition-kit", &package, "entry.js").unwrap();
    assert_eq!(file_count, 3);
    assert!(bundle.contains("condition-kit/cjs.js"));
    assert!(bundle.contains("condition-kit/cjs/child.js"));
    assert!(!bundle.contains("condition-kit/esm.js"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn same_specifier_keeps_import_and_require_condition_targets() {
    let root = temp_registry("mixed-condition-specifier");
    let modules = root.join("node_modules");
    let owner = modules.join("owner");
    let dual = modules.join("dual");
    fs::create_dir_all(&owner).unwrap();
    fs::create_dir_all(&dual).unwrap();
    fs::write(dual.join("package.json"), r#"{"name":"dual","exports":{".":{"import":"./esm.js","require":"./cjs.js"}}}"#).unwrap();
    fs::write(dual.join("esm.js"), "export default 'import';").unwrap();
    fs::write(dual.join("cjs.js"), "module.exports = 'require';").unwrap();
    fs::write(owner.join("entry.js"), "import value from 'dual'; const sync = require('dual'); export default [value, sync, import('dual')];").unwrap();

    let (bundle, _, file_count, _) = bundle_commonjs_package(&modules, "owner", &owner, "entry.js").unwrap();
    assert_eq!(file_count, 3);
    let import_maps = bundle.split("var __thaw_bundle_import_maps = {").nth(1).unwrap().split("};").next().unwrap();
    let require_maps = bundle.split("var __thaw_bundle_require_maps = {").nth(1).unwrap().split("};").next().unwrap();
    assert!(import_maps.contains("\"dual\": \"dual/./esm.js\""), "{import_maps}");
    assert!(require_maps.contains("\"dual\": \"dual/./cjs.js\""), "{require_maps}");
    assert!(bundle.contains("__thaw_bundle_target(importMap, spec)"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn package_import_same_specifier_and_nonliteral_fallback_keep_conditions() {
    let root = temp_registry("mixed-package-import");
    let modules = root.join("node_modules");
    let owner = modules.join("owner");
    let dual = modules.join("dual");
    fs::create_dir_all(&owner).unwrap();
    fs::create_dir_all(&dual).unwrap();
    fs::write(owner.join("package.json"), r##"{"name":"owner","dependencies":{"dual":"1.0.0"},"imports":{"#choice":{"import":"./esm.js","require":"./cjs.js"}}}"##).unwrap();
    fs::write(owner.join("esm.js"), "export default 'import';").unwrap();
    fs::write(owner.join("cjs.js"), "module.exports = 'require';").unwrap();
    fs::write(dual.join("package.json"), r#"{"name":"dual","exports":{".":{"import":"./esm.js","require":"./cjs.js"}}}"#).unwrap();
    fs::write(dual.join("esm.js"), "export default 'import';").unwrap();
    fs::write(dual.join("cjs.js"), "module.exports = 'require';").unwrap();
    fs::write(owner.join("entry.js"), "import value from '#choice'; const sync = require('#choice'); const dynamic = import('#choice'); const name = 'dual'; module.exports = [value, sync, dynamic, require(name)];").unwrap();

    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "owner", &owner, "entry.js").unwrap();
    let import_maps = bundle.split("var __thaw_bundle_import_maps = {").nth(1).unwrap().split("};").next().unwrap();
    let require_maps = bundle.split("var __thaw_bundle_require_maps = {").nth(1).unwrap().split("};").next().unwrap();
    assert!(import_maps.contains("\"#choice\": \"owner/esm.js\""), "{import_maps}");
    assert!(require_maps.contains("\"#choice\": \"owner/cjs.js\""), "{require_maps}");
    assert!(import_maps.contains("\"dual\": \"dual/./esm.js\""), "{import_maps}");
    assert!(require_maps.contains("\"dual\": \"dual/./cjs.js\""), "{require_maps}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn nonliteral_import_can_use_subpath_without_import_condition_at_package_root() {
    let root = temp_registry("nonliteral-import-subpath");
    let modules = root.join("node_modules");
    let owner = modules.join("owner");
    let dual = modules.join("dual");
    fs::create_dir_all(&owner).unwrap();
    fs::create_dir_all(&dual).unwrap();
    fs::write(owner.join("package.json"), r#"{"name":"owner","dependencies":{"dual":"1.0.0"}}"#).unwrap();
    fs::write(owner.join("entry.js"), "const name = 'dual/feature'; module.exports = import(name);").unwrap();
    fs::write(dual.join("package.json"), r#"{"name":"dual","exports":{".":{"require":"./cjs.js"},"./feature":{"import":"./esm.js"}}}"#).unwrap();
    fs::write(dual.join("cjs.js"), "module.exports = 'require';").unwrap();
    fs::write(dual.join("esm.js"), "export default 'import';").unwrap();

    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "owner", &owner, "entry.js").unwrap();
    let import_maps = bundle.split("var __thaw_bundle_import_maps = {").nth(1).unwrap().split("};").next().unwrap();
    assert!(import_maps.contains("\"dual/feature\": \"dual/./esm.js\""), "{import_maps}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_import_condition_cannot_fall_back_to_require_target() {
    let root = temp_registry("require-only-import");
    let modules = root.join("node_modules");
    let owner = modules.join("owner");
    let dual = modules.join("dual");
    fs::create_dir_all(&owner).unwrap();
    fs::create_dir_all(&dual).unwrap();
    fs::write(dual.join("package.json"), r#"{"name":"dual","exports":{".":{"require":"./cjs.js"}}}"#).unwrap();
    fs::write(dual.join("cjs.js"), "module.exports = 'require';").unwrap();
    fs::write(owner.join("entry.js"), "module.exports = [require('dual'), import('dual')];").unwrap();

    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "owner", &owner, "entry.js").unwrap();
    let import_maps = bundle.split("var __thaw_bundle_import_maps = {").nth(1).unwrap().split("};").next().unwrap();
    let require_maps = bundle.split("var __thaw_bundle_require_maps = {").nth(1).unwrap().split("};").next().unwrap();
    let known = bundle.split("var __thaw_bundle_known_package_maps = {").nth(1).unwrap().split("};").next().unwrap();
    assert!(!import_maps.contains("\"dual\":"), "{import_maps}");
    assert!(require_maps.contains("\"dual\": \"dual/./cjs.js\""), "{require_maps}");
    assert!(known.contains("\"owner/entry.js\": [\"dual\"]"), "{known}");
    assert!(bundle.contains("if (__thaw_bundle_import_missing(knownPackages, spec)) throw new Error"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn two_nested_copies_of_one_package_keep_distinct_factories_and_versions() {
    let root = temp_registry("nested-two-copies");
    let modules = root.join("node_modules");
    let app = modules.join("app");
    let left = modules.join("left");
    let right = modules.join("right");
    let left_shared = left.join("node_modules/shared");
    let right_shared = right.join("node_modules/shared");
    for directory in [&app, &left, &right, &left_shared, &right_shared] {
        fs::create_dir_all(directory).unwrap();
    }
    fs::write(app.join("package.json"), r#"{"name":"app","version":"1.0.0"}"#).unwrap();
    fs::write(app.join("index.js"), "module.exports = [require('left'), require('right')];").unwrap();
    fs::write(left.join("package.json"), r#"{"name":"left","version":"1.0.0"}"#).unwrap();
    fs::write(right.join("package.json"), r#"{"name":"right","version":"1.0.0"}"#).unwrap();
    fs::write(left.join("index.js"), "module.exports = require('shared');").unwrap();
    fs::write(right.join("index.js"), "module.exports = require('shared');").unwrap();
    fs::write(left_shared.join("package.json"), r#"{"name":"shared","version":"1.0.0"}"#).unwrap();
    fs::write(right_shared.join("package.json"), r#"{"name":"shared","version":"2.0.0"}"#).unwrap();
    fs::write(left_shared.join("index.js"), "module.exports = 'left-v1';").unwrap();
    fs::write(right_shared.join("index.js"), "module.exports = 'right-v2';").unwrap();

    let (bundle, main, file_count, versions) = bundle_commonjs_package(&modules, "app", &app, "index.js").unwrap();
    assert_eq!(main, "app/index.js");
    assert_eq!(file_count, 5);
    assert!(bundle.contains("\"left/node_modules/shared/index.js\""));
    assert!(bundle.contains("\"right/node_modules/shared/index.js\""));
    assert!(bundle.contains("\"shared\": \"left/node_modules/shared/index.js\""));
    assert!(bundle.contains("\"shared\": \"right/node_modules/shared/index.js\""));
    assert_eq!(versions.get("left/node_modules/shared").map(String::as_str), Some("1.0.0"));
    assert_eq!(versions.get("right/node_modules/shared").map(String::as_str), Some("2.0.0"));
    assert_eq!(versions.get("app").map(String::as_str), Some("1.0.0"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bare_requests_use_calling_file_but_package_import_targets_use_manifest_root() {
    let root = temp_registry("file-origin-dependency");
    let modules = root.join("node_modules");
    let app = modules.join("app");
    let near = app.join("lib/node_modules/dep");
    let far = app.join("node_modules/dep");
    for directory in [&near, &far] { fs::create_dir_all(directory).unwrap(); }
    fs::write(app.join("package.json"), r##"{"name":"app","version":"1.0.0","dependencies":{"dep":"*"},"imports":{"#dep":"dep"}}"##).unwrap();
    fs::write(app.join("lib/entry.js"), "const name = 'dep'; module.exports = [require('dep'), require('#dep'), import('dep'), require(name)];").unwrap();
    fs::write(near.join("package.json"), r#"{"name":"dep","version":"3.0.0"}"#).unwrap();
    fs::write(far.join("package.json"), r#"{"name":"dep","version":"2.0.0"}"#).unwrap();
    fs::write(near.join("index.js"), "module.exports = 'near-v3';").unwrap();
    fs::write(far.join("index.js"), "module.exports = 'far-v2';").unwrap();

    let (bundle, _, file_count, versions) = bundle_commonjs_package(&modules, "app", &app, "lib/entry.js").unwrap();
    assert_eq!(file_count, 3);
    assert!(bundle.contains("\"dep\": \"app/lib/node_modules/dep/index.js\""), "{bundle}");
    assert!(bundle.contains("\"#dep\": \"app/node_modules/dep/index.js\""), "{bundle}");
    assert!(bundle.contains("near-v3"));
    assert!(bundle.contains("far-v2"));
    assert_eq!(versions.get("app/lib/node_modules/dep").map(String::as_str), Some("3.0.0"));
    assert_eq!(versions.get("app/node_modules/dep").map(String::as_str), Some("2.0.0"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn create_require_uses_the_longest_nested_instance_owner() {
    use std::ffi::CString;
    let root = temp_registry("nested-create-require-owner");
    let modules = root.join("node_modules");
    let app = modules.join("app");
    let left = modules.join("left");
    let top = modules.join("shared");
    let nested = left.join("node_modules/shared");
    for directory in [&app, &left, &top, &nested] { fs::create_dir_all(directory).unwrap(); }
    fs::write(app.join("index.js"), "module.exports = [require('left'), require('shared')];").unwrap();
    fs::write(left.join("package.json"), r#"{"name":"left","main":"index.js"}"#).unwrap();
    fs::write(left.join("index.js"), "module.exports = require('shared');").unwrap();
    fs::write(top.join("package.json"), r#"{"name":"shared","main":"index.js"}"#).unwrap();
    fs::write(nested.join("package.json"), r#"{"name":"shared","main":"index.js"}"#).unwrap();
    fs::write(top.join("index.js"), "module.exports = require('./other');").unwrap();
    fs::write(nested.join("index.js"), "module.exports = require('./other');").unwrap();
    fs::write(top.join("other.js"), "module.exports = 'top';").unwrap();
    fs::write(nested.join("other.js"), "module.exports = 'nested';").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "app", &app, "index.js").unwrap();
    let source = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle}")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result = thaw_quickjs::eval_json("__thaw_bundle_create_require('/thaw_modules/left/node_modules/shared/index.js').resolve('./other')").unwrap();
    assert_eq!(result.as_deref(), Some("\"left/node_modules/shared/other.js\""));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn nested_worker_bootstrap_uses_its_package_instance_key() {
    let root = temp_registry("nested-worker-key");
    let modules = root.join("node_modules");
    let app = modules.join("app");
    let child = app.join("node_modules/child");
    fs::create_dir_all(&child).unwrap();
    fs::write(app.join("index.js"), "module.exports = require('child');").unwrap();
    fs::write(child.join("package.json"), r#"{"name":"child","main":"index.js"}"#).unwrap();
    fs::write(child.join("index.js"), "var Worker = require('node:worker_threads').Worker; module.exports = new Worker(new URL('./worker.js', import.meta.url));").unwrap();
    fs::write(child.join("worker.js"), "module.exports = 7;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "app", &app, "index.js").unwrap();
    let worker_key = "app/node_modules/child/worker.js";
    let encoded_key = worker_key.bytes().map(|byte| format!("%{byte:02X}")).collect::<String>();
    assert!(bundle.contains(&format!("\"{worker_key}\"")));
    assert!(bundle.contains(&encoded_key), "Worker bootstrap must use its factory key");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_root_alias_collision_reports_both_files() {
    let root = temp_registry("external-root-key-collision");
    let modules = root.join("node_modules");
    let outside = root.join("outside-dep");
    let installed = modules.join("dep");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(&installed).unwrap();
    fs::write(outside.join("index.js"), "module.exports = require('dep');").unwrap();
    fs::write(installed.join("package.json"), r#"{"name":"dep","main":"index.js"}"#).unwrap();
    fs::write(installed.join("index.js"), "module.exports = 2;").unwrap();
    let error = bundle_commonjs_package(&modules, "dep", &outside, "index.js").unwrap_err();
    assert!(error.contains("bundle module key `dep/index.js` names both"), "{error}");
    assert!(error.contains("outside-dep/index.js"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn shared_source_cache_projects_versions_after_all_subpath_bundles() {
    let root = temp_registry("subpaths-two-nested-versions");
    let modules = root.join("node_modules");
    let app = modules.join("app");
    let left = modules.join("left");
    let right = modules.join("right");
    let left_shared = left.join("node_modules/shared");
    let right_shared = right.join("node_modules/shared");
    for directory in [&app, &left, &right, &left_shared, &right_shared] { fs::create_dir_all(directory).unwrap(); }
    fs::write(app.join("package.json"), r#"{"name":"app","version":"1.0.0"}"#).unwrap();
    fs::write(app.join("main.js"), "module.exports = require('left');").unwrap();
    fs::write(app.join("subpath.js"), "module.exports = require('right');").unwrap();
    for (owner, name, version) in [(&left, "left", "1.0.0"), (&right, "right", "2.0.0")] {
        fs::write(owner.join("package.json"), format!(r#"{{"name":"{name}","main":"index.js"}}"#)).unwrap();
        fs::write(owner.join("index.js"), "module.exports = require('shared');").unwrap();
        let shared = owner.join("node_modules/shared");
        fs::write(shared.join("package.json"), format!(r#"{{"name":"shared","version":"{version}"}}"#)).unwrap();
        fs::write(shared.join("index.js"), "module.exports = 1;").unwrap();
    }
    let mut cache = SourceCache::default();
    let (_, _, _, main_versions) = bundle_commonjs_package_cached(&modules, "app", &app, "main.js", &mut cache).unwrap();
    assert_eq!(main_versions.get("shared").map(String::as_str), Some("1.0.0"));
    bundle_commonjs_package_cached(&modules, "app", &app, "subpath.js", &mut cache).unwrap();
    let versions = project_package_versions(cache.package_versions.clone()).unwrap();
    assert_eq!(versions.get("left/node_modules/shared").map(String::as_str), Some("1.0.0"));
    assert_eq!(versions.get("right/node_modules/shared").map(String::as_str), Some("2.0.0"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn source_cache_separates_the_same_file_by_package_owner() {
    let root = temp_registry("source-cache-owner");
    let modules = root.join("node_modules");
    let app = modules.join("app");
    let child = app.join("node_modules/child");
    fs::create_dir_all(&child).unwrap();
    fs::write(child.join("package.json"), r#"{"name":"child","main":"index.js"}"#).unwrap();
    fs::write(child.join("index.js"), "var Worker = require('node:worker_threads').Worker; module.exports = new Worker(new URL('./worker.js', import.meta.url));").unwrap();
    fs::write(child.join("worker.js"), "module.exports = 1;").unwrap();
    let mut cache = SourceCache::default();
    bundle_commonjs_package_cached(&modules, "app", &app, "node_modules/child/index.js", &mut cache).unwrap();
    bundle_commonjs_package_cached(&modules, "child", &child, "index.js", &mut cache).unwrap();
    let file = child.join("index.js");
    assert!(cache.modules.contains_key(&(file.clone(), "app".to_string())));
    assert!(cache.modules.contains_key(&(file, "app/node_modules/child".to_string())));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn package_imports_external_targets_use_nested_dependency_and_selected_pattern() {
    let root = temp_registry("imports-external");
    let modules = root.join("node_modules");
    let package = modules.join("owner");
    let nested = package.join("node_modules/dependency");
    let shadow = package.join("lib/node_modules/dependency");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(&shadow).unwrap();
    fs::write(package.join("package.json"), r##"{"name":"owner","imports":{"#dep/*":"dependency/*","#dep/private/*":null,"#dep/public/*":"dependency/public/*","#builtin":"fs"}}"##).unwrap();
    fs::write(package.join("lib/entry.js"), "module.exports = require('#dep/public/feature');").unwrap();
    fs::write(nested.join("package.json"), r#"{"name":"dependency","version":"2.0.0"}"#).unwrap();
    fs::create_dir_all(nested.join("public")).unwrap();
    fs::write(nested.join("public/feature.js"), "module.exports = 7;").unwrap();
    fs::write(shadow.join("package.json"), r#"{"name":"dependency","version":"9.0.0"}"#).unwrap();
    fs::create_dir_all(shadow.join("public")).unwrap();
    fs::write(shadow.join("public/feature.js"), "module.exports = 9;").unwrap();
    let (bundle, _, file_count, versions) = bundle_commonjs_package(&modules, "owner", &package, "lib/entry.js").unwrap();
    assert_eq!(file_count, 2);
    assert!(bundle.contains("owner/node_modules/dependency/public/feature.js"));
    assert_eq!(versions.get("dependency").map(String::as_str), Some("2.0.0"));
    assert!(!bundle.contains("module.exports = 9"), "external imports target must resolve from the defining package root");
    assert!(resolve_package_import(&modules, &package.join("lib/entry.js"), "#dep/private/hidden", &["require", "node", "default"]).is_none());
    assert!(matches!(resolve_package_import(&modules, &package.join("lib/entry.js"), "#builtin", &["require", "node", "default"]), Some(PackageImportResolution::Builtin(name)) if name == "fs"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_diamond_tracks_terminal_binding_and_live_value() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-diamond-origin");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './a.js'; export * from './b.js';").unwrap();
    fs::write(package.join("a.js"), "export * from './leaf.js';").unwrap();
    fs::write(package.join("b.js"), "export * from './leaf.js';").unwrap();
    fs::write(package.join("leaf.js"), "export let value = 1; export function bump() { value++; }").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ var old = module.exports.value; module.exports.bump(); return [old, module.exports.value, Object.keys(module.exports).filter(function(key) {{ return key === 'value'; }}).length, Object.getOwnPropertyDescriptor(module.exports, 'value').configurable, delete module.exports.value]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[1,2,1,false,false]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_equal_values_from_distinct_bindings_are_ambiguous() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-ambiguous-origin");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './a.js'; export * from './b.js';").unwrap();
    fs::write(package.join("a.js"), "export const value = 1;").unwrap();
    fs::write(package.join("b.js"), "export const value = 1;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ return [Object.prototype.hasOwnProperty.call(module.exports, 'value'), Object.keys(module.exports).indexOf('value')]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[false,-1]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_cycle_discovers_late_leaf_name() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-cycle-origin");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './b.js'; export * from './c.js';").unwrap();
    fs::write(package.join("b.js"), "export * from './index.js';").unwrap();
    fs::write(package.join("c.js"), "export const x = 7;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} var cycleB = globalThis.__thaw_bundle_create_require('origin-pkg/index.js')('./b.js'); globalThis.starResult = function() {{ return [module.exports.x, Object.prototype.hasOwnProperty.call(module.exports, 'x'), cycleB.x, Object.prototype.hasOwnProperty.call(cycleB, 'x')]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[7,true,7,true]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_cycle_exposes_static_name_before_late_module_evaluates() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-cycle-mid-evaluation");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './b.js'; export * from './c.js';").unwrap();
    fs::write(package.join("b.js"), "import * as root from './index.js'; export * from './index.js'; globalThis.midCycle = ['x' in root, 'x' in module.exports, globalThis.events.join(',')];").unwrap();
    fs::write(package.join("c.js"), "globalThis.events.push('C'); export const x = 7;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; globalThis.events = []; {bundle} globalThis.starResult = function() {{ return [midCycle[0], midCycle[1], midCycle[2], events.join(','), module.exports.x]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[true,true,\"\",\"C\",7]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_cycle_keeps_hoisted_function_readable() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-cycle-hoisted-function");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './b.js'; export * from './c.js';").unwrap();
    fs::write(package.join("b.js"), "import * as root from './index.js'; export * from './index.js'; globalThis.midFunction = typeof root.f;").unwrap();
    fs::write(package.join("c.js"), "export function f() { return 7; }").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ return [midFunction, module.exports.f()]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[\"function\",7]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_static_name_is_not_blocked_by_unloaded_named_native_reexport() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-mixed-named-native-cycle");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './b.js'; export * from './c.js'; export { v } from './native.node';").unwrap();
    fs::write(package.join("b.js"), "import * as root from './index.js'; export * from './index.js'; globalThis.midNative = ['x' in root, globalThis.events.join(',')]; try { module.exports.v; } catch (error) { globalThis.midNativeRead = error.name; }").unwrap();
    fs::write(package.join("c.js"), "globalThis.events.push('C'); export const x = 7;").unwrap();
    fs::write(package.join("native.node"), "").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.events = []; globalThis.require = function(name) {{ throw new Error(name); }}; globalThis.require.addon = function() {{ events.push('N'); return {{ v: 9 }}; }}; {bundle} var cycleB = globalThis.__thaw_bundle_create_require('origin-pkg/index.js')('./b.js'); globalThis.starResult = function() {{ return [midNative[0], midNative[1], midNativeRead, events.join(','), module.exports.x, module.exports.v, cycleB.v]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[true,\"\",\"ReferenceError\",\"C,N\",7,9,9]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn async_esm_star_diamond_keeps_the_terminal_binding() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("async-star-origin");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './a.js'; export * from './b.js';").unwrap();
    fs::write(package.join("a.js"), "export * from './leaf.js';").unwrap();
    fs::write(package.join("b.js"), "export * from './leaf.js';").unwrap();
    fs::write(package.join("leaf.js"), "await Promise.resolve(); export const value = 9;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ return module.exports.value; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "9");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_proto_named_binding_is_data_not_graph_prototype() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-proto-origin");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './leaf.js';").unwrap();
    fs::write(package.join("leaf.js"), "export const __proto__ = 3;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ return [Object.prototype.hasOwnProperty.call(module.exports, '__proto__'), module.exports['__proto__']]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[true,3]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_nested_ambiguous_star_does_not_revive_from_later_valid_star() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-nested-ambiguous");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './barrel.js'; export * from './valid.js';").unwrap();
    fs::write(package.join("barrel.js"), "export * from './left.js'; export * from './right.js';").unwrap();
    fs::write(package.join("left.js"), "export const x = 1;").unwrap();
    fs::write(package.join("right.js"), "export const x = 1;").unwrap();
    fs::write(package.join("valid.js"), "export const x = 1;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ return Object.prototype.hasOwnProperty.call(module.exports, 'x'); }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "false");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_distinct_cjs_modules_sharing_exports_object_remain_ambiguous() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-cjs-shared-object");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './one.js'; export * from './two.js';").unwrap();
    fs::write(package.join("one.js"), "module.exports = require('./shared.js');").unwrap();
    fs::write(package.join("two.js"), "module.exports = require('./shared.js');").unwrap();
    fs::write(package.join("shared.js"), "module.exports = { x: 1 };").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ return Object.prototype.hasOwnProperty.call(module.exports, 'x'); }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "false");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_reexported_cjs_default_uses_existing_interop_value() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-cjs-default-interop");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './one.js'; export * from './two.js';").unwrap();
    fs::write(package.join("one.js"), "import fn from './leaf.cjs'; export { fn as value };").unwrap();
    fs::write(package.join("two.js"), "import fn from './leaf.cjs'; export { fn as value };").unwrap();
    fs::write(package.join("leaf.cjs"), "module.exports = function() { return 7; };").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ return [typeof module.exports.value, module.exports.value()]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[\"function\",7]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_native_addon_uses_the_original_loaded_exports_snapshot() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-native-addon-snapshot");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './native.node';").unwrap();
    fs::write(package.join("native.node"), "").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; globalThis.addonCalls = 0; globalThis.require.addon = function() {{ return {{ value: ++addonCalls }}; }}; {bundle} globalThis.starResult = function() {{ return [module.exports.value, addonCalls]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[1,1]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_cjs_inherited_enumerable_key_is_not_a_synthetic_export() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-cjs-inherited");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './leaf.cjs';").unwrap();
    fs::write(package.join("leaf.cjs"), "function Source() {} Source.prototype.inherited = 5; module.exports = new Source();").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ return [Object.prototype.hasOwnProperty.call(module.exports, 'inherited'), Object.keys(module.exports).indexOf('inherited') >= 0]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[false,false]");
    let _ = fs::remove_dir_all(root);
}


#[test]
fn esm_star_cycle_defers_unloaded_native_edge_without_reordering_body() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-native-cycle-order");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './b.js'; export * from './native.node';").unwrap();
    fs::write(package.join("b.js"), "export * from './index.js'; globalThis.events.push('B');").unwrap();
    fs::write(package.join("native.node"), "").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.events = []; globalThis.require = function(name) {{ throw new Error(name); }}; globalThis.addonCalls = 0; globalThis.require.addon = function() {{ events.push('N'); return {{ value: ++addonCalls }}; }}; {bundle} var cycleB = globalThis.__thaw_bundle_create_require('origin-pkg/index.js')('./b.js'); globalThis.starResult = function() {{ return [events.join(','), addonCalls, module.exports.value, cycleB.value]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[\"B,N\",1,1,1]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_keeps_explicit_false_esmodule_binding_and_default_import() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-esmodule-binding");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "import esmValue from './leaf.js'; import cjsValue from './legacy.js'; export * from './leaf.js'; export * from './legacy.js'; export const values = [esmValue, cjsValue];").unwrap();
    fs::write(package.join("leaf.js"), "export const __esModule = false; export default 7;").unwrap();
    fs::write(package.join("legacy.js"), "exports.__esModule = true; exports.default = 99; exports.extra = 3;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ var out = module.exports; return [out.__esModule, out.values[0], out.values[1], out.extra, typeof Object.getOwnPropertyDescriptor(out, '__esModule').get]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[false,7,99,3,\"function\"]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_explicit_namespace_reexport_precedes_competing_star() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-explicit-namespace");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * as ns from './leaf.js'; export * from './other.js';").unwrap();
    fs::write(package.join("leaf.js"), "export const value = 1;").unwrap();
    fs::write(package.join("other.js"), "export const ns = 2;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ return [module.exports.ns.value, module.exports.ns === globalThis.__thaw_bundle_create_require('origin-pkg/index.js')('./leaf.js')]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[1,true]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn esm_star_diamond_coalesces_same_namespace_reexport() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("star-namespace-diamond");
    let modules = root.join("node_modules");
    let package = modules.join("origin-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export * from './a.js'; export * from './b.js';").unwrap();
    fs::write(package.join("a.js"), "export * as ns from './leaf.js';").unwrap();
    fs::write(package.join("b.js"), "export * as ns from './leaf.js';").unwrap();
    fs::write(package.join("leaf.js"), "export const value = 7;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "origin-pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.starResult = function() {{ return [module.exports.ns.value, module.exports.ns === globalThis.__thaw_bundle_create_require('origin-pkg/index.js')('./a.js').ns]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"starResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[7,true]");
    let _ = fs::remove_dir_all(root);
}
#[test]
fn bundled_require_cache_and_commonjs_this_respect_module_format() {
    use std::ffi::{CStr, CString};
    let package = temp_registry("bundle_factory_context");
    let modules = temp_registry("bundle_factory_context_modules");
    fs::create_dir_all(package.join("esm_sub/nested")).unwrap();
    fs::create_dir_all(package.join("esm_sub/bad")).unwrap();
    fs::write(package.join("esm_sub/package.json"), r#"{"type":"module"}"#).unwrap();
    fs::write(package.join("esm_sub/nested/package.json"), r#"{"type":"commonjs"}"#).unwrap();
    fs::write(package.join("esm_sub/bad/package.json"), "{bad").unwrap();
    fs::write(package.join("index.js"), "var cache = require.cache, same = cache === globalThis.__thaw_bundle_create_require('pkg/index.js').cache, self = cache['pkg/index.js'].exports === exports; var sloppy = require('./sloppy.js'), strict = require('./strict.cjs'), hybrid = require('./hybrid.cjs'), plain = require('./plain'), custom = require('./custom.xyz'); require('./meta.js'); require('./bare.mjs'); require('./esm_sub/bare.js'); require('./esm_sub/bad/inner.js'); var override = require('./esm_sub/override.cjs'), nested = require('./esm_sub/nested/inner.js'), graph = require('./graph.js'); var first = require('./child.cjs'); delete cache['pkg/child.cjs']; var second = require('./child.cjs'); try { require('./boom.cjs'); } catch (error) {} module.exports = [same, self, sloppy.value, strict.value, hybrid.value, plain.value, custom.value, override.value, nested.value, graph.x, first.n, second.n, !Object.prototype.hasOwnProperty.call(cache, 'pkg/boom.cjs'), globalThis.metaThis === globalThis, globalThis.mjsThis === globalThis, globalThis.typedThis === globalThis, globalThis.badThis === globalThis, globalThis.graphThis === globalThis];").unwrap();
    fs::write(package.join("sloppy.js"), "var text = 'import.meta'; // import.meta is not syntax\nthis.value = 1;").unwrap();
    fs::write(package.join("strict.cjs"), "'use strict'; this.value = 2;").unwrap();
    fs::write(package.join("hybrid.cjs"), "import.meta.url; this.value = 3;").unwrap();
    fs::write(package.join("plain"), "this.value = 7;").unwrap();
    fs::write(package.join("custom.xyz"), "this.value = 8;").unwrap();
    fs::write(package.join("meta.js"), "import.meta.url; globalThis.metaThis = this;").unwrap();
    fs::write(package.join("bare.mjs"), "globalThis.mjsThis = this;").unwrap();
    fs::write(package.join("esm_sub/bare.js"), "globalThis.typedThis = this;").unwrap();
    fs::write(package.join("esm_sub/bad/inner.js"), "globalThis.badThis = this;").unwrap();
    fs::write(package.join("esm_sub/override.cjs"), "this.value = 4;").unwrap();
    fs::write(package.join("esm_sub/nested/inner.js"), "this.value = 5;").unwrap();
    fs::write(package.join("graph.js"), "globalThis.graphThis = this; export const x = 6;").unwrap();
    fs::write(package.join("child.cjs"), "globalThis.childRuns = (globalThis.childRuns || 0) + 1; this.n = globalThis.childRuns;").unwrap();
    fs::write(package.join("boom.cjs"), "throw new Error('boom');").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.factoryContext = function() {{ return module.exports; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"factoryContext".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[true,true,1,2,3,7,8,4,5,6,1,2,true,true,false,false,true,true]");
    let _ = fs::remove_dir_all(package);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn async_commonjs_factory_keeps_top_level_this_after_await() {
    use std::ffi::{CStr, CString};
    let package = temp_registry("bundle_async_factory_context");
    let modules = temp_registry("bundle_async_factory_context_modules");
    fs::write(package.join("index.js"), "module.exports = async function() { var value = await import('./late.cjs'); return value.answer; };").unwrap();
    fs::write(package.join("late.cjs"), "await Promise.resolve(); this.answer = 42;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.asyncFactoryContext = module.exports;");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"asyncFactoryContext".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");
    let _ = fs::remove_dir_all(package);
    let _ = fs::remove_dir_all(modules);
}
#[test]
fn bundled_require_resolve_finds_resolve_only_files_and_rejects_missing_modules() {
    use std::ffi::{CStr, CString};
    let package = temp_registry("bundle_require_resolve_only");
    let modules = temp_registry("bundle_require_resolve_only_modules");
    fs::create_dir_all(package.join("sub")).unwrap();
    fs::create_dir_all(modules.join("present-package")).unwrap();
    fs::write(modules.join("present-package/package.json"), r#"{"main":"index.js"}"#).unwrap();
    fs::write(modules.join("present-package/index.js"), "globalThis.resolveOnlyRuns = (globalThis.resolveOnlyRuns || 0) + 1;").unwrap();
    fs::write(package.join("index.js"), "import { createRequire } from 'node:module'; var made = createRequire(__filename); module.exports = function() { var only = require['resolve']('./' + 'only.js'), created = made.resolve('./created.js'), other = require.resolve('./sub/other.js'), bare = require.resolve('present-package'); var all = globalThis.__thaw_bundle_create_require('unmatched/base').resolve('./elsewhere.js'), exported = made.resolve('pkg/index.js'); var scoped, missingLocal, missingBare; try { made.resolve('./elsewhere.js'); } catch (error) { scoped = error.code; } try { require.resolve('./absent.js'); } catch (error) { missingLocal = error.code; } try { made.resolve('uninstalled-optional'); } catch (error) { missingBare = error.code; } require('node:module').builtinModules.length = 0; return [only, created, other, bare, all, exported, scoped, missingLocal, missingBare, require.resolve('node:inspector/promises'), globalThis.resolveOnlyRuns || 0]; };").unwrap();
    fs::write(package.join("only.js"), "globalThis.resolveOnlyRuns = (globalThis.resolveOnlyRuns || 0) + 1;").unwrap();
    fs::write(package.join("created.js"), "globalThis.resolveOnlyRuns = (globalThis.resolveOnlyRuns || 0) + 1;").unwrap();
    fs::write(package.join("sub/other.js"), "module.exports = require('./elsewhere.js');").unwrap();
    fs::write(package.join("sub/elsewhere.js"), "globalThis.resolveOnlyRuns = (globalThis.resolveOnlyRuns || 0) + 1;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &package, "index.js").unwrap();
    let script = format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; globalThis.resolveOnlyRuns = 0; {bundle} globalThis.resolveOnly = module.exports;");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"resolveOnly".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[\"pkg/only.js\",\"pkg/created.js\",\"pkg/sub/other.js\",\"present-package/index.js\",\"pkg/sub/elsewhere.js\",\"pkg/index.js\",\"MODULE_NOT_FOUND\",\"MODULE_NOT_FOUND\",\"MODULE_NOT_FOUND\",\"node:inspector/promises\",0]");
    let _ = fs::remove_dir_all(package);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn native_esm_cycle_links_original_mjs_before_evaluation() {
    // Unrun regression: B evaluates first, observes A's hoisted function, and
    // gets TDZ for A's let binding. The old factory rewrite evaluated eagerly.
    use std::ffi::{CStr, CString};
    let root = temp_registry("native-esm-cycle-origin");
    let modules = root.join("node_modules");
    let package = modules.join("native-origin-cycle-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.mjs"), "import { observed } from './b.mjs'; export function ready() { return 7; } export let value = 3; export const result = observed;").unwrap();
    fs::write(package.join("b.mjs"), "import { ready, value } from './index.mjs'; export const observed = [typeof ready, (() => { try { return value; } catch (error) { return error.name; } })()];").unwrap();
    let (bundle, _, count, _) = bundle_commonjs_package(&modules, "native-origin-cycle-pkg", &package, "index.mjs").unwrap();
    assert_eq!(count, 2);
    assert!(bundle.contains("__thaw_register_native_bundle"));
    let script = format!("globalThis.module = {{ exports: {{}} }}; {bundle} globalThis.nativeOriginResult = function() {{ return [module.exports.result, module.exports.ready(), module.exports.value]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"nativeOriginResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[[\"function\",\"ReferenceError\"],7,3]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_esm_route_preserves_mixed_mjs_and_cjs_evaluation_order() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("native-esm-mixed-fallback");
    let modules = root.join("node_modules");
    let package = modules.join("native-mixed-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.mjs"), "import legacy from './legacy.cjs'; export const value = legacy; export { default as again } from './legacy.cjs'; export * as namespace from './legacy.cjs';").unwrap();
    fs::write(package.join("legacy.cjs"), "globalThis.__thaw_mixed_runs = (globalThis.__thaw_mixed_runs || 0) + 1; module.exports = 9;").unwrap();
    let (bundle, _, count, _) = bundle_commonjs_package(&modules, "native-mixed-pkg", &package, "index.mjs").unwrap();
    assert_eq!(count, 2);
    assert!(bundle.contains("__thaw_register_native_bundle"));
    let script = format!("globalThis.module = {{ exports: {{}} }}; {bundle} globalThis.mixedOriginValue = function() {{ return [module.exports.value, module.exports.again, module.exports.namespace, globalThis.__thaw_mixed_runs]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"mixedOriginValue".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[9,9,9,1]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_mixed_pure_namespace_keeps_engine_identity() {
    // Unrun: a mixed facade's namespace reexport of a pure native module
    // must be the same engine namespace used by untouched and dynamic imports.
    use std::ffi::{CStr, CString};
    let root = temp_registry("native-mixed-pure-namespace");
    let modules = root.join("node_modules");
    let package = modules.join("native-namespace-identity-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.mjs"), "import legacy from './legacy.cjs'; import * as mixedImported from './mixed.mjs'; import * as imported from './pure.mjs'; export * as mixedNs from './mixed.mjs'; export * from './left.mjs'; export * from './right.mjs'; export { imported, mixedImported }; export const value = legacy; export const loadPure = () => import('./pure.mjs'); export const loadMixed = () => import('./mixed.mjs');").unwrap();
    fs::write(package.join("pure.mjs"), "globalThis.__thaw_pure_runs++; export let value = 17; export function set(value_) { value = value_; }").unwrap();
    fs::write(package.join("bridge.mjs"), "export * as ns from './pure.mjs';").unwrap();
    fs::write(package.join("left.mjs"), "export { ns } from './bridge.mjs';").unwrap();
    fs::write(package.join("right.mjs"), "export { ns } from './bridge.mjs';").unwrap();
    fs::write(package.join("mixed.mjs"), "import { ns } from './index.mjs'; import legacy from './legacy.cjs'; export const earlyNs = ns; export const value = legacy;").unwrap();
    fs::write(package.join("legacy.cjs"), "module.exports = 9;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "native-namespace-identity-pkg", &package, "index.mjs").unwrap();
    assert!(bundle.contains("__thaw_register_native_bundle"));
    let script = format!("globalThis.__thaw_pure_runs = 0; globalThis.module = {{ exports: {{}} }}; {bundle} globalThis.readNamespaceIdentity = function() {{ var exported = module.exports; exported.ns.set(18); return [exported.ns === exported.imported, exported.ns.value, exported.mixedNs === exported.mixedImported, exported.mixedNs.earlyNs === exported.ns, exported.mixedNs.value, __thaw_pure_runs]; }}; globalThis.readDynamicNamespaceIdentity = function() {{ var exported = module.exports; return Promise.all([exported.loadPure(), exported.loadMixed()]).then(function(values) {{ return [values[0] === exported.ns, values[1] === exported.mixedNs, values[0].value, __thaw_pure_runs]; }}); }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let direct = thaw_quickjs::thaw_js_call(c"readNamespaceIdentity".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(direct) }.to_string_lossy(), "[true,18,true,true,9,1]");
    let dynamic = thaw_quickjs::thaw_js_call(c"readDynamicNamespaceIdentity".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(dynamic) }.to_string_lossy(), "[true,true,18,1]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_mixed_namespace_anchor_reseeds_after_stripped_edge() {
    // Unrun three-module cycle: A's export through mixed D is stripped, but
    // D's export of pure B is native. Reader R runs before D's later B edge,
    // so its import of A.ns must read D's retained alias before B evaluates.
    use std::ffi::{CStr, CString};
    let root = temp_registry("native-namespace-anchor-reseed");
    let modules = root.join("node_modules");
    let package = modules.join("native-anchor-reseed-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.mjs"), "import './legacy.cjs'; import './mixed.mjs'; export { ns } from './mixed.mjs';").unwrap();
    fs::write(package.join("mixed.mjs"), "import './reader.mjs'; export * as ns from './pure.mjs';").unwrap();
    fs::write(package.join("reader.mjs"), "import { ns } from './index.mjs'; globalThis.__thaw_early_before_pure = globalThis.__thaw_pure_runs === 0; globalThis.__thaw_early_ns = ns; export const early = ns;").unwrap();
    fs::write(package.join("pure.mjs"), "globalThis.__thaw_pure_runs++; export const value = 17;").unwrap();
    fs::write(package.join("legacy.cjs"), "module.exports = 9;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "native-anchor-reseed-pkg", &package, "index.mjs").unwrap();
    assert!(bundle.contains("__thaw_register_native_bundle"));
    let script = format!("globalThis.__thaw_pure_runs = 0; globalThis.module = {{ exports: {{}} }}; {bundle} globalThis.readAnchorReseed = function() {{ return [__thaw_early_before_pure, __thaw_early_ns === module.exports.ns, __thaw_early_ns.value, __thaw_pure_runs]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"readAnchorReseed".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[true,true,17,1]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_mixed_rewrite_uses_private_opaque_edge_and_origin_reads() {
    use std::collections::BTreeSet;
    let source = "import { value as local } from './legacy.cjs'; \
        export { local as renamed }; export { value as indirect } from './legacy.cjs'; \
        export * from './legacy.cjs'; export const direct = local; \
        export async function later() { return import('./legacy.cjs'); }";
    let source_name = thaw_parser::common::FileName::Custom("mixed.mjs".into());
    let specs = BTreeSet::from(["./legacy.cjs".to_owned()]);
    let rewritten = rewrite_native_opaque_edges_named(
        source, &specs, "__thaw_private_origin_spec/test", &source_name,
    ).expect("static opaque imports rewrite");
    assert!(rewritten.contains("import { origin as __thaw_esm_origin_"), "{rewritten}");
    assert!(rewritten.contains("import \"./legacy.cjs\";"), "{rewritten}");
    assert!(rewritten.contains(".readImport(\"./legacy.cjs\", \"value\")"), "{rewritten}");
    assert!(rewritten.contains(".selectDynamic("), "{rewritten}");
    assert!(!rewritten.contains("export { local as renamed }"), "{rewritten}");
    assert!(!rewritten.contains("export { value as indirect } from"), "{rewritten}");
    assert!(!rewritten.contains("export * from"), "{rewritten}");
}

#[test]
fn native_mixed_cjs_dynamic_import_uses_bound_native_ready() {
    // Unrun regression: the CJS factory starts after its native importer
    // links, then dynamically imports another native module through the same
    // registration. It must not invoke the renderer's native stub.
    use std::ffi::{CStr, CString};
    let root = temp_registry("native-mixed-cjs-dynamic");
    let modules = root.join("node_modules");
    let package = modules.join("native-mixed-dynamic-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.mjs"), "import later from './legacy.cjs'; export const result = later;").unwrap();
    fs::write(package.join("legacy.cjs"), "module.exports = Promise.all([import('./native.mjs'), import('./native.mjs')]).then(([a, b]) => [a.value, b.value, globalThis.__thaw_native_dynamic_runs]);").unwrap();
    fs::write(package.join("native.mjs"), "globalThis.__thaw_native_dynamic_runs = (globalThis.__thaw_native_dynamic_runs || 0) + 1; export const value = 17;").unwrap();
    let (bundle, _, count, _) = bundle_commonjs_package(&modules, "native-mixed-dynamic-pkg", &package, "index.mjs").unwrap();
    assert_eq!(count, 3);
    assert!(bundle.contains("__thaw_register_native_bundle"));
    let script = format!("globalThis.module = {{ exports: {{}} }}; {bundle} globalThis.readNativeDynamic = function() {{ return module.exports.result; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"readNativeDynamic".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[17,17,1]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_mixed_side_effect_only_mjs_keeps_module_this() {
    // Unrun regression: an imported .mjs with no import/export declaration
    // still has native ESM top-level-this semantics in a mixed package.
    use std::ffi::{CStr, CString};
    let root = temp_registry("native-mixed-sideeffect-mjs");
    let modules = root.join("node_modules");
    let package = modules.join("native-mixed-sideeffect-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.mjs"), "import './side.mjs'; import legacy from './legacy.cjs'; export const value = legacy;").unwrap();
    fs::write(package.join("side.mjs"), "globalThis.__thaw_side_module_this = (this === undefined);").unwrap();
    fs::write(package.join("legacy.cjs"), "module.exports = 9;").unwrap();
    let (bundle, _, count, _) = bundle_commonjs_package(&modules, "native-mixed-sideeffect-pkg", &package, "index.mjs").unwrap();
    assert_eq!(count, 3);
    assert!(bundle.contains("__thaw_register_native_bundle"));
    let script = format!("globalThis.module = {{ exports: {{}} }}; {bundle} globalThis.readNativeSideEffect = function() {{ return [module.exports.value, globalThis.__thaw_side_module_this]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"readNativeSideEffect".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[9,true]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_mixed_async_opaque_back_edge_keeps_legacy_bridge() {
    // Unrun cycle gate: the native entry waits for its async opaque import,
    // whose dynamic import can in turn wait for that same native entry.
    let root = temp_registry("native-mixed-async-cycle");
    let modules = root.join("node_modules");
    let package = modules.join("native-mixed-cycle-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.mjs"), "import { value } from './legacy.js'; export const result = value;").unwrap();
    fs::write(package.join("legacy.js"), "export const value = await import('./index.mjs');").unwrap();
    let (bundle, _, count, _) = bundle_commonjs_package(&modules, "native-mixed-cycle-pkg", &package, "index.mjs").unwrap();
    assert_eq!(count, 2);
    assert!(!bundle.contains("__thaw_register_native_bundle"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_mixed_removed_named_edges_fail_before_any_factory_runs() {
    // Unrun regression: rewriting an unused named import to an opaque shell
    // must retain native link-time missing/ambiguous export validation.
    use std::ffi::{CStr, CString};
    for (kind, middle, extras) in [
        ("missing", "import legacy from './legacy.cjs'; export const present = legacy;", false),
        ("ambiguous", "import legacy from './legacy.cjs'; export * from './left.mjs'; export * from './right.mjs'; export const present = legacy;", true),
    ] {
        let root = temp_registry(&format!("native-mixed-link-{kind}"));
        let modules = root.join("node_modules");
        let package = modules.join("native-mixed-link-pkg");
        fs::create_dir_all(&package).unwrap();
        let requested = if extras { "duplicate" } else { "absent" };
        fs::write(package.join("index.mjs"), format!("import {{ {requested} }} from './middle.mjs'; export const value = 1;")).unwrap();
        fs::write(package.join("middle.mjs"), middle).unwrap();
        fs::write(package.join("legacy.cjs"), "globalThis.__thaw_link_runs++; module.exports = 9;").unwrap();
        if extras {
            fs::write(package.join("left.mjs"), "globalThis.__thaw_link_runs++; export const duplicate = 1;").unwrap();
            fs::write(package.join("right.mjs"), "globalThis.__thaw_link_runs++; export const duplicate = 2;").unwrap();
        }
        let (bundle, _, _, _) = bundle_commonjs_package(&modules, "native-mixed-link-pkg", &package, "index.mjs").unwrap();
        assert!(bundle.contains("validateStatic"), "{kind}: {bundle}");
        let script = format!("globalThis.__thaw_link_runs = 0; globalThis.readMixedLinkRuns = function() {{ return __thaw_link_runs; }}; globalThis.module = {{ exports: {{}} }}; {bundle}");
        assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 0);
        let result = thaw_quickjs::thaw_js_call(c"readMixedLinkRuns".as_ptr(), c"[]".as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "0");
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn native_mixed_static_then_cjs_dynamic_import_reuses_declared_sibling() {
    // Unrun regression: the loader already declared native.mjs for the
    // static import before a CJS factory dynamically requests the same key.
    use std::ffi::{CStr, CString};
    let root = temp_registry("native-mixed-static-then-dynamic");
    let modules = root.join("node_modules");
    let package = modules.join("native-mixed-static-dynamic-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.mjs"), "import { value } from './native.mjs'; import later from './legacy.cjs'; export const result = later.then(other => [value, other, globalThis.__thaw_static_dynamic_runs]);").unwrap();
    fs::write(package.join("native.mjs"), "globalThis.__thaw_static_dynamic_runs = (globalThis.__thaw_static_dynamic_runs || 0) + 1; export const value = 17;").unwrap();
    fs::write(package.join("legacy.cjs"), "module.exports = import('./native.mjs').then(ns => ns.value);").unwrap();
    let (bundle, _, count, _) = bundle_commonjs_package(&modules, "native-mixed-static-dynamic-pkg", &package, "index.mjs").unwrap();
    assert_eq!(count, 3);
    assert!(bundle.contains("__thaw_register_native_bundle"));
    let script = format!("globalThis.module = {{ exports: {{}} }}; {bundle} globalThis.readMixedStaticDynamic = function() {{ return module.exports.result; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"readMixedStaticDynamic".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[17,17,1]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_mixed_lazy_link_errors_wait_for_the_dynamic_entry() {
    // Unrun controls for both native import() and the registration-bound CJS
    // import(): unrelated lazy modules cannot fail the initial entry, and
    // their missing/ambiguous names reject before their factories execute.
    use std::ffi::{CStr, CString};
    for (kind, entry, middle, extra) in [
        ("native", "import front from './front.cjs'; export const load = () => import('./lazy.mjs'); export const loadValid = () => import('./valid.mjs'); export const ready = front; Promise.reject = () => { throw new Error('mutated reject'); }; Promise.prototype.then = () => { throw new Error('mutated then'); };", "import late from './late.cjs'; export const present = late;", false),
        ("cjs", "import front from './front.cjs'; export const load = front;", "import late from './late.cjs'; export * from './left.mjs'; export * from './right.mjs'; export const present = late;", true),
    ] {
        let root = temp_registry(&format!("native-mixed-lazy-link-{kind}"));
        let modules = root.join("node_modules");
        let package = modules.join("native-mixed-lazy-link-pkg");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("index.mjs"), entry).unwrap();
        fs::write(package.join("front.cjs"), if extra { "module.exports = () => import('./lazy.mjs');" } else { "module.exports = 9;" }).unwrap();
        let requested = if extra { "duplicate" } else { "absent" };
        fs::write(package.join("lazy.mjs"), format!("import {{ {requested} }} from './middle.mjs'; export const value = 1;")).unwrap();
        if !extra {
            fs::write(package.join("valid.mjs"), "export const value = 17;").unwrap();
        }
        fs::write(package.join("middle.mjs"), middle).unwrap();
        fs::write(package.join("late.cjs"), "globalThis.__thaw_late_runs++; module.exports = 3;").unwrap();
        if extra {
            fs::write(package.join("left.mjs"), "globalThis.__thaw_late_runs++; export const duplicate = 1;").unwrap();
            fs::write(package.join("right.mjs"), "globalThis.__thaw_late_runs++; export const duplicate = 2;").unwrap();
        }
        let (bundle, _, _, _) = bundle_commonjs_package(&modules, "native-mixed-lazy-link-pkg", &package, "index.mjs").unwrap();
        assert!(bundle.contains("__thaw_register_native_bundle"));
        let script = format!("globalThis.__thaw_late_runs = 0; globalThis.__thaw_saved_then = Promise.prototype.then; globalThis.__thaw_saved_reject = Promise.reject; globalThis.__thaw_saved_apply = Reflect.apply; globalThis.module = {{ exports: {{}} }}; {bundle} globalThis.readLazyState = function() {{ return [module.exports.ready, __thaw_late_runs]; }}; globalThis.loadValidLazy = function() {{ return __thaw_saved_apply(__thaw_saved_then, module.exports.loadValid(), [namespace => [namespace.value, __thaw_late_runs], error => ['unexpected', error.name]]); }}; globalThis.loadLazy = function() {{ return __thaw_saved_apply(__thaw_saved_then, module.exports.load(), [() => ['unexpected'], error => [error.name, __thaw_late_runs]]); }}; globalThis.restoreLazyPromise = function() {{ Promise.prototype.then = __thaw_saved_then; Promise.reject = __thaw_saved_reject; return true; }};");
        assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
        let ready = thaw_quickjs::thaw_js_call(c"readLazyState".as_ptr(), c"[]".as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(ready) }.to_string_lossy(), if extra { "[null,0]" } else { "[9,0]" });
        let valid = if extra {
            None
        } else {
            let result = thaw_quickjs::thaw_js_call(c"loadValidLazy".as_ptr(), c"[]".as_ptr());
            Some(unsafe { CStr::from_ptr(result) }.to_string_lossy().into_owned())
        };
        let failed = thaw_quickjs::thaw_js_call(c"loadLazy".as_ptr(), c"[]".as_ptr());
        let failed = unsafe { CStr::from_ptr(failed) }.to_string_lossy().into_owned();
        let restored = thaw_quickjs::thaw_js_call(c"restoreLazyPromise".as_ptr(), c"[]".as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(restored) }.to_string_lossy(), "true");
        if let Some(valid) = valid {
            assert_eq!(valid, "[17,0]");
        }
        assert_eq!(failed, "[\"SyntaxError\",0]");
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn native_esm_bundle_name_is_fresh_for_same_key_new_source() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("native-esm-fresh-bundle");
    let modules = root.join("node_modules");
    let package = modules.join("native-fresh-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.mjs"), "export const value = 1;").unwrap();
    let (first, _, _, _) = bundle_commonjs_package(&modules, "native-fresh-pkg", &package, "index.mjs").unwrap();
    let first_script = format!("globalThis.module = {{ exports: {{}} }}; {first} globalThis.firstNativeValue = function() {{ return module.exports.value; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(first_script).unwrap().as_ptr()), 1);
    let one = thaw_quickjs::thaw_js_call(c"firstNativeValue".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(one) }.to_string_lossy(), "1");
    fs::write(package.join("index.mjs"), "export const value = 2;").unwrap();
    let (second, _, _, _) = bundle_commonjs_package(&modules, "native-fresh-pkg", &package, "index.mjs").unwrap();
    let second_script = format!("globalThis.module = {{ exports: {{}} }}; {second} globalThis.secondNativeValue = function() {{ return module.exports.value; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(second_script).unwrap().as_ptr()), 1);
    let two = thaw_quickjs::thaw_js_call(c"secondNativeValue".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(two) }.to_string_lossy(), "2");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_esm_query_suffixes_retain_distinct_module_identity() {
    use std::ffi::{CStr, CString};
    let root = temp_registry("native-esm-query-identity");
    let modules = root.join("node_modules");
    let package = modules.join("native-query-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.mjs"), "import * as a from './leaf.mjs?one'; import * as b from './leaf.mjs?two'; export const result = [a.token === b.token, globalThis.queryLeafRuns];").unwrap();
    fs::write(package.join("leaf.mjs"), "globalThis.queryLeafRuns = (globalThis.queryLeafRuns || 0) + 1; export const token = {};").unwrap();
    let (bundle, _, count, _) = bundle_commonjs_package(&modules, "native-query-pkg", &package, "index.mjs").unwrap();
    assert_eq!(count, 3);
    assert!(bundle.contains("__thaw_register_native_bundle"));
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.queryLeafRuns = 0; {bundle} globalThis.nativeQueryResult = function() {{ return module.exports.result; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"nativeQueryResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[false,2]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_esm_type_module_js_uses_parsed_capability_gate() {
    let root = temp_registry("native-esm-type-module");
    let modules = root.join("node_modules");
    let package = modules.join("native-type-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"), r#"{"type":"module"}"#).unwrap();
    fs::write(package.join("index.js"), "// module require Worker fetch import.meta are only words here\nexport const text = 'module require Worker fetch import.meta';").unwrap();
    let (bundle, _, count, _) = bundle_commonjs_package(&modules, "native-type-pkg", &package, "index.js").unwrap();
    assert_eq!(count, 1);
    assert!(bundle.contains("__thaw_register_native_bundle"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_esm_requires_a_valid_declared_file_context() {
    let root = temp_registry("native-esm-declared-context");
    let modules = root.join("node_modules");
    let package = modules.join("native-context-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.js"), "export const value = 1;").unwrap();
    fs::write(package.join("package.json"), r#"{"type":"unknown"}"#).unwrap();
    let (invalid_type, _, _, _) = bundle_commonjs_package(&modules, "native-context-pkg", &package, "index.js").unwrap();
    assert!(!invalid_type.contains("__thaw_register_native_bundle"));
    fs::write(package.join("package.json"), "{").unwrap();
    assert!(!declared_native_esm_context(&package.join("index.js"), &package));
    fs::write(package.join("package.json"), r#"{"type":"module"}"#).unwrap();
    fs::write(package.join("other.xyz"), "export const value = 2;").unwrap();
    let (arbitrary_extension, _, _, _) = bundle_commonjs_package(&modules, "native-context-pkg", &package, "other.xyz").unwrap();
    assert!(!arbitrary_extension.contains("__thaw_register_native_bundle"));
    let (declared_js, _, _, _) = bundle_commonjs_package(&modules, "native-context-pkg", &package, "index.js").unwrap();
    assert!(declared_js.contains("__thaw_register_native_bundle"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_esm_dynamic_global_capability_uses_legacy_bundle() {
    let root = temp_registry("native-esm-computed-worker");
    let modules = root.join("node_modules");
    let package = modules.join("native-computed-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.mjs"), "const name = 'Worker'; export const capability = globalThis[name];").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "native-computed-pkg", &package, "index.mjs").unwrap();
    assert!(!bundle.contains("__thaw_register_native_bundle"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_esm_dynamic_absolute_name_cannot_escape_its_bundle() {
    // An isolated realm makes the first Rust-minted prefix deterministic.
    std::thread::spawn(|| {
        use std::ffi::{CStr, CString};
        let root = temp_registry("native-esm-absolute-escape");
        let modules = root.join("node_modules");
        let package = modules.join("native-escape-pkg");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("index.mjs"), "export const value = 1;").unwrap();
        let (first, _, _, _) = bundle_commonjs_package(&modules, "native-escape-pkg", &package, "index.mjs").unwrap();
        let first_script = format!("globalThis.module = {{ exports: {{}} }}; {first}");
        assert_eq!(thaw_quickjs::thaw_js_load(CString::new(first_script).unwrap().as_ptr()), 1);
        let old_literal = serde_json::to_string("thaw-bundle:1:native-escape-pkg/index.mjs").unwrap();
        fs::write(package.join("index.mjs"), format!("export const blocked = await import({old_literal}).then(function() {{ return false; }}, function() {{ return true; }});")).unwrap();
        let (second, _, _, _) = bundle_commonjs_package(&modules, "native-escape-pkg", &package, "index.mjs").unwrap();
        assert!(second.contains("__thaw_register_native_bundle"));
        let second_script = format!("globalThis.module = {{ exports: {{}} }}; {second} globalThis.absoluteImportBlocked = function() {{ return module.exports.blocked; }};");
        assert_eq!(thaw_quickjs::thaw_js_load(CString::new(second_script).unwrap().as_ptr()), 1);
        let result = thaw_quickjs::thaw_js_call(c"absoluteImportBlocked".as_ptr(), c"[]".as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "true");
        let _ = fs::remove_dir_all(root);
    }).join().unwrap();
}
#[test]
fn native_esm_bundle_sequence_ignores_mutable_global_reset() {
    std::thread::spawn(|| {
        use std::ffi::{CStr, CString};
        let root = temp_registry("native-esm-sequence-reset");
        let modules = root.join("node_modules");
        let package = modules.join("native-counter-pkg");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("index.mjs"), "export const value = 1;").unwrap();
        let (first, _, _, _) = bundle_commonjs_package(&modules, "native-counter-pkg", &package, "index.mjs").unwrap();
        let first_script = format!("globalThis.module = {{ exports: {{}} }}; {first}");
        assert_eq!(thaw_quickjs::thaw_js_load(CString::new(first_script).unwrap().as_ptr()), 1);
        let reset_script = "globalThis.__thaw_native_esm_next_sequence = function() { return '1'; }; globalThis.__thaw_native_esm_sources = { 'thaw-bundle:1:native-counter-pkg/index.mjs': { source: 'export const value = 999;', imports: {} } };";
        assert_eq!(thaw_quickjs::thaw_js_load(CString::new(reset_script).unwrap().as_ptr()), 1);
        fs::write(package.join("index.mjs"), "export const value = 2;").unwrap();
        let (second, _, _, _) = bundle_commonjs_package(&modules, "native-counter-pkg", &package, "index.mjs").unwrap();
        let second_script = format!("globalThis.module = {{ exports: {{}} }}; {second} globalThis.secondCounterValue = function() {{ return [module.exports.value, typeof globalThis.__thaw_eval_native_entry]; }};");
        assert_eq!(thaw_quickjs::thaw_js_load(CString::new(second_script).unwrap().as_ptr()), 1);
        let result = thaw_quickjs::thaw_js_call(c"secondCounterValue".as_ptr(), c"[]".as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[2,\"undefined\"]");
        let _ = fs::remove_dir_all(root);
    }).join().unwrap();
}
#[test]
fn native_esm_private_entry_reentry_and_intrinsic_then() {
    // Unrun: a bound evaluator, not a public token, owns this entry. The
    // module reenters it while evaluating and receives the same ready Promise.
    std::thread::spawn(|| {
        use std::ffi::{CStr, CString};
        let script = r#"
          globalThis.nativeRuns = 0;
          globalThis.nativeSelfEval = __thaw_register_native_bundle(JSON.stringify({
            main: 'self/index.mjs', modules: [{ key: 'self/index.mjs', imports: {},
              source: 'globalThis.nativeRuns++; globalThis.nativeSelfReady = globalThis.nativeSelfEval(); export const value = 7;'
            }]
          }));
          Promise.prototype.then = function() { throw Error('mutated then'); };
          globalThis.__thaw_module_ready = globalThis.nativeSelfEval();
          globalThis.privateEntryResult = function() {
            return [globalThis.nativeRuns, globalThis.nativeSelfReady === globalThis.nativeSelfEval(),
              typeof globalThis.__thaw_eval_native_entry, typeof globalThis.__thaw_native_esm_sources];
          };
        "#;
        assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
        let result = thaw_quickjs::thaw_js_call(c"privateEntryResult".as_ptr(), c"[]".as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[1,true,\"undefined\",\"undefined\"]");
    }).join().unwrap();
}

#[test]
fn native_esm_private_linked_read_is_owner_scoped_and_live() {
    // Unrun: each source's first private marker runs after linking, before
    // that source body. A cyclic peer can read hoisted exports but not TDZ.
    std::thread::spawn(|| {
        use std::ffi::{CStr, CString};
        let script = r#"
          globalThis.entryA = __thaw_register_native_bundle(JSON.stringify({
            main: 'a', modules: [
              { key: 'a', imports: { './b': 'b', './unused': 'unused' }, source:
                'import "./b"; export function available() { return 7; } export const late = 9; export function loadUnused() { return import("./unused"); }' },
              { key: 'b', imports: { './a': 'a' }, source:
                'import "./a"; globalThis.nativeHoisted = globalThis.entryA.readNative("thaw-bundle:1:a", "available")(); try { globalThis.entryA.readNative("thaw-bundle:1:a", "late"); } catch (e) { globalThis.nativeTdz = e instanceof ReferenceError; } export const peer = 3;' },
              { key: 'unused', imports: {}, source: 'export const hidden = 4;' }
            ]
          }));
          globalThis.beforeNative = (() => {
            try { entryA.readNative('thaw-bundle:1:a', 'available'); return false; }
            catch (_) { return true; }
          })();
          globalThis.beforeNamespace = (() => {
            try { entryA.readNative('thaw-bundle:1:a'); return false; }
            catch (_) { return true; }
          })();
          globalThis.entryB = __thaw_register_native_bundle(JSON.stringify({
            main: 'other', modules: [{ key: 'other', imports: {}, source: 'export const value = 8;' }]
          }));
          globalThis.readyA = entryA();
          globalThis.readyB = entryB();
          globalThis.__thaw_module_ready = Promise.all([readyA, readyB]);
          globalThis.privateLinkedResult = function() {
            let unused, unusedNamespace, foreign, foreignNamespace;
            try { entryA.readNative('thaw-bundle:1:unused', 'hidden'); unused = false; }
            catch (_) { unused = true; }
            try { entryA.readNative('thaw-bundle:1:unused'); unusedNamespace = false; }
            catch (_) { unusedNamespace = true; }
            try { entryA.readNative('thaw-bundle:2:other', 'value'); foreign = false; }
            catch (_) { foreign = true; }
            try { entryA.readNative('thaw-bundle:2:other'); foreignNamespace = false; }
            catch (_) { foreignNamespace = true; }
            const descriptor = Object.getOwnPropertyDescriptor(entryA, 'readNative');
            return [beforeNative, beforeNamespace, nativeHoisted, nativeTdz, entryA.readNative('thaw-bundle:1:a', 'late'),
              entryA.readNative('thaw-bundle:1:b', 'peer'), unused, unusedNamespace,
              entryB.readNative('thaw-bundle:2:other', 'value') === 8,
              entryA.readNative('thaw-bundle:1:a').late === 9, foreign, foreignNamespace,
              descriptor.enumerable === false && descriptor.writable === false,
              entryA() === readyA];
          };
        "#;
        assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
        let result = thaw_quickjs::thaw_js_call(c"privateLinkedResult".as_ptr(), c"[]".as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[true,true,7,true,9,3,true,true,true,true,true,true,true,true]");
    }).join().unwrap();
}

#[test]
fn native_esm_failed_link_never_marks_source_linked() {
    std::thread::spawn(|| {
        use std::ffi::{CStr, CString};
        let script = r#"
          globalThis.failedEntry = __thaw_register_native_bundle(JSON.stringify({
            main: 'bad', modules: [{ key: 'bad', imports: {}, source:
              'import "./unmapped"; export const value = 1;' }]
          }));
          failedEntry().catch(() => {});
          globalThis.failedRead = function() {
            try { failedEntry.readNative('thaw-bundle:1:bad', 'value'); return false; }
            catch (_) { return true; }
          };
        "#;
        assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
        let result = thaw_quickjs::thaw_js_call(c"failedRead".as_ptr(), c"[]".as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "true");
    }).join().unwrap();
}

#[test]
fn commonjs_first_require_uses_cached_native_namespace_and_live_cycle_exports() {
    // Unrun: a CommonJS main enters a native ESM dependency synchronously.
    // The native star getter must read the final module.exports after the
    // provisional CommonJS object was replaced later in the same cycle.
    use std::ffi::{CStr, CString};
    let root = temp_registry("cjs-first-native-cycle");
    let modules = root.join("node_modules");
    let package = modules.join("cjs-first-native-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.cjs"), "exports.value = 1; var first = require('./native.mjs'); var second = require('./native.mjs'); var created = require('node:module').createRequire(__filename)('./native.mjs'); module.exports = { value: 2, same: first === second && second === created, read: function() { return first.value; } };").unwrap();
    fs::write(package.join("native.mjs"), "export * from './index.cjs';").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "cjs-first-native-pkg", &package, "index.cjs").unwrap();
    assert!(bundle.contains("evalNativeSync"));
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.cjsFirstNativeResult = function() {{ return [module.exports.same, module.exports.read()]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"cjsFirstNativeResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[true,2]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sync_require_rejects_async_factory_and_native_tla_before_side_effects() {
    // Unrun: both direct and created synchronous require must fail before
    // any pre-await statement runs. Async import remains a separate path.
    use std::ffi::{CStr, CString};
    let root = temp_registry("cjs-first-async-guard");
    let modules = root.join("node_modules");
    let package = modules.join("cjs-first-async-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.cjs"), "var created = require('node:module').createRequire(__filename); var errors = []; try { require('./late.cjs'); } catch (error) { errors.push(error.code === 'ERR_REQUIRE_ASYNC_MODULE'); } try { created('./late.cjs'); } catch (error) { errors.push(error.code === 'ERR_REQUIRE_ASYNC_MODULE'); } try { require('./native.mjs'); } catch (error) { errors.push(error.code === 'ERR_REQUIRE_ASYNC_MODULE'); } try { created('./native.mjs'); } catch (error) { errors.push(error.code === 'ERR_REQUIRE_ASYNC_MODULE'); } module.exports = errors;").unwrap();
    fs::write(package.join("late.cjs"), "globalThis.asyncFactoryRan = true; await Promise.resolve(); module.exports = 1;").unwrap();
    fs::write(package.join("native.mjs"), "globalThis.nativeTlaRan = true; await Promise.resolve(); export const value = 2;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "cjs-first-async-pkg", &package, "index.cjs").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.asyncGuardResult = function() {{ return [module.exports, globalThis.asyncFactoryRan === undefined, globalThis.nativeTlaRan === undefined]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"asyncGuardResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[[true,true,true,true],true,true]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn commonjs_first_native_route_keeps_worker_url_on_legacy_snapshot() {
    // Unrun: the Worker URL serializes factories, not native registrations.
    // Keep this graph on the existing renderer until Worker native parity.
    let root = temp_registry("cjs-first-worker-gate");
    let modules = root.join("node_modules");
    let package = modules.join("cjs-first-worker-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.cjs"), "var Worker = require('node:worker_threads').Worker; module.exports = new Worker(new URL('./worker.mjs', import.meta.url));").unwrap();
    fs::write(package.join("worker.mjs"), "export const value = 1;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "cjs-first-worker-pkg", &package, "index.cjs").unwrap();
    assert!(bundle.contains("__thaw_worker_bundle_source"));
    assert!(!bundle.contains("__thaw_register_native_bundle"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn commonjs_first_require_preserves_pure_native_namespace_identity_and_live_binding() {
    // Unrun: a pure ESM dependency keeps its engine namespace identity across
    // synchronous require calls, and a mutable export stays live.
    use std::ffi::{CStr, CString};
    let root = temp_registry("cjs-first-pure-namespace");
    let modules = root.join("node_modules");
    let package = modules.join("cjs-first-pure-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.cjs"), "var first = require('./pure.mjs'), second = require('./pure.mjs'); module.exports = [first === second, first.value, (first.bump(), second.value)];").unwrap();
    fs::write(package.join("pure.mjs"), "export let value = 1; export function bump() { value++; }").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "cjs-first-pure-pkg", &package, "index.cjs").unwrap();
    assert!(bundle.contains("evalNativeSync"));
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.pureNamespaceResult = function() {{ return module.exports; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"pureNamespaceResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[true,1,2]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn commonjs_created_require_keeps_computed_native_query_instances_distinct() {
    // Unrun: this suffix is requested after collection, through the public
    // created-require closure. It must not return the registered base module.
    use std::ffi::{CStr, CString};
    let root = temp_registry("cjs-first-native-query-instance");
    let modules = root.join("node_modules");
    let package = modules.join("query-instance-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.cjs"), "module.exports = require('./leaf.mjs');").unwrap();
    fs::write(package.join("leaf.mjs"), "globalThis.queryInstanceRuns = (globalThis.queryInstanceRuns || 0) + 1; export let value = 0; export function bump() { value++; }").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "query-instance-pkg", &package, "index.cjs").unwrap();
    assert!(bundle.contains("evalNativeSync"));
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.require = function(name) {{ throw new Error(name); }}; globalThis.queryInstanceRuns = 0; {bundle} var base = module.exports, made = globalThis.__thaw_bundle_create_require('query-instance-pkg/index.cjs'), one = made('./leaf.mjs?one'), two = made('./leaf.mjs?two'); one.bump(); globalThis.queryInstanceResult = function() {{ return [base === one, one === two, one === made('./leaf.mjs?one'), base.value, one.value, two.value, queryInstanceRuns]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"queryInstanceResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[false,false,true,0,1,0,3]");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn commonjs_created_require_native_query_clones_private_opaque_edges() {
    // Unrun: each native query instance gets its own private origin/edge
    // records, while the imported CommonJS factory still runs only once.
    use std::ffi::{CStr, CString};
    let root = temp_registry("cjs-first-native-query-opaque");
    let modules = root.join("node_modules");
    let package = modules.join("query-opaque-pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("index.cjs"), "globalThis.queryIndexRuns = (globalThis.queryIndexRuns || 0) + 1; exports.value = 1; var base = require('./leaf.mjs'); module.exports = { value: 2, base: base };").unwrap();
    fs::write(package.join("leaf.mjs"), "globalThis.queryOpaqueRuns = (globalThis.queryOpaqueRuns || 0) + 1; export * from './index.cjs'; export const token = {};").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "query-opaque-pkg", &package, "index.cjs").unwrap();
    assert!(bundle.contains("evalNativeSync"));
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.require = function(name) {{ throw new Error(name); }}; globalThis.queryOpaqueRuns = 0; globalThis.queryIndexRuns = 0; {bundle} var made = globalThis.__thaw_bundle_create_require('query-opaque-pkg/index.cjs'), base = module.exports.base, one = made('./leaf.mjs?one'), two = made('./leaf.mjs?two'); globalThis.queryOpaqueResult = function() {{ return [base.value, one.value, two.value, base === one, one === two, one === made('./leaf.mjs?one'), base.token === one.token, one.token === two.token, queryOpaqueRuns, queryIndexRuns]; }};");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"queryOpaqueResult".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[2,2,2,false,false,true,false,false,3,1]");
    let _ = fs::remove_dir_all(root);
}
