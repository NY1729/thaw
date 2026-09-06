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
        "module.exports = function () { return require('./value')(); };",
    )
    .unwrap();
    fs::write(
        dir.join("value.js"),
        "module.exports = function () { return 42; };",
    )
    .unwrap();
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules_dir, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let source = CString::new(format!(
            "globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle}"
        ))
        .unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);

    let result = thaw_quickjs::eval_json(
            "Function('require', __thaw_worker_bundle_source + \"return __thaw_bundle_create_require('pkg/index.js')('./value')();\")(function(name) { throw new Error(name); })",
        )
        .unwrap();
    assert_eq!(result.as_deref(), Some("42"));

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
