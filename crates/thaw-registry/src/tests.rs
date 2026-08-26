use super::*;

#[test]
fn selects_package_exports_conditions_for_runtime_and_types() {
    let manifest: serde_json::Value = serde_json::from_str(
        r#"{
                "main": "legacy.js",
                "types": "legacy.d.ts",
                "exports": {
                    ".": {
                        "types": "./dist/index.d.ts",
                        "import": "./dist/index.mjs",
                        "require": "./dist/index.cjs",
                        "default": "./dist/index.js"
                    },
                    "./feature": {
                        "types": "./dist/feature.d.ts",
                        "require": "./dist/feature.cjs"
                    }
                }
            }"#,
    )
    .unwrap();
    assert_eq!(
        package_export_target(&manifest, None, &["require", "import", "default"]),
        Some("./dist/index.cjs")
    );
    assert_eq!(
        package_export_target(&manifest, None, &["types"]),
        Some("./dist/index.d.ts")
    );
    assert_eq!(
        package_export_target(&manifest, Some("feature"), &["types"]),
        Some("./dist/feature.d.ts")
    );

    let array: serde_json::Value = serde_json::from_str(
        r#"{"exports":{".":[null,{"types":"./fallback.d.ts","require":"./fallback.cjs"}]}}"#,
    )
    .unwrap();
    assert_eq!(
        package_export_target(&array, None, &["types"]),
        Some("./fallback.d.ts")
    );
    assert_eq!(
        package_export_target(&array, None, &["require", "default"]),
        Some("./fallback.cjs")
    );
}

#[test]
fn expands_wildcard_package_exports_from_type_files() {
    let package_dir = temp_registry("wildcard-exports");
    fs::create_dir_all(package_dir.join("dist/features")).unwrap();
    fs::write(package_dir.join("dist/features/alpha.d.ts"), "").unwrap();
    fs::write(package_dir.join("dist/features/alpha.cjs"), "").unwrap();
    fs::write(package_dir.join("dist/features/beta.d.ts"), "").unwrap();
    fs::write(package_dir.join("dist/features/beta.cjs"), "").unwrap();
    let manifest: serde_json::Value = serde_json::from_str(
            r#"{"exports":{"./features/*":{"types":"./dist/features/*.d.ts","require":"./dist/features/*.cjs"}}}"#,
        )
        .unwrap();
    assert_eq!(
        package_subpath_exports(&manifest, &package_dir).unwrap(),
        vec![
            PackageSubpathExport {
                subpath: "features/alpha".to_string(),
                runtime_entry: "./dist/features/alpha.cjs".to_string(),
                types_entry: "./dist/features/alpha.d.ts".to_string(),
            },
            PackageSubpathExport {
                subpath: "features/beta".to_string(),
                runtime_entry: "./dist/features/beta.cjs".to_string(),
                types_entry: "./dist/features/beta.d.ts".to_string(),
            },
        ]
    );
    let _ = fs::remove_dir_all(package_dir);
}

#[test]
fn installed_npm_layout_registers_wildcard_subpath_artifacts() {
    let scratch = temp_registry("installed-wildcard-scratch");
    let registry = temp_registry("installed-wildcard-registry");
    let package = scratch.join("node_modules/feature-kit");
    fs::create_dir_all(package.join("dist/features")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{
                "name":"feature-kit",
                "version":"1.2.3",
                "types":"./index.d.ts",
                "main":"./index.js",
                "exports":{
                    ".":{"types":"./index.d.ts","require":"./index.js"},
                    "./features/*":{
                        "types":"./dist/features/*.d.ts",
                        "require":"./dist/features/*.js"
                    }
                }
            }"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "export declare function root(): number;",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { root: function() { return 1; } };",
    )
    .unwrap();
    fs::write(
        package.join("dist/features/double.d.ts"),
        "export default function double(value: number): number;",
    )
    .unwrap();
    fs::write(
        package.join("dist/features/double.js"),
        "module.exports = function(value) { return value * 2; };",
    )
    .unwrap();

    let added = add_installed(&registry, &scratch.join("node_modules"), "feature-kit").unwrap();
    assert_eq!(added.resolved_version, "1.2.3");
    let subpath = resolve(&registry, "feature-kit/features/double").unwrap();
    assert!(subpath.dts_source.contains("double"));
    assert!(subpath.bundle_js.unwrap().contains("value * 2"));
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn resolves_an_installed_package_subpath() {
    let registry = temp_registry("subpath");
    let dir = registry.join("math-kit/subpaths/advanced");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("package.d.ts"),
        "export declare function square(value: number): number;",
    )
    .unwrap();
    fs::write(
        dir.join("bundle.js"),
        "module.exports = { square: function(value) { return value * value; } };",
    )
    .unwrap();

    let package = resolve(&registry, "math-kit/advanced").unwrap();
    assert_eq!(package.name, "math-kit/advanced");
    assert!(package.dts_source.contains("square"));
    assert!(package.bundle_js.unwrap().contains("value * value"));
    let _ = fs::remove_dir_all(registry);
}

fn temp_registry(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "thaw-registry-test-{test_name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn resolves_a_package_with_all_runtime_backends() {
    let registry = temp_registry("full");
    let pkg_dir = registry.join("left-pad");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("package.d.ts"),
        "export declare function pad(s: string): string;",
    )
    .unwrap();
    fs::write(pkg_dir.join("native.a"), b"fake archive").unwrap();
    fs::write(pkg_dir.join("native.node"), b"fake addon").unwrap();
    fs::write(pkg_dir.join("bundle.js"), "function pad(s){return s;}").unwrap();
    fs::write(pkg_dir.join("version.txt"), "1.3.0").unwrap();

    let resolved = resolve(&registry, "left-pad").unwrap();
    assert_eq!(resolved.name, "left-pad");
    assert!(resolved.dts_source.contains("declare function pad"));
    assert_eq!(resolved.native_lib, Some(pkg_dir.join("native.a")));
    assert_eq!(resolved.native_addon, Some(pkg_dir.join("native.node")));
    assert_eq!(
        resolved.bundle_js.as_deref(),
        Some("function pad(s){return s;}")
    );
    assert_eq!(resolved.version.as_deref(), Some("1.3.0"));

    let _ = fs::remove_dir_all(&registry);
}

#[test]
fn selects_the_current_targets_bundled_node_prebuild() {
    let package = temp_registry("select_native_prebuild");
    let (platform, arch, libc) = target_prebuild_components();
    let target = package.join("prebuilds").join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target).unwrap();
    let filename = if libc == "musl" {
        "binding.musl.node"
    } else {
        "binding.node"
    };
    fs::write(target.join(filename), b"native bytes").unwrap();
    // The opposite Linux libc must not be selected accidentally.
    if platform == "linux" {
        let opposite = if libc == "musl" {
            "binding.node"
        } else {
            "binding.musl.node"
        };
        fs::write(target.join(opposite), b"wrong libc").unwrap();
    }

    let selected = select_prebuilt_addon(&package).unwrap().unwrap();
    assert_eq!(selected.path, target.join(filename));
    assert_eq!(selected.platform, platform);
    assert_eq!(selected.arch, arch);
    assert_eq!(selected.libc, libc);
    let _ = fs::remove_dir_all(package);
}

#[test]
fn builds_prebuild_install_github_asset_for_the_current_target() {
    let manifest = serde_json::json!({
        "name": "sqlite3",
        "version": "5.1.7",
        "repository": {
            "type": "git",
            "url": "git+https://github.com/TryGhost/node-sqlite3.git"
        },
        "binary": { "napi_versions": [3, 6, 99] }
    });
    let (url, asset, platform, arch) = prebuild_install_asset(&manifest).unwrap();
    let (target_platform, target_arch, libc) = target_prebuild_components();
    let asset_platform = if target_platform == "linux" && libc == "musl" {
        "linuxmusl"
    } else {
        target_platform
    };
    assert_eq!(
        asset,
        format!("sqlite3-v5.1.7-napi-v6-{asset_platform}-{target_arch}.tar.gz")
    );
    assert_eq!(
        url,
        format!("https://github.com/TryGhost/node-sqlite3/releases/download/v5.1.7/{asset}")
    );
    assert_eq!(platform, asset_platform);
    assert_eq!(arch, target_arch);
}

#[test]
fn reports_available_targets_when_no_prebuild_matches() {
    let package = temp_registry("mismatched_native_prebuild");
    let target = package.join("prebuilds/imaginary-other");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("binding.node"), b"native bytes").unwrap();
    let diagnostic = select_prebuilt_addon(&package).unwrap_err();
    assert!(diagnostic.contains("no bundled native addon matches"));
    assert!(diagnostic.contains("imaginary-other"));
    let _ = fs::remove_dir_all(package);
}

#[test]
fn selects_a_platform_optional_dependency_node_addon() {
    let node_modules = temp_registry("optional_native_prebuild");
    let (platform, arch, libc) = target_prebuild_components();
    let dependency = if platform == "linux" {
        format!("@example/addon-{platform}-{arch}-{libc}")
    } else {
        format!("@example/addon-{platform}-{arch}")
    };
    let dependency_dir = node_modules.join(&dependency);
    fs::create_dir_all(&dependency_dir).unwrap();
    fs::write(
        dependency_dir.join("package.json"),
        format!(r#"{{"name":"{dependency}","main":"binding.node"}}"#),
    )
    .unwrap();
    fs::write(dependency_dir.join("binding.node"), b"native bytes").unwrap();
    let manifest = serde_json::json!({
        "optionalDependencies": { dependency.clone(): "1.0.0" }
    });
    let selected = select_optional_dependency_addon(&node_modules, &manifest)
        .unwrap()
        .unwrap();
    assert_eq!(selected.path, dependency_dir.join("binding.node"));
    assert_eq!(selected.source, format!("{dependency}/binding.node"));
    assert_eq!(selected.platform, platform);
    assert_eq!(selected.arch, arch);
    assert_eq!(selected.libc, libc);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn resolves_a_packages_lock_json_when_present() {
    let registry = temp_registry("with_lock");
    let pkg_dir = registry.join("qs");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("package.d.ts"),
        "export declare function stringify(x: string): string;",
    )
    .unwrap();
    fs::write(
        pkg_dir.join("bundle.js"),
        "function stringify(x){return x;}",
    )
    .unwrap();
    fs::write(pkg_dir.join("version.txt"), "6.11.0").unwrap();
    fs::write(
        pkg_dir.join("lock.json"),
        r#"{"qs": "6.11.0", "side-channel": "1.0.4"}"#,
    )
    .unwrap();

    let resolved = resolve(&registry, "qs").unwrap();
    let deps = resolved.dependency_versions.expect("lock.json was written");
    assert_eq!(deps.get("qs").map(String::as_str), Some("6.11.0"));
    assert_eq!(deps.get("side-channel").map(String::as_str), Some("1.0.4"));

    let _ = fs::remove_dir_all(&registry);
}

#[test]
fn resolves_a_package_with_only_the_required_dts() {
    let registry = temp_registry("dts_only");
    let pkg_dir = registry.join("is-odd");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("package.d.ts"),
        "export declare function isOdd(n: number): boolean;",
    )
    .unwrap();

    let resolved = resolve(&registry, "is-odd").unwrap();
    assert!(resolved.native_lib.is_none());
    assert!(resolved.bundle_js.is_none());
    // A hand-curated package (or one `add`ed before `version.txt`
    // existed) has no version on record -- not an error, just unknown.
    assert!(resolved.version.is_none());

    let _ = fs::remove_dir_all(&registry);
}

#[test]
fn errors_when_package_dts_is_missing() {
    let registry = temp_registry("missing_dts");
    let pkg_dir = registry.join("ghost");
    fs::create_dir_all(&pkg_dir).unwrap();

    let err = resolve(&registry, "ghost").unwrap_err();
    assert!(err.contains("ghost"));
    assert!(err.contains("package.d.ts"));

    let _ = fs::remove_dir_all(&registry);
}

#[test]
fn errors_when_package_directory_does_not_exist() {
    let registry = temp_registry("no_such_pkg");
    let err = resolve(&registry, "nonexistent").unwrap_err();
    assert!(err.contains("nonexistent"));

    let _ = fs::remove_dir_all(&registry);
}

/// `add`'s actual `npm install` step needs network access and isn't
/// exercised by the automated suite (consistent with this project's
/// other network-touching work, which was validated manually rather
/// than in `cargo test` -- see docs/design/registry.md). `find_own_dts`/
/// `types_package_name` are the pieces of `add` with real decision
/// logic and no network dependency, so they get full offline coverage
/// here.
#[test]
fn finds_dts_from_types_field() {
    let manifest: serde_json::Value =
        serde_json::from_str(r#"{"types": "dist/index.d.ts"}"#).unwrap();
    let dir = temp_registry("dts_types_field");
    let (rel, abs) = find_own_dts(&manifest, &dir).unwrap();
    assert_eq!(rel, "dist/index.d.ts");
    assert_eq!(abs, dir.join("dist/index.d.ts"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn finds_dts_from_typings_field_when_types_is_absent() {
    let manifest: serde_json::Value = serde_json::from_str(r#"{"typings": "index.d.ts"}"#).unwrap();
    let dir = temp_registry("dts_typings_field");
    let (rel, _) = find_own_dts(&manifest, &dir).unwrap();
    assert_eq!(rel, "index.d.ts");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prefers_types_field_over_typings_field() {
    let manifest: serde_json::Value =
        serde_json::from_str(r#"{"types": "a.d.ts", "typings": "b.d.ts"}"#).unwrap();
    let dir = temp_registry("dts_prefers_types");
    let (rel, _) = find_own_dts(&manifest, &dir).unwrap();
    assert_eq!(rel, "a.d.ts");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn falls_back_to_index_d_ts_when_no_field_is_present() {
    let manifest: serde_json::Value = serde_json::from_str(r#"{"main": "index.js"}"#).unwrap();
    let dir = temp_registry("dts_index_fallback");
    fs::write(dir.join("index.d.ts"), "declare function f(): void;").unwrap();
    let (rel, _) = find_own_dts(&manifest, &dir).unwrap();
    assert_eq!(rel, "index.d.ts");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn finds_no_dts_when_nothing_is_bundled() {
    let manifest: serde_json::Value = serde_json::from_str(r#"{"main": "index.js"}"#).unwrap();
    let dir = temp_registry("dts_none");
    assert!(find_own_dts(&manifest, &dir).is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn types_package_name_for_an_unscoped_package() {
    assert_eq!(types_package_name("left-pad"), "@types/left-pad");
}

#[test]
fn types_package_name_for_a_scoped_package() {
    assert_eq!(types_package_name("@babel/core"), "@types/babel__core");
}

/// Found by running a real npm package (`ms`, `"main": "./index"`)
/// through `add`: reading the literal `main` string as a path fails
/// since Node resolves the missing `.js` extension at require-time,
/// which we don't get for free.
#[test]
fn resolves_main_field_missing_its_extension() {
    let dir = temp_registry("main_no_extension");
    fs::write(dir.join("index.js"), "module.exports = 1;").unwrap();
    let (rel, abs) = resolve_module_path(&dir, "./index").unwrap();
    assert_eq!(rel, "./index.js");
    assert_eq!(abs, dir.join("index.js"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolves_main_field_pointing_at_a_directory() {
    let dir = temp_registry("main_directory");
    fs::create_dir_all(dir.join("lib")).unwrap();
    fs::write(dir.join("lib/index.js"), "module.exports = 1;").unwrap();
    let (rel, abs) = resolve_module_path(&dir, "./lib").unwrap();
    assert_eq!(rel, "./lib/index.js");
    assert_eq!(abs, dir.join("lib/index.js"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolves_a_required_directory_through_its_own_package_manifest() {
    let dir = temp_registry("nested_directory_manifest");
    let feature = dir.join("feature");
    fs::create_dir_all(feature.join("dist")).unwrap();
    fs::write(
        feature.join("package.json"),
        r#"{"exports":{".":{"require":"./dist/index.cjs"}}}"#,
    )
    .unwrap();
    fs::write(feature.join("dist/index.cjs"), "module.exports = 42;").unwrap();
    let (relative, absolute) = resolve_module_path(&dir, "./feature").unwrap();
    assert_eq!(relative, "feature/dist/index.cjs");
    assert_eq!(absolute, feature.join("dist/index.cjs"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolves_main_field_written_exactly() {
    let dir = temp_registry("main_exact");
    fs::write(dir.join("main.js"), "module.exports = 1;").unwrap();
    let (rel, abs) = resolve_module_path(&dir, "main.js").unwrap();
    assert_eq!(rel, "main.js");
    assert_eq!(abs, dir.join("main.js"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn errors_with_all_tried_candidates_when_main_cannot_be_resolved() {
    let dir = temp_registry("main_missing");
    let err = resolve_module_path(&dir, "./index").unwrap_err();
    assert!(err.contains("./index"));
    assert!(err.contains("./index.js"));
    assert!(err.contains("./index/index.js"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn finds_single_and_double_quoted_relative_requires() {
    let specs: Vec<_> =
        find_module_specs(r#"var a = require('./a'); var b = require("../lib/b");"#)
            .into_iter()
            .filter(|spec| spec.starts_with('.'))
            .collect();
    assert_eq!(specs, vec!["./a".to_string(), "../lib/b".to_string()]);
}

#[test]
fn finds_literal_dependencies_called_through_create_require_aliases() {
    assert_eq!(
        find_module_specs(
            "import { createRequire as makeRequire } from 'node:module';\n\
                 import * as Module from 'module';\n\
                 const local = makeRequire('pkg/index.js');\n\
                 const other = Module.createRequire('pkg/index.js');\n\
                 local('./dependency'); other(`./other`);"
        ),
        vec!["./dependency", "./other", "node:module", "module"]
    );
}

#[test]
fn parser_collects_esm_reexports_and_dynamic_imports_without_false_positives() {
    let specs = find_module_specs(
        r#"
                import main from './main.js';
                export { value } from "./value.js";
                export * from './all.js';
                const package = import(`external-package`);
                const feature = import(("external-" + "feature"));
                const later = import('./later.js');
                const text = "require('./not-real.js')";
                // require('./also-not-real.js')
                object.require('./member.js');
                require(variable);
            "#,
    );
    assert_eq!(
        specs,
        vec![
            "external-package",
            "external-feature",
            "./later.js",
            "./main.js",
            "./value.js",
            "./all.js"
        ]
    );
}

#[test]
fn dynamic_import_candidate_expansion_is_bounded() {
    let analysis = analyze_module(
            "import((a ? 'a' : 'b') + (b ? 'a' : 'b') + (c ? 'a' : 'b') + (d ? 'a' : 'b') + (e ? 'a' : 'b') + (f ? 'a' : 'b') + (g ? 'a' : 'b'));",
        );
    assert!(analysis.has_nonliteral_dynamic_import);
    assert!(analysis.specs.is_empty());
}

#[test]
fn parser_identifies_commonjs_export_assignments() {
    let analysis = analyze_module(
        r#"
                exports.alpha = 1;
                module.exports.beta = 2;
                module.exports["gamma"] = 3;
                module.exports = function () {};
                object.exports.nope = 4;
            "#,
    );
    assert_eq!(
        analysis._commonjs_exports,
        vec!["alpha", "beta", "default", "gamma"]
    );
}

#[test]
fn rewrites_literal_dynamic_import_to_an_async_bundle_require() {
    let rewritten =
        rewrite_esm_to_commonjs("function load() { return import('./feature.js'); }").unwrap();
    assert!(rewritten.contains("requireAsync(String('./feature.js'))"));
    assert!(!rewritten.contains("import("));
}

#[test]
fn ignores_bare_specifier_requires() {
    let specs: Vec<_> = find_module_specs(r#"var x = require('is-number');"#)
        .into_iter()
        .filter(|spec| spec.starts_with('.'))
        .collect();
    assert!(specs.is_empty());
}

#[test]
fn finds_bare_specifiers_including_scoped_packages() {
    let specs: Vec<_> = find_module_specs(
            r#"var a = require('side-channel'); var b = require('@babel/core'); var c = require('./local');"#,
        )
        .into_iter()
        .filter(|spec| !spec.starts_with('.'))
        .collect();
    assert_eq!(
        specs,
        vec!["side-channel".to_string(), "@babel/core".to_string()]
    );
}

#[test]
fn splits_bare_specs_into_package_and_subpath() {
    assert_eq!(split_bare_spec("lodash"), ("lodash", None));
    assert_eq!(split_bare_spec("lodash/fp"), ("lodash", Some("fp")));
    assert_eq!(
        split_bare_spec("es-errors/type"),
        ("es-errors", Some("type"))
    );
    assert_eq!(split_bare_spec("@babel/core"), ("@babel/core", None));
    assert_eq!(
        split_bare_spec("@babel/core/lib/index"),
        ("@babel/core", Some("lib/index"))
    );
}

#[test]
fn splits_version_specs_from_add_arguments() {
    assert_eq!(split_package_spec("left-pad"), ("left-pad", None));
    assert_eq!(
        split_package_spec("left-pad@1.3.0"),
        ("left-pad", Some("1.3.0"))
    );
    assert_eq!(
        split_package_spec("left-pad@^1.2.0"),
        ("left-pad", Some("^1.2.0"))
    );
    assert_eq!(
        split_package_spec("left-pad@next"),
        ("left-pad", Some("next"))
    );
    // A scoped package's leading `@scope/` is never mistaken for a
    // version separator -- only an `@` after the scope's own `/`
    // starts one.
    assert_eq!(split_package_spec("@hapi/hoek"), ("@hapi/hoek", None));
    assert_eq!(
        split_package_spec("@hapi/hoek@9.0.0"),
        ("@hapi/hoek", Some("9.0.0"))
    );
}

/// The exact shape found in `qs`'s own real transitive dependency
/// chain: `require('es-errors/type')`, a "deep import" subpath into
/// another package, resolved directly against that package's root
/// (not through its `main` field).
#[test]
fn resolves_a_deep_import_subpath_into_a_dependency() {
    let node_modules = temp_registry("deep_import_node_modules");
    fs::create_dir_all(node_modules.join("es-errors")).unwrap();
    fs::write(
        node_modules.join("es-errors/package.json"),
        r#"{"main": "index.js"}"#,
    )
    .unwrap();
    fs::write(
        node_modules.join("es-errors/index.js"),
        "module.exports = {};",
    )
    .unwrap();
    fs::write(
        node_modules.join("es-errors/type.js"),
        "module.exports = TypeError;",
    )
    .unwrap();

    let (name, relative, abs, dir) = resolve_bare_require(&node_modules, "es-errors/type").unwrap();
    assert_eq!(name, "es-errors");
    assert_eq!(relative, "type.js");
    assert_eq!(abs, node_modules.join("es-errors/type.js"));
    assert_eq!(dir, node_modules.join("es-errors"));

    let _ = fs::remove_dir_all(&node_modules);
}

#[test]
fn resolves_a_deep_import_into_a_scoped_package() {
    let node_modules = temp_registry("deep_import_scoped_node_modules");
    fs::create_dir_all(node_modules.join("@scope/pkg/lib")).unwrap();
    fs::write(
        node_modules.join("@scope/pkg/lib/util.js"),
        "module.exports = 1;",
    )
    .unwrap();

    let (name, relative, ..) = resolve_bare_require(&node_modules, "@scope/pkg/lib/util").unwrap();
    assert_eq!(name, "@scope/pkg");
    assert_eq!(relative, "lib/util.js");

    let _ = fs::remove_dir_all(&node_modules);
}

#[test]
fn ignores_dynamic_and_malformed_require_calls() {
    // `require(name)` (a variable, not a literal) and a stray
    // "require" that isn't actually a call must not confuse the scan
    // -- and must not stop it from still finding a real one after.
    let specs: Vec<_> = find_module_specs(
        "var x = require(name); var note = 'requirements'; var y = require('./y');",
    )
    .into_iter()
    .filter(|spec| spec.starts_with('.'))
    .collect();
    assert_eq!(specs, vec!["./y".to_string()]);
}

#[test]
fn normalizes_dot_and_dot_dot_segments() {
    assert_eq!(normalize_path_string("lib/./stringify"), "lib/stringify");
    assert_eq!(normalize_path_string("lib/../parse"), "parse");
    assert_eq!(normalize_path_string("a/b/../../c"), "c");
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

/// The exact pattern found in a real ESM npm package (`has-flag`):
/// `import process from 'process'`, then reading `process.argv`.
#[test]
fn process_builtin_polyfill_actually_runs_through_quickjs() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_process");
    fs::write(
        dir.join("index.js"),
        "import process from 'process';\n\
             export default function getPlatform() { return process.platform; }",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_process_node_modules");

    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.getPlatform = module.exports.default;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "failed to load"
    );

    let func = CString::new("getPlatform").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(result, "\"linux\"");

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn querystring_builtin_runs_through_bundled_commonjs_require() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_querystring");
    fs::write(
            dir.join("index.js"),
            "var querystring = require('querystring');\n\
             module.exports = function () {\n\
             \x20 return querystring.stringify({ a: [1, 2], space: 'two words' }) + ':' + JSON.stringify(querystring.parse('x=1&x=2'));\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_querystring_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseQuerystring = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseQuerystring").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#""a=1&a=2&space=two%20words:{\"x\":[\"1\",\"2\"]}""#
    );

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn events_builtin_runs_event_emitter_through_commonjs_require() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_events");
    fs::write(
            dir.join("index.js"),
            "var EventEmitter = require('events');\n\
             function Child() { EventEmitter.call(this); }\n\
             Child.prototype = Object.create(EventEmitter.prototype);\n\
             Child.prototype.constructor = Child;\n\
             module.exports = function () {\n\
             \x20 var emitter = new Child(); var seen = [];\n\
             \x20 function regular(value) { seen.push('regular:' + value); }\n\
             \x20 emitter.on('value', regular);\n\
             \x20 emitter.prependOnceListener('value', function(value) { seen.push('once:' + value); });\n\
             \x20 var first = emitter.emit('value', 1); var second = emitter.emit('value', 2);\n\
             \x20 emitter.off('value', regular); var third = emitter.emit('value', 3);\n\
             \x20 return [seen.join(','), first, second, third, emitter.listenerCount('value'), emitter.eventNames().length];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_events_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseEvents = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseEvents").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["once:1,regular:1,regular:2",true,true,false,0,0]"#
    );

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn events_builtin_supports_symbols_async_iteration_and_abort() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_events_async");
    fs::write(
            dir.join("index.js"),
            "var events = require('node:events'); module.exports = async function () { var emitter = new events.EventEmitter(), symbol = Symbol('value'), observed = []; emitter.on(symbol, function(value) { observed.push(value); }); emitter.emit(symbol, 1); var iterator = events.on(emitter, 'data'); emitter.emit('data', 'first', 2); emitter.emit('data', 'second', 3); var first = await iterator.next(), second = await iterator.next(), returned = await iterator.return(); var controller = new AbortController(), aborted = events.on(emitter, 'wait', { signal: controller.signal }), error; var pending = aborted.next().catch(function(value) { error = [value.name, value.code, value.cause]; }); controller.abort('reason'); await pending; var onceController = new AbortController(), onceError; var oncePending = events.once(emitter, 'never', { signal: onceController.signal }).catch(function(value) { onceError = [value.name, value.code, value.cause]; }); onceController.abort('once-reason'); await oncePending; events.setMaxListeners(4, emitter); var listener = function() {}; emitter.on('probe', listener); return [observed, first.value, second.value, returned.done, emitter.listenerCount('data'), emitter.eventNames().some(function(value) { return value === symbol; }), events.getEventListeners(emitter, 'probe')[0] === listener, emitter.getMaxListeners(), events.getMaxListeners(emitter), error, onceError, typeof events.errorMonitor, typeof events.captureRejectionSymbol]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_events_async_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseEventsAsync = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseEventsAsync").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[1],["first",2],["second",3],true,0,true,true,4,4,["AbortError","ABORT_ERR","reason"],["AbortError","ABORT_ERR","once-reason"],"symbol","symbol"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn assert_builtin_reports_structured_failures_through_commonjs_require() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_assert");
    fs::write(
            dir.join("index.js"),
            "var assert = require('node:assert/strict');\n\
             module.exports = function () {\n\
             \x20 assert.deepStrictEqual({ a: [1, 2], date: new Date(3) }, { date: new Date(3), a: [1, 2] });\n\
             \x20 assert.throws(function() { throw new TypeError('bad value'); }, /bad/);\n\
             \x20 var failure; try { assert.strictEqual(1, 2, 'different'); } catch (error) { failure = [error instanceof assert.AssertionError, error.name, error.code, error.actual, error.expected, error.operator, error.message]; }\n\
             \x20 return failure;\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_assert_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseAssert = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseAssert").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"AssertionError","ERR_ASSERTION",1,2,"strictEqual","different"]"#
    );

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

/// Not just plausible text -- runs through the real QuickJS-NG
/// engine, confirming `util.inspect.custom` actually comes back as a
/// real `Symbol` (what `object-inspect` needs it to be), not merely
/// present.
#[test]
fn util_polyfill_actually_runs_through_quickjs() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_util_runs");
    fs::write(
        dir.join("index.js"),
        "var inspect = require('util').inspect;\n\
             module.exports = function () { return typeof inspect.custom; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_util_runs_node_modules");

    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.checkInspectCustom = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "bundle failed to load"
    );

    let func = CString::new("checkInspectCustom").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(result, "\"symbol\"");

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn util_helpers_run_through_bundled_commonjs_require() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_util_helpers");
    fs::write(
            dir.join("index.js"),
            "var util = require('node:util');\n\
             function Base() {} function Child() {} util.inherits(Child, Base);\n\
             module.exports = async function () {\n\
             \x20 var custom = {}; custom[util.inspect.custom] = function() { return 'custom'; };\n\
             \x20 var add = util.promisify(function(a, b, callback) { queueMicrotask(function() { callback(null, a + b); }); });\n\
             \x20 var sum = await add(2, 3);\n\
             \x20 var callbackValue = await new Promise(function(resolve, reject) { util.callbackify(async function(value) { return value * 2; })(4, function(error, value) { if (error) reject(error); else resolve(value); }); });\n\
             \x20 return [util.format('%s:%d:%j:%%', 'value', 4, { ok: true }), util.inspect(custom), Child.super_ === Base, new Child() instanceof Base, util.types.isDate(new Date()), util.types.isRegExp(/x/), util.types.isMap(new Map()), util.types.isTypedArray(new Uint8Array(1)), sum, callbackValue, util.stripVTControlCharacters('\\u001b[31mred\\u001b[0m'), util.toUSVString('x\\ud800y')];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_util_helpers_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseUtil = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseUtil").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["value:4:{\"ok\":true}:%","custom",true,true,true,true,true,true,5,8,"red","x�y"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn util_parse_args_handles_long_short_multiple_negative_and_tokens() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_util_parse_args");
    fs::write(
            dir.join("index.js"),
            "var util = require('node:util'); module.exports = function () { var result = util.parseArgs({ args: ['-v', '--name=thaw', '--tag', 'one', '--tag=two', '--no-color', 'input.ts', '--', '-literal'], options: { verbose: { type: 'boolean', short: 'v' }, name: { type: 'string' }, tag: { type: 'string', multiple: true }, color: { type: 'boolean', default: true } }, allowNegative: true, allowPositionals: true, tokens: true }); var unknown; try { util.parseArgs({ args: ['--missing'], options: {} }); } catch (error) { unknown = error.code; } return [result.values.verbose, result.values.name, result.values.tag, result.values.color, result.positionals, result.tokens.map(function(token) { return token.kind; }), unknown, new util.TextEncoder().encode('ok').length, new util.TextDecoder().decode(Uint8Array.of(111, 107))]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_util_parse_args_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseUtilParseArgs = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseUtilParseArgs").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"thaw",["one","two"],false,["input.ts","-literal"],["option","option","option","option","option","positional","option-terminator","positional"],"ERR_PARSE_ARGS_UNKNOWN_OPTION",2,"ok"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn buffer_builtin_shares_the_global_buffer_implementation() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_buffer");
    fs::write(
            dir.join("index.js"),
            "var buffer = require('node:buffer');\n\
             module.exports = function () {\n\
             \x20 var value = buffer.Buffer.from('雪', 'utf8'); var invalid = buffer.Buffer.from([0xff]);\n\
             \x20 return [buffer.Buffer === globalThis.Buffer, value.toString('hex'), value.toString('base64'), buffer.byteLength('雪'), buffer.isUtf8(value), buffer.isUtf8(invalid), buffer.isAscii(buffer.Buffer.from('abc')), buffer.transcode(buffer.Buffer.from('hi'), 'utf8', 'utf16le').toString('hex')];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_buffer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseBuffer = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseBuffer").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"e99baa","6Zuq",3,true,false,true,"68006900"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn path_builtin_exposes_posix_and_win32_operations() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_path_operations");
    fs::write(
            dir.join("index.js"),
            "var path = require('node:path');\n\
             module.exports = function () {\n\
             \x20 var parsed = path.parse('/tmp/archive.tar.gz');\n\
             \x20 return [path.normalize('/a//b/../c/'), path.relative('/a/b', '/a/c/d'), path.extname('archive.tar.gz'), path.basename('archive.tar.gz', '.gz'), parsed, path.format(parsed), path.isAbsolute('/a'), path.posix === path, path.win32.normalize('C:\\\\a\\\\..\\\\b'), path.win32.isAbsolute('C:\\\\a'), path.delimiter, path.win32.delimiter];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_path_operations_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exercisePath = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exercisePath").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["/a/c/","../c/d",".gz","archive.tar",{"root":"/","dir":"/tmp","base":"archive.tar.gz","ext":".gz","name":"archive.tar"},"/tmp/archive.tar.gz",true,true,"C:\\b",true,":",";"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn legacy_builtin_aliases_and_domains_preserve_node_behavior() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_legacy_aliases_domain");
    fs::write(
            dir.join("index.js"),
            "var path = require('node:path'), posix = require('path/posix'), win32 = require('node:path/win32'), util = require('node:util'), sys = require('sys'), domain = require('node:domain'), EventEmitter = require('node:events'); module.exports = function () { var errors = [], values = [], active = false, d = domain.create(); d.on('error', function(error) { errors.push([error.message, error.domain === d, error.domainThrown]); }); d.run(function() { active = domain.active === d && process.domain === d; throw new Error('run'); }); var receiver = { value: 4, callback: d.bind(function(extra) { values.push(this.value + extra); throw new Error('bound'); }) }; receiver.callback(3); var intercepted = d.intercept(function(value) { values.push(value); }); intercepted(new Error('intercepted')); intercepted(null, 9); var emitter = new EventEmitter(); d.add(emitter); var added = emitter.domain === d && d.members[0] === emitter; d.remove(emitter); return [posix === path.posix, win32 === path.win32, sys === util, posix.join('a', 'b'), win32.join('C:\\\\a', 'b'), active, domain.active, process.domain, errors, values, added, emitter.domain, d.members.length]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_legacy_aliases_domain_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 8);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseLegacyBuiltins = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseLegacyBuiltins").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,"a/b","C:\\a\\b",true,null,null,[["run",true,true],["bound",true,true],["intercepted",true,true]],[7,9],true,null,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn internal_stream_aliases_and_trace_categories_share_state() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_stream_aliases_trace");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), Readable = require('_stream_readable'), Writable = require('node:_stream_writable'), Duplex = require('_stream_duplex'), Transform = require('_stream_transform'), PassThrough = require('_stream_passthrough'), StreamWrap = require('_stream_wrap'), trace = require('node:trace_events'); module.exports = function () { var first = trace.createTracing({ categories: ['node', 'v8', 'v8'] }), second = new trace.Tracing({ categories: ['v8', 'custom'] }), states = [first.enabled, trace.getEnabledCategories()]; first.enable(); first.enable(); states.push(first.enabled, trace.getEnabledCategories()); second.enable(); states.push(trace.getEnabledCategories()); first.disable(); states.push(first.enabled, trace.getEnabledCategories()); second.disable(); states.push(trace.getEnabledCategories()); return [Readable === stream.Readable, Writable === stream.Writable, Duplex === stream.Duplex, Transform === stream.Transform, PassThrough === stream.PassThrough, StreamWrap === stream.Duplex, first.categories, second.categories, states]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_aliases_trace_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 9);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTraceAliases = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseTraceAliases").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,true,true,true,["node","v8"],["v8","custom"],[false,"",true,"node,v8","custom,node,v8",false,"custom,v8",""]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn inspector_sessions_evaluate_through_callback_and_promise_protocols() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_inspector_sessions");
    fs::write(
            dir.join("index.js"),
            "var inspector = require('node:inspector'), promises = require('node:inspector/promises'); function post(session, method, params) { return new Promise(function(resolve) { session.post(method, params, function(error, result) { resolve(error ? { error: error.code } : result); }); }); } module.exports = async function () { var before = inspector.url(), disposable = inspector.open(9333, 'localhost'), opened = inspector.url(), callbackSession = new inspector.Session(); callbackSession.connect(); var evaluated = await post(callbackSession, 'Runtime.evaluate', { expression: '6 * 7', returnByValue: true }), exception = await post(callbackSession, 'Runtime.evaluate', { expression: 'throw new Error(\"boom\")' }), isolate = await post(callbackSession, 'Runtime.getIsolateId'), schema = await post(callbackSession, 'Schema.getDomains'), unsupported = await post(callbackSession, 'Network.enable'); callbackSession.disconnect(); var disconnected = await post(callbackSession, 'Runtime.enable'), promiseSession = new promises.Session(); promiseSession.connectToMainThread(); var object = await promiseSession.post('Runtime.evaluate', { expression: '({ answer: 42 })', returnByValue: true }); await promiseSession.post('Debugger.enable'); promiseSession.disconnect(); disposable.dispose(); return [before, opened, inspector.url(), evaluated.result, exception.exceptionDetails.text, isolate.id, schema.domains.map(function(value) { return value.name; }), unsupported.error, disconnected.error, object.result.value, inspector.console === console]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_inspector_sessions_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseInspector = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseInspector").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[null,"ws://localhost:9333/thaw",null,{"type":"number","value":42},"boom","thaw-quickjs-main",["Runtime","Debugger","Profiler","HeapProfiler","Schema"],"ERR_INSPECTOR_COMMAND","ERR_INSPECTOR_NOT_CONNECTED",{"answer":42},true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn repl_server_evaluates_input_and_drives_commands() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_repl_server");
    fs::write(
            dir.join("index.js"),
            "var repl = require('node:repl'); module.exports = function () { var output = { text: '', write: function(value) { this.text += value; } }, events = [], server = repl.start({ prompt: 'thaw> ', output: output, historySize: 2, replMode: repl.REPL_MODE_STRICT, writer: function(value) { return '<' + String(value) + '>'; } }); server.on('result', function(value) { events.push(value); }); server.on('exit', function() { events.push('exit'); }); server.defineCommand('double', { help: 'double a value', action: function(value) { this.write(String(Number(value) * 2)); } }); server.write('20 + 22'); server.write('.double 5'); server.write('1'); server.write('2'); var history = server.history.slice(); server.setPrompt('next> '); server.displayPrompt(); var setup = false; server.setupHistory('/tmp/history', function(error) { setup = !error; }); server.write('.exit'); return Promise.resolve().then(function() { return [server instanceof repl.REPLServer, server.closed, server.last, server.context._, history, events, setup, output.text, typeof repl.writer({ answer: 42 }), repl._builtinLibs.length, repl.REPL_MODE_SLOPPY !== repl.REPL_MODE_STRICT]; }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_repl_server_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseRepl = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseRepl").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,2,2,["2","1"],[42,10,1,2,"exit"],true,"thaw> <42>\nthaw> <10>\nthaw> <1>\nthaw> <2>\nthaw> next> ","string",0,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn cluster_primary_tracks_worker_lifecycle_and_messages() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_cluster_primary");
    fs::write(
            dir.join("index.js"),
            "var cluster = require('node:cluster'); module.exports = async function () { var events = []; cluster.on('setup', function(settings) { events.push('setup:' + settings.exec); }); cluster.on('fork', function(worker) { events.push('fork:' + worker.id); }); cluster.on('online', function(worker) { events.push('online:' + worker.id); }); cluster.on('message', function(worker, message) { events.push('cluster-message:' + message.value); }); cluster.on('disconnect', function(worker) { events.push('cluster-disconnect:' + worker.id); }); cluster.on('exit', function(worker, code, signal) { events.push('cluster-exit:' + signal); }); cluster.setupPrimary({ exec: 'service.js', args: ['--serve'] }); cluster.schedulingPolicy = cluster.SCHED_NONE; var worker = cluster.fork({ PORT: '8080' }); worker.on('online', function() { events.push('worker-online'); }); worker.on('message', function(message) { events.push('worker-message:' + message.value); }); worker.on('disconnect', function() { events.push('worker-disconnect'); }); worker.on('exit', function(code, signal) { events.push('worker-exit:' + signal); }); worker.send({ value: 42 }, function(error) { events.push(error ? error.code : 'sent'); }); await new Promise(function(resolve) { cluster.disconnect(function() { events.push('all-disconnected'); resolve(); }); }); var connected = worker.isConnected(), exitedAfterDisconnect = worker.exitedAfterDisconnect; worker.kill('SIGKILL'); await Promise.resolve(); return [cluster.isPrimary, cluster.isMaster, cluster.isWorker, cluster.settings.exec, cluster.schedulingPolicy, worker.environment.PORT, connected, exitedAfterDisconnect, worker.isDead(), Object.keys(cluster.workers).length, events]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_cluster_primary_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseCluster = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseCluster").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,false,"service.js",1,"8080",false,true,true,0,["setup:service.js","fork:1","worker-online","online:1","worker-message:42","cluster-message:42","sent","worker-disconnect","cluster-disconnect:1","all-disconnected","worker-exit:SIGKILL","cluster-exit:SIGKILL"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn internal_http_and_tls_modules_share_public_implementations() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_internal_http_tls");
    fs::write(
            dir.join("index.js"),
            "var http = require('node:http'), tls = require('node:tls'), agent = require('_http_agent'), client = require('node:_http_client'), common = require('_http_common'), incoming = require('_http_incoming'), outgoing = require('_http_outgoing'), server = require('_http_server'), tlsCommon = require('_tls_common'), tlsWrap = require('_tls_wrap'); module.exports = function () { var context = tlsCommon.createSecureContext({ minVersion: 'TLSv1.2' }); context.setKey('key'); context.setCert('cert'); context.addCACert('ca'); var parser = common.parsers.alloc(); parser.initialize(common.HTTPParser.RESPONSE); common.parsers.free(parser); return [agent.Agent === http.Agent, agent.globalAgent === http.globalAgent, client.ClientRequest === http.ClientRequest, incoming.IncomingMessage === http.IncomingMessage, outgoing.OutgoingMessage === http.ServerResponse, server.Server === http.Server, server.ServerResponse === http.ServerResponse, tlsWrap.TLSSocket === tls.TLSSocket, tlsCommon.SecureContext === tls.SecureContext, context instanceof tls.SecureContext, context.options, common.methods === http.METHODS, common._checkIsHttpToken('x-test'), common._checkIsHttpToken('bad header'), common._checkInvalidHeaderChar('ok\\nno'), common.kLenientAll, common.parsers.list.length]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_internal_http_tls_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 13);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseInternalHttpTls = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseInternalHttpTls").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,true,true,true,true,true,true,true,{"minVersion":"TLSv1.2","key":"key","cert":"cert","ca":["ca"]},true,true,false,true,1023,1]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http2_h2c_sessions_exchange_real_frames_over_tcp() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http2_h2c");
    fs::write(
            dir.join("index.js"),
            "var http2 = require('node:http2'); module.exports = function () { return new Promise(function(resolve, reject) { var large = 'x'.repeat(20000), server = http2.createServer(); server.on('error', reject); server.on('stream', function(stream, headers) { stream.respond({ ':status': 200, 'content-type': 'text/plain', 'x-path': headers[':path'], 'x-return': headers['x-large'] }); stream.end('hello ' + headers['x-name']); }); server.listen(0, '127.0.0.1', function() { var address = server.address(), session = http2.connect('http://127.0.0.1:' + address.port); session.on('error', reject); session.on('connect', function() { var ping = Buffer.from('12345678'), request = session.request({ ':path': '/demo', 'x-name': 'thaw', 'x-large': large }), chunks = [], response; request.on('response', function(headers) { response = headers; }); request.on('data', function(chunk) { chunks.push(Buffer.from(chunk)); }); request.on('end', function() { session.ping(ping, function(error, duration, payload) { if (error) return reject(error); var packed = http2.getPackedSettings({ enablePush: false, maxConcurrentStreams: 12 }), unpacked = http2.getUnpackedSettings(packed); session.close(); server.close(function() { resolve([session instanceof http2.ClientHttp2Session, request instanceof http2.ClientHttp2Stream, response[':status'], response['content-type'], response['x-path'], response['x-return'].length, Buffer.concat(chunks).toString(), payload.toString(), duration >= 0, unpacked.enablePush, unpacked.maxConcurrentStreams, http2.constants.NGHTTP2_NO_ERROR, typeof http2.sensitiveHeaders]); }); }); }); request.end(); }); }); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http2_h2c_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttp2 = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttp2").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,"200","text/plain","/demo",20000,"hello thaw","12345678",true,0,12,0,"symbol"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http2_flow_control_resumes_large_bidirectional_bodies() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http2_flow_control");
    fs::write(
            dir.join("index.js"),
            "var http2 = require('node:http2'); module.exports = function () { return new Promise(function(resolve, reject) { var requestBody = 'q'.repeat(100000), responseBody = 'z'.repeat(120000), server = http2.createServer(); server.on('error', reject); server.on('stream', function(stream) { var received = 0; stream.on('data', function(chunk) { received += chunk.length; }); stream.on('end', function() { stream.respond({ ':status': 201, 'x-received': String(received) }); stream.end(responseBody); }); }); server.listen(0, '127.0.0.1', function() { var session = http2.connect('http://127.0.0.1:' + server.address().port); session.on('error', reject); session.on('connect', function() { var request = session.request({ ':method': 'POST', ':path': '/upload' }), chunks = [], response, immediate = request.write(requestBody); request.on('response', function(headers) { response = headers; }); request.on('data', function(chunk) { chunks.push(Buffer.from(chunk)); }); request.on('end', function() { var body = Buffer.concat(chunks); session.close(); server.close(function() { resolve([immediate, request.writableEnded, response[':status'], response['x-received'], body.length, body[0], body[body.length - 1]]); }); }); request.end(); }); }); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http2_flow_control_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttp2Flow = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttp2Flow").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[false,true,"201","100000",120000,122,122]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http2_hpack_reuses_dynamic_entries_across_streams() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http2_hpack_dynamic");
    fs::write(
            dir.join("index.js"),
            "var http2 = require('node:http2'); module.exports = function () { return new Promise(function(resolve, reject) { var serverSession, server = http2.createServer(); server.on('session', function(session) { serverSession = session; }); server.on('stream', function(stream, headers) { stream.respond({ ':status': 200, 'x-repeated-response': 'compress-me-compress-me' }); stream.end(headers['x-repeated-request']); }); server.listen(0, '127.0.0.1', function() { var session = http2.connect('http://127.0.0.1:' + server.address().port); session.on('error', reject); session.on('connect', async function() { function run(path) { return new Promise(function(done) { var request = session.request({ ':path': path, 'x-repeated-request': 'www.example.com' }), chunks = []; request.on('data', function(chunk) { chunks.push(Buffer.from(chunk)); }); request.on('end', function() { done(Buffer.concat(chunks).toString()); }); request.end(); }); } var first = await run('/first'), firstEncoderSize = session._encoderTable.length, second = await run('/second'); var clientEntry = session._encoderTable.some(function(entry) { return entry[0] === 'x-repeated-request' && entry[1] === 'www.example.com'; }), serverEntry = serverSession._decoderTable.some(function(entry) { return entry[0] === 'x-repeated-request' && entry[1] === 'www.example.com'; }), responseEntry = session._decoderTable.some(function(entry) { return entry[0] === 'x-repeated-response'; }); session.close(); server.close(function() { resolve([first, second, firstEncoderSize > 0, clientEntry, serverEntry, responseEntry]); }); }); }); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http2_hpack_dynamic_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttp2Hpack = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttp2Hpack").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["www.example.com","www.example.com",true,true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http2_tls_sessions_negotiate_h2_with_alpn() {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    use rustls::{ServerConfig, ServerConnection, StreamOwned};
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::Command;
    use std::sync::Arc;

    let certificate_dir = temp_registry("builtin_http2_tls_certificate");
    let key_path = certificate_dir.join("key.pem");
    let cert_path = certificate_dir.join("cert.pem");
    let key_der_path = certificate_dir.join("key.der");
    let cert_der_path = certificate_dir.join("cert.der");
    assert!(Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature,keyEncipherment",
            "-addext",
            "extendedKeyUsage=serverAuth",
            "-keyout",
        ])
        .arg(&key_path)
        .arg("-out")
        .arg(&cert_path)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"])
        .arg(&cert_path)
        .args(["-outform", "DER", "-out"])
        .arg(&cert_der_path)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
        .arg(&key_path)
        .args(["-outform", "DER", "-out"])
        .arg(&key_der_path)
        .output()
        .unwrap()
        .status
        .success());
    let certificate = fs::read_to_string(&cert_path).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut tls_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(fs::read(&cert_der_path).unwrap())],
            PrivatePkcs8KeyDer::from(fs::read(&key_der_path).unwrap()).into(),
        )
        .unwrap();
    tls_config.alpn_protocols = vec![b"h2".to_vec()];
    let tls_config = Arc::new(tls_config);
    let peer = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let connection = ServerConnection::new(tls_config).unwrap();
        let mut stream = StreamOwned::new(connection, socket);
        let mut preface = [0u8; 24];
        stream.read_exact(&mut preface).unwrap();
        assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
        loop {
            let mut header = [0u8; 9];
            stream.read_exact(&mut header).unwrap();
            let length =
                ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
            let mut payload = vec![0u8; length];
            stream.read_exact(&mut payload).unwrap();
            if header[3] == 1 {
                let body = b"encrypted";
                let response = [
                    vec![0, 0, 0, 4, 0, 0, 0, 0, 0],
                    vec![0, 0, 1, 1, 4, 0, 0, 0, 1, 0x88],
                    vec![0, 0, body.len() as u8, 0, 1, 0, 0, 0, 1],
                    body.to_vec(),
                ]
                .concat();
                stream.write_all(&response).unwrap();
                stream.flush().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(250));
                break;
            }
        }
    });
    let dir = temp_registry("builtin_http2_tls");
    fs::write(
            dir.join("index.js"),
            format!(
                "var http2 = require('node:http2'), certificate = {}; module.exports = function () {{ return new Promise(function(resolve, reject) {{ var secureServer = http2.createSecureServer({{}}), session = http2.connect('https://127.0.0.1:{}', {{ ca: certificate }}); session.on('error', reject); session.on('connect', function() {{ var request = session.request({{ ':path': '/secure' }}), chunks = [], response; request.on('response', function(headers) {{ response = headers; }}); request.on('data', function(chunk) {{ chunks.push(Buffer.from(chunk)); }}); request.on('end', function() {{ session.close(); resolve([secureServer instanceof http2.Http2SecureServer, session.encrypted, session.alpnProtocol, session.socket.alpnProtocol, response[':status'], Buffer.concat(chunks).toString()]); }}); request.end(); }}); }}); }};",
                serde_json::to_string(&certificate).unwrap(),
                port
            ),
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http2_tls_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttp2Tls = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttp2Tls").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,true,"h2","h2","200","encrypted"]"#);
    peer.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    let _ = fs::remove_dir_all(&certificate_dir);
}

#[test]
fn http2_secure_server_accepts_an_alpn_h2_peer() {
    use rustls::pki_types::{CertificateDer, ServerName};
    use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    let certificate_dir = temp_registry("builtin_http2_secure_server_certificate");
    let key_path = certificate_dir.join("key.pem");
    let cert_path = certificate_dir.join("cert.pem");
    let cert_der_path = certificate_dir.join("cert.der");
    assert!(Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature,keyEncipherment",
            "-addext",
            "extendedKeyUsage=serverAuth",
            "-keyout",
        ])
        .arg(&key_path)
        .arg("-out")
        .arg(&cert_path)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"])
        .arg(&cert_path)
        .args(["-outform", "DER", "-out"])
        .arg(&cert_der_path)
        .output()
        .unwrap()
        .status
        .success());
    let certificate = fs::read_to_string(&cert_path).unwrap();
    let key = fs::read_to_string(&key_path).unwrap();
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(fs::read(&cert_der_path).unwrap()))
        .unwrap();
    let mut client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_config.alpn_protocols = vec![b"h2".to_vec()];
    let client_config = Arc::new(client_config);
    let peer = std::thread::spawn(move || {
        let socket = (0..100)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(socket) => Some(socket),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                }
            })
            .expect("HTTP/2 secure server did not start");
        let connection = ClientConnection::new(
            client_config,
            ServerName::try_from("127.0.0.1".to_string()).unwrap(),
        )
        .unwrap();
        let mut stream = StreamOwned::new(connection, socket);
        let request = [
            b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec(),
            vec![0, 0, 0, 4, 0, 0, 0, 0, 0],
            vec![0, 0, 3, 1, 5, 0, 0, 0, 1, 0x82, 0x84, 0x87],
        ]
        .concat();
        stream.write_all(&request).unwrap();
        stream.flush().unwrap();
        let mut body = Vec::new();
        loop {
            let mut header = [0u8; 9];
            stream.read_exact(&mut header).unwrap();
            let length =
                ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
            let mut payload = vec![0u8; length];
            stream.read_exact(&mut payload).unwrap();
            if header[3] == 0 {
                body.extend(payload);
                if header[4] & 1 != 0 {
                    break;
                }
            }
        }
        (
            stream
                .conn
                .alpn_protocol()
                .map(|value| value.to_vec())
                .unwrap_or_default(),
            body,
        )
    });
    let dir = temp_registry("builtin_http2_secure_server");
    fs::write(
            dir.join("index.js"),
            format!(
                "var http2 = require('node:http2'), certificate = {}, key = {}; module.exports = function () {{ return new Promise(function(resolve, reject) {{ var sessionState, server = http2.createSecureServer({{ cert: certificate, key: key }}); server.on('error', reject); server.on('sessionError', reject); server.on('session', function(session) {{ sessionState = [session.encrypted, session.alpnProtocol, session.socket.alpnProtocol]; }}); server.on('stream', function(stream, headers) {{ stream.on('close', function() {{ server.close(function() {{ resolve([server instanceof http2.Http2SecureServer, sessionState, headers[':scheme']]); }}); }}); stream.respond({{ ':status': 202 }}); stream.end('served'); }}); server.listen({}, '127.0.0.1'); }}); }};",
                serde_json::to_string(&certificate).unwrap(),
                serde_json::to_string(&key).unwrap(),
                port
            ),
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http2_secure_server_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttp2SecureServer = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttp2SecureServer").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,[true,"h2","h2"],"https"]"#);
    let (alpn, body) = peer.join().unwrap();
    assert_eq!(alpn, b"h2");
    assert_eq!(body, b"served");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    let _ = fs::remove_dir_all(&certificate_dir);
}

#[test]
fn node_test_runs_suites_hooks_mocks_and_reporters() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_node_test");
    fs::write(
            dir.join("index.js"),
            "var test = require('node:test'), reporters = require('node:test/reporters'), assert = require('node:assert'); module.exports = async function () { var events = [], object = { add: function(value) { return value + 1; } }; var suite = await test.describe('math', function() { test.before(function() { events.push('before'); }); test.after(function() { events.push('after'); }); test.beforeEach(function() { events.push('beforeEach'); }); test.afterEach(function() { events.push('afterEach'); }); test.it('adds', async function(t) { events.push('test'); var mocked = t.mock.method(object, 'add', function(value) { return value * 2; }); assert.strictEqual(object.add(4), 8); assert.strictEqual(mocked.mock.callCount(), 1); await Promise.resolve(); }); test.skip('skipped', function() { throw new Error('must not run'); }); test.todo('later', function() { throw new Error('must not run'); }); }); var restored = object.add(4), raw = [], stream = test.run(); for await (var event of stream) raw.push([event.type, event.name, event.status]); var text = '', formatted = test.run(); for await (var line of reporters.spec(formatted)) text += line; return [suite.status, events, restored, raw, text.includes('✔ math > adds'), text.includes('- math > skipped'), test === test.test, test.it === test, typeof test.mock.fn(function() {})]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_node_test_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 5);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNodeTest = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNodeTest").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["passed",["before","beforeEach","test","afterEach","after"],5,[["test","adds","passed"],["test","skipped","skipped"],["test","later","todo"],["suite","math","passed"]],true,true,true,true,"function"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn node_wasi_runs_preview1_commands_and_reactors() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_node_wasi");
    let wasi_source = r#"var WASI = require('node:wasi').WASI;
               module.exports = function () {
                 var commandSource = new TextEncoder().encode(`(module
                   (import "wasi_snapshot_preview1" "args_sizes_get" (func $args_sizes_get (param i32 i32) (result i32)))
                   (import "wasi_snapshot_preview1" "environ_sizes_get" (func $environ_sizes_get (param i32 i32) (result i32)))
                   (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
                   (memory (export "memory") 1)
                   (func (export "_start")
                     i32.const 0 i32.const 4 call $args_sizes_get drop
                     i32.const 8 i32.const 12 call $environ_sizes_get drop
                     i32.const 7 call $proc_exit))`);
                 var wasi = new WASI({ version: 'preview1', args: ['alpha', 'beta'], env: { A: '1', B: 'two' }, returnOnExit: true });
                 var command = new WebAssembly.Instance(new WebAssembly.Module(commandSource), wasi.getImportObject());
                 var exit = wasi.start(command), words = new Uint32Array(command.exports.memory.buffer, 0, 4), repeated;
                 try { wasi.start(command); } catch (error) { repeated = error.code; }
                 var reactorSource = new TextEncoder().encode(`(module (memory (export "memory") 1) (func (export "_initialize")))`);
                 var reactorWasi = new WASI({ version: 'preview1' });
                 var reactor = new WebAssembly.Instance(new WebAssembly.Module(reactorSource), reactorWasi.getImportObject());
                 var initialized = reactorWasi.initialize(reactor), invalid = false;
                 try { new WASI({ version: 'preview2' }); } catch (error) { invalid = error.code === 'ERR_INVALID_ARG_VALUE'; }
                 var preopenSource = new TextEncoder().encode(`(module
                   (import "wasi_snapshot_preview1" "fd_prestat_get" (func $get (param i32 i32) (result i32)))
                   (import "wasi_snapshot_preview1" "fd_prestat_dir_name" (func $name (param i32 i32 i32) (result i32)))
                   (memory (export "memory") 1)
                   (func (export "_start") i32.const 3 i32.const 0 call $get drop i32.const 3 i32.const 16 i32.const 8 call $name drop))`);
                 var preopenWasi = new WASI({ version: 'preview1', preopens: { '/sandbox': __HOST_PATH__ }, returnOnExit: true });
                 var preopen = new WebAssembly.Instance(new WebAssembly.Module(preopenSource), preopenWasi.getImportObject()); preopenWasi.start(preopen);
                 var preopenLength = new Uint32Array(preopen.exports.memory.buffer, 4, 1)[0], preopenName = new TextDecoder().decode(new Uint8Array(preopen.exports.memory.buffer, 16, preopenLength));
                 return [exit, Array.from(words), repeated, initialized, invalid, wasi.wasiImport === wasi.getImportObject().wasi_snapshot_preview1, preopenLength, preopenName];
               };"#
        .replace(
            "__HOST_PATH__",
            &serde_json::to_string(dir.to_str().unwrap()).unwrap(),
        );
    fs::write(dir.join("index.js"), wasi_source).unwrap();
    let empty_node_modules = temp_registry("builtin_node_wasi_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNodeWasi = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNodeWasi").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[7,[2,11,2,10],"ERR_WASI_ALREADY_STARTED",null,true,true,8,"/sandbox"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn string_decoder_preserves_multibyte_boundaries() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_string_decoder");
    fs::write(
            dir.join("index.js"),
            "var StringDecoder = require('node:string_decoder').StringDecoder;\n\
             module.exports = function () {\n\
             \x20 var utf8 = new StringDecoder('utf8'); var snow = Buffer.from('雪'); var utf8Parts = [utf8.write(snow.subarray(0, 1)), utf8.write(snow.subarray(1, 2)), utf8.write(snow.subarray(2)), utf8.end()];\n\
             \x20 var utf16 = new StringDecoder('utf16le'); var wide = Buffer.from('A雪', 'utf16le'); var utf16Parts = [utf16.write(wide.subarray(0, 3)), utf16.end(wide.subarray(3))];\n\
             \x20 var base64 = new StringDecoder('base64'); var hello = Buffer.from('hello'); var encoded = base64.write(hello.subarray(0, 2)) + base64.write(hello.subarray(2)) + base64.end();\n\
             \x20 var incomplete = new StringDecoder(); var replacement = incomplete.end(Buffer.from([0xe9]));\n\
             \x20 return [utf8Parts, utf16Parts, encoded, replacement, utf8.lastNeed, utf8.encoding];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_string_decoder_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseStringDecoder = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseStringDecoder").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["","","雪",""],["A","雪"],"aGVsbG8=","�",0,"utf8"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn timer_modules_share_the_runtime_queue_and_support_abort() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_timers");
    fs::write(
            dir.join("index.js"),
            "var timers = require('node:timers'); var promises = require('node:timers/promises');\n\
             module.exports = async function () {\n\
             \x20 var immediate = await new Promise(function(resolve) { timers.setImmediate(resolve, 'immediate'); });\n\
             \x20 var delayed = await promises.setTimeout(0, 'delayed'); await promises.scheduler.yield();\n\
             \x20 var controller = new AbortController(); controller.abort('cancelled'); var reason; try { await promises.setTimeout(1, 'wrong', { signal: controller.signal }); } catch (error) { reason = error; }\n\
             \x20 var interval = promises.setInterval(0, 'tick'); var first = await interval.next(); var second = await interval.next(); var ended = await interval.return();\n\
             \x20 return [immediate, delayed, reason, first.value, first.done, second.value, ended.done];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_timers_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseTimers = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseTimers").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["immediate","delayed","cancelled","tick",false,"tick",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_read_and_write_streams_integrate_with_node_streams() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_streams");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), stream = require('node:stream'); module.exports = async function (path) { var writer = fs.createWriteStream(path, { highWaterMark: 2 }), writerEvents = []; writer.on('open', function() { writerEvents.push('open'); }); writer.on('ready', function() { writerEvents.push('ready'); }); writer.on('close', function() { writerEvents.push('close'); }); var written = new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); }); writer.write('abc'); writer.end('def'); await written; await new Promise(function(resolve) { queueMicrotask(resolve); }); var appender = fs.createWriteStream(path, { flags: 'a' }), appended = new Promise(function(resolve, reject) { appender.on('error', reject); appender.on('finish', resolve); }); appender.end('!'); await appended; var reader = fs.createReadStream(path, { start: 1, end: 4, highWaterMark: 2 }), chunks = [], readerEvents = []; reader.on('open', function() { readerEvents.push('open'); }); reader.on('ready', function() { readerEvents.push('ready'); }); reader.on('data', function(value) { chunks.push(value.toString()); }); reader.on('close', function() { readerEvents.push('close'); }); var read = new Promise(function(resolve, reject) { reader.on('error', reject); reader.on('end', resolve); }); await read; await new Promise(function(resolve) { queueMicrotask(resolve); }); return [writer instanceof fs.WriteStream, writer instanceof stream.Writable, writer.bytesWritten, writerEvents, reader instanceof fs.ReadStream, reader instanceof stream.Readable, reader.bytesRead, chunks, readerEvents, fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_streams_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsStreams = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("stream.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsStreams").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,6,["open","ready","close"],true,true,4,["bc","de"],["open","ready","close"],"abcdef!"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_streams_use_incremental_positioned_host_io() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_incremental_streams");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (path) { fs.writeFileSync(path, 'abcdef'); var writer = fs.createWriteStream(path, { flags: 'r+', start: 2 }), observed; await new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); writer.write('XY', function(error) { if (error) return reject(error); observed = fs.readFileSync(path, 'utf8'); writer.end('Z'); }); }); var beforeRead = fs.readFileSync(path, 'utf8'), chunks = [], reader = fs.createReadStream(path, { highWaterMark: 2 }); await new Promise(function(resolve, reject) { reader.on('error', reject); reader.on('data', function(chunk) { chunks.push(chunk.toString()); if (chunks.length === 1) fs.writeFileSync(path, 'abWXYZ'); }); reader.on('end', resolve); }); return [observed, beforeRead, writer.bytesWritten, chunks, reader.bytesRead, fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_incremental_streams_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseIncrementalFsStreams = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("stream.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseIncrementalFsStreams")
            .unwrap()
            .as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["abXYef","abXYZf",3,["ab","WX","YZ"],6,"abWXYZ"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_write_streams_apply_backpressure_and_serialize_callbacks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_stream_backpressure");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (root) { var path = root + '/queued.txt', directPath = root + '/direct.txt', events = [], writer = fs.createWriteStream(path, { highWaterMark: 3 }); writer.on('drain', function() { events.push('drain'); }); writer.on('finish', function() { events.push('finish'); }); var first = writer.write('ab', function() { events.push('first'); }), second = writer.write('cd', function() { events.push('second'); }), beforeDrain = fs.readFileSync(path, 'utf8'); var finished = new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); }); writer.end('ef', function() { events.push('end'); }); await finished; await new Promise(function(resolve) { queueMicrotask(resolve); }); var direct = new fs.WriteStream(directPath, { highWaterMark: 1 }), directResult = direct.write('x'); var directFinished = new Promise(function(resolve, reject) { direct.on('error', reject); direct.on('finish', resolve); }); direct.end(); await directFinished; return [first, second, beforeDrain, writer.writableLength, writer.writableHighWaterMark, events, fs.readFileSync(path, 'utf8'), directResult, direct.writableHighWaterMark, fs.readFileSync(directPath, 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_stream_backpressure_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsStreamBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsStreamBackpressure")
            .unwrap()
            .as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,"ab",0,3,["first","drain","second","end","finish"],"abcdef",false,1,"x"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_builtin_pipes_transforms_and_finishes() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_stream");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); var streamPromises = require('node:stream/promises');\n\
             module.exports = async function () {\n\
             \x20 var output = []; var source = stream.Readable.from(['a', Buffer.from('b')]);\n\
             \x20 var upper = new stream.Transform({ transform: function(chunk, encoding, callback) { callback(null, chunk.toString().toUpperCase()); } });\n\
             \x20 var sink = new stream.Writable({ write: function(chunk, encoding, callback) { output.push(chunk.toString()); callback(); } });\n\
             \x20 await new Promise(function(resolve, reject) { stream.pipeline(source, upper, sink, function(error) { if (error) reject(error); else resolve(); }); });\n\
             \x20 var queued = new stream.Readable(); queued.push('left'); queued.push('right'); queued.push(null); var combined = queued.read().toString();\n\
             \x20 var pass = new stream.PassThrough(); var passed = []; pass.on('data', function(chunk) { passed.push(chunk.toString()); }); var completion = streamPromises.finished(pass); pass.end('pass'); await completion;\n\
             \x20 var promiseOutput = []; await streamPromises.pipeline(stream.Readable.from(['promise']), new stream.Writable({ write: function(chunk, encoding, callback) { promiseOutput.push(chunk.toString()); callback(); } }));\n\
             \x20 var controller = new AbortController(); var aborted = new stream.Readable(); var reason; aborted.on('error', function(error) { reason = error; }); stream.addAbortSignal(controller.signal, aborted); controller.abort('stop'); await Promise.resolve();\n\
             \x20 return [output.join(''), sink.writableFinished, source.readableEnded, combined, passed.join(''), pass.readableEnded, promiseOutput.join(''), aborted.destroyed, reason.name, reason.code, reason.message, reason.cause];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseStream = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseStream").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["AB",true,true,"leftright","pass",true,"promise",true,"AbortError","ABORT_ERR","The operation was aborted","stop"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_compose_and_duplex_from_bridge_supported_sources() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compose");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); function collect(value) { var output = []; value.on('data', function(chunk) { output.push(chunk.toString()); }); return new Promise(function(resolve, reject) { value.on('error', reject); value.on('end', function() { resolve(output.join('')); }); }); } module.exports = async function () { var upper = new stream.Transform({ transform: function(chunk, encoding, callback) { callback(null, chunk.toString().toUpperCase()); } }), suffix = new stream.Transform({ transform: function(chunk, encoding, callback) { callback(null, chunk.toString() + '!'); } }), composed = stream.compose(upper, suffix), composedResult = collect(composed); composed.write('a'); composed.end('b'); var iterable = stream.Duplex.from((async function*() { yield 'x'; await Promise.resolve(); yield 'y'; })()), iterableValues = []; for await (var value of iterable) iterableValues.push(value.toString()); var promised = stream.Duplex.from(Promise.resolve('z')), promisedValues = []; for await (var value of promised) promisedValues.push(value.toString()); var written = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { written.push(chunk.toString()); callback(); } }), readable = new stream.Readable(), pair = stream.Duplex.from({ writable: writable, readable: readable }), pairResult = collect(pair); pair.write('input'); pair.end(); readable.push('output'); readable.push(null); var functional = stream.Duplex.from(async function*(source) { for await (var chunk of source) yield chunk.toString().toUpperCase(); }), functionalResult = collect(functional); functional.write('function'); functional.end(); return [composed instanceof stream.Duplex, await composedResult, iterable instanceof stream.Duplex, iterableValues, promisedValues, await pairResult, written, await functionalResult]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compose_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamCompose = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamCompose").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"A!B!",true,["x","y"],["z"],"output",["input"],"FUNCTION"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_compose_bridges_function_stages_and_abort_signals() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compose_functions");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); function collect(value) { var chunks = []; value.on('data', function(chunk) { chunks.push(String(chunk)); }); return new Promise(function(resolve, reject) { value.on('error', reject); value.on('end', function() { resolve(chunks.join('')); }); }); } module.exports = async function () { var signals = []; async function* upper(source, options) { signals.push(options.signal instanceof AbortSignal); for await (var chunk of source) yield String(chunk).toUpperCase(); } async function* suffix(source, options) { signals.push(options.signal instanceof AbortSignal); for await (var chunk of source) yield String(chunk) + '!'; } var transformed = stream.compose(upper, suffix), result = collect(transformed); transformed.write('a'); transformed.end('b'); var output = await result, aborted = false; async function* waiting(source, options) { options.signal.addEventListener('abort', function() { aborted = true; }); for await (var chunk of source) yield chunk; } var cancellable = stream.compose(waiting); cancellable.on('error', function() {}); var closed = new Promise(function(resolve) { cancellable.once('close', resolve); }); cancellable.resume(); await Promise.resolve(); var reason = new Error('stop'); cancellable.destroy(reason); await closed; return [output, signals, aborted, cancellable.errored === reason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compose_functions_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamComposeFunctions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamComposeFunctions")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["A!B!",[true,true],true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_abort_signals_use_node_abort_errors() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_abort_errors");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); function code(action) { try { action(); } catch (error) { return error.code; } } module.exports = async function () { var reason = new Error('stop'), pre = new AbortController(); pre.abort(reason); var direct = new stream.PassThrough(), directError = new Promise(function(resolve) { direct.once('error', resolve); }); stream.addAbortSignal(pre.signal, direct); var directFailure = await directError, source = new stream.Readable({ read: function() {} }), controller = new AbortController(), stageSignal, finalized = false; async function* stage(input, options) { stageSignal = options.signal; try { for await (var value of input) yield value; } finally { finalized = true; } } var composed = source.compose(stage, { signal: controller.signal }), composedError = new Promise(function(resolve) { composed.once('error', resolve); }); composed.resume(); await Promise.resolve(); controller.abort(reason); var composedFailure = await composedError; await Promise.resolve(); return [directFailure.name, directFailure.code, directFailure.cause === reason, composedFailure.name, composedFailure.code, composedFailure.cause === reason, stageSignal.aborted, finalized, source.destroyed, composed.destroyed, code(function() { stream.addAbortSignal({}, direct); }), code(function() { stream.addAbortSignal(pre.signal, {}); })]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_abort_errors_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamAbortErrors = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamAbortErrors").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["AbortError","ABORT_ERR",true,"AbortError","ABORT_ERR",true,true,true,true,true,"ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_TYPE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_abort_signals_error_locked_web_streams() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_web_abort");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var reason = new Error('stop'), readableCancelled = false, readable = new ReadableStream({ cancel: function() { readableCancelled = true; } }), reader = readable.getReader(), pendingRead = reader.read(), readableController = new AbortController(); stream.addAbortSignal(readableController.signal, readable); readableController.abort(reason); var readError; try { await pendingRead; } catch (error) { readError = error; } var writableAborted = false, writable = new WritableStream({ abort: function() { writableAborted = true; } }), writer = writable.getWriter(), writerClosed = writer.closed.catch(function(error) { return error; }), writableController = new AbortController(); stream.addAbortSignal(writableController.signal, writable); writableController.abort(reason); var writeError; try { await writer.write('value'); } catch (error) { writeError = error; } var closedError = await writerClosed, pre = new AbortController(); pre.abort(reason); var preReadable = new ReadableStream(); stream.addAbortSignal(pre.signal, preReadable); var preError; try { await preReadable.getReader().read(); } catch (error) { preError = error; } return [readError.name, readError.code, readError.cause === reason, readableCancelled, readable.locked, writeError === closedError, writeError.name, writeError.code, writeError.cause === reason, writableAborted, writable.locked, preError.name, preError.code, preError.cause === reason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_web_abort_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamWebAbort = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamWebAbort").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["AbortError","ABORT_ERR",true,false,true,true,"AbortError","ABORT_ERR",true,false,true,"AbortError","ABORT_ERR",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_writable_serializes_abort_behind_pending_writes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_writable_abort_order");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var release, gate = new Promise(function(resolve) { release = resolve; }), events = [], reason = new Error('stop'), writable = new WritableStream({ write: function() { events.push('write-start'); return gate.then(function() { events.push('write-end'); }); }, abort: function(error) { events.push(['abort', error === reason]); } }), writer = writable.getWriter(), directAbort, directClose; try { await writable.abort(reason); } catch (error) { directAbort = error.code; } try { await writable.close(); } catch (error) { directClose = error.code; } var write = writer.write('x').then(function() { events.push('write-resolve'); }), abort = writer.abort(reason).then(function() { events.push('abort-resolve'); }), pending = events.slice(); release(); await Promise.all([write, abort]); var closedError; try { await writer.closed; } catch (error) { closedError = error; } var laterError; try { await writer.write('y'); } catch (error) { laterError = error; } var repeated = await writer.abort(new Error('ignored')); return [directAbort, directClose, pending, events, closedError === reason, laterError === reason, repeated]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_writable_abort_order_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebWritableAbort = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebWritableAbort").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ERR_INVALID_STATE","ERR_INVALID_STATE",[],["write-start","write-end",["abort",true],"write-resolve","abort-resolve"],true,true,null]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_writable_tracks_strategy_backpressure_and_ready() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_writable_backpressure");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var releases = [], sized = [], stream = new WritableStream({ write: function() { return new Promise(function(resolve) { releases.push(resolve); }); } }, { highWaterMark: 2, size: function(value) { sized.push(value); return value; } }), writer = stream.getWriter(), initial = writer.desiredSize, first = writer.write(1), afterFirst = writer.desiredSize, firstReady = false; writer.ready.then(function() { firstReady = true; }); await Promise.resolve(); var firstHadCapacity = firstReady, second = writer.write(2), afterSecond = writer.desiredSize, secondReady = false, ready = writer.ready.then(function() { secondReady = true; }); await Promise.resolve(); await Promise.resolve(); var secondBackpressured = !secondReady; releases.shift()(); await first; await Promise.resolve(); var afterFirstCompletion = [writer.desiredSize, secondReady]; await Promise.resolve(); releases.shift()(); await second; await ready; var final = [writer.desiredSize, secondReady], zero = new WritableStream({}, { highWaterMark: 0 }), zeroWriter = zero.getWriter(), zeroReady = false; zeroWriter.ready.then(function() { zeroReady = true; }); await Promise.resolve(); var zeroInitiallyPending = !zeroReady; await zeroWriter.close(); await zeroWriter.ready; var invalidHighWaterMark, invalidSizeStream = new WritableStream({}, { size: function() { return -1; } }), invalidSizeWriter = invalidSizeStream.getWriter(), invalidSize = await invalidSizeWriter.write('x').catch(function(error) { return error; }), invalidClosed = await invalidSizeWriter.closed.catch(function(error) { return error; }), invalidReady = await invalidSizeWriter.ready.catch(function(error) { return error; }); try { new WritableStream({}, { highWaterMark: -1 }); } catch (error) { invalidHighWaterMark = error.code; } return [initial, afterFirst, firstHadCapacity, afterSecond, secondBackpressured, afterFirstCompletion, final, sized, zeroInitiallyPending, zeroWriter.desiredSize, invalidHighWaterMark, invalidSize.name, invalidSize.code, invalidClosed === invalidSize, invalidReady === invalidSize]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_writable_backpressure_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebWritableBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebWritableBackpressure")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[2,1,true,-1,true,[0,false],[2,true],[1,2],true,0,"ERR_INVALID_ARG_VALUE","RangeError","ERR_INVALID_ARG_VALUE",true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_stream_locks_can_be_released_and_reacquired() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_stream_release_locks");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var readableController, readable = new ReadableStream({ start: function(controller) { readableController = controller; } }), reader = readable.getReader(), pendingRead = reader.read().catch(function(error) { return error; }); reader.releaseLock(); var pendingReadError = await pendingRead, oldReadError, oldReaderClosed; try { await reader.read(); } catch (error) { oldReadError = error; } try { await reader.closed; } catch (error) { oldReaderClosed = error; } var readerUnlocked = !readable.locked, replacementReader = readable.getReader(); readableController.enqueue('value'); var replacementRead = await replacementReader.read(), releaseWrite, writeGate = new Promise(function(resolve) { releaseWrite = resolve; }), writable = new WritableStream({ write: function() { return writeGate; } }), writer = writable.getWriter(), pendingWrite = writer.write('x'); writer.releaseLock(); var oldWriteError, oldWriterReady, oldWriterClosed; try { await writer.write('y'); } catch (error) { oldWriteError = error; } try { await writer.ready; } catch (error) { oldWriterReady = error; } try { await writer.closed; } catch (error) { oldWriterClosed = error; } var writerUnlocked = !writable.locked, replacementWriter = writable.getWriter(); releaseWrite(); await pendingWrite; await replacementWriter.close(); return [pendingReadError.code, oldReadError.code, oldReaderClosed.code, readerUnlocked, replacementRead, oldWriteError.code, oldWriterReady.code, oldWriterClosed.code, writerUnlocked, writable.locked]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_stream_release_locks_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebStreamLocks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebStreamLocks").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ERR_INVALID_STATE","ERR_INVALID_STATE","ERR_INVALID_STATE",true,{"value":"value","done":false},"ERR_INVALID_STATE","ERR_INVALID_STATE","ERR_INVALID_STATE",true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_stream_tee_branches_and_aggregates_cancellation() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_stream_tee");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { function code(action) { try { action(); } catch (error) { return error.code; } } var pulls = 0, chunks = [{ n: 1 }, { n: 2 }], source = new ReadableStream({ pull: function(controller) { controller.enqueue(chunks[pulls++]); if (pulls === 2) controller.close(); } }), branches = source.tee(), leftReader = branches[0].getReader(), rightReader = branches[1].getReader(), leftFirst = await leftReader.read(), leftSecond = await leftReader.read(), rightFirst = await rightReader.read(), rightSecond = await rightReader.read(), leftDone = await leftReader.read(), rightDone = await rightReader.read(), cancelReasons, cancelSource = new ReadableStream({ cancel: function(reasons) { cancelReasons = reasons; } }), cancelBranches = cancelSource.tee(), firstSettled = false, firstCancel = cancelBranches[0].cancel('left').then(function() { firstSettled = true; }); await Promise.resolve(); await Promise.resolve(); var firstPending = !firstSettled, secondCancel = cancelBranches[1].cancel('right'); await Promise.all([firstCancel, secondCancel]); var errorController, failed = new ReadableStream({ start: function(controller) { errorController = controller; } }), failedBranches = failed.tee(), failedLeftReader = failedBranches[0].getReader(), failedRightReader = failedBranches[1].getReader(), failedLeft = failedLeftReader.read().catch(function(error) { return error; }), failedRight = failedRightReader.read().catch(function(error) { return error; }), failure = new Error('boom'); errorController.error(failure); var failures = await Promise.all([failedLeft, failedRight]), locked = new ReadableStream(), lockedReader = locked.getReader(), lockedCode = code(function() { locked.tee(); }); lockedReader.releaseLock(); return [pulls, leftFirst.value === chunks[0], leftSecond.value === chunks[1], rightFirst.value === chunks[0], rightSecond.value === chunks[1], leftDone.done, rightDone.done, source.locked, firstPending, cancelReasons, cancelSource.locked, failures[0] === failure, failures[1] === failure, failed.locked, lockedCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_stream_tee_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableStreamTee = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableStreamTee").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[2,true,true,true,true,true,true,true,true,["left","right"],true,true,true,true,"ERR_INVALID_STATE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_byte_streams_support_byob_readers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_stream_byob");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); module.exports = async function () { var requestClass, viewClass, desired, source = new ReadableStream({ type: 'bytes', pull: function(controller) { var request = controller.byobRequest; requestClass = request.constructor === web.ReadableStreamBYOBRequest; viewClass = request.view instanceof Uint8Array; desired = controller.desiredSize; request.view[0] = 65; request.view[1] = 66; request.respond(2); controller.close(); } }), reader = source.getReader({ mode: 'byob' }), first = await reader.read(new Uint8Array(8)), done = await reader.read(new Uint8Array(4)), queued = new ReadableStream({ type: 'bytes', start: function(controller) { controller.enqueue(new Uint8Array([1, 2, 3])); controller.close(); } }), queuedReader = queued.getReader({ mode: 'byob' }), queuedFirst = await queuedReader.read(new Uint8Array(2)), queuedSecond = await queuedReader.read(new Uint8Array(4)), queuedDone = await queuedReader.read(new Uint8Array(1)), defaultReader = new ReadableStream({ type: 'bytes', start: function(controller) { controller.enqueue(new Uint8Array([9])); controller.close(); } }).getReader(), defaultValue = await defaultReader.read(), minController, minStream = new ReadableStream({ type: 'bytes', start: function(controller) { minController = controller; } }), minReader = minStream.getReader({ mode: 'byob' }), minSettled = false, minRead = minReader.read(new Uint8Array(5), { min: 3 }).then(function(result) { minSettled = true; return result; }); minController.enqueue(new Uint8Array([1, 2])); await Promise.resolve(); await Promise.resolve(); var waitedForMin = !minSettled; minController.enqueue(new Uint8Array([3, 4])); var minResult = await minRead, allocatedInfo, allocated = new ReadableStream({ type: 'bytes', autoAllocateChunkSize: 4, pull: function(controller) { allocatedInfo = controller.byobRequest.view.byteLength; controller.byobRequest.view[0] = 7; controller.byobRequest.respond(1); controller.close(); } }), allocatedReader = allocated.getReader(), allocatedValue = await allocatedReader.read(), teeSource = new ReadableStream({ type: 'bytes', start: function(controller) { controller.enqueue(new Uint8Array([5, 6])); controller.close(); } }), teeBranches = teeSource.tee(), teeLeft = await teeBranches[0].getReader({ mode: 'byob' }).read(new Uint8Array(2)), teeRight = await teeBranches[1].getReader({ mode: 'byob' }).read(new Uint8Array(2)), wrongStream, wrongMode; try { new ReadableStream().getReader({ mode: 'byob' }); } catch (error) { wrongStream = error.code; } try { new ReadableStream().getReader({ mode: 'invalid' }); } catch (error) { wrongMode = error.code; } return [requestClass, viewClass, desired, Array.from(first.value), first.done, done.value.byteLength, done.done, Array.from(queuedFirst.value), Array.from(queuedSecond.value), queuedDone.value.byteLength, queuedDone.done, Array.from(defaultValue.value), waitedForMin, Array.from(minResult.value), allocatedInfo, Array.from(allocatedValue.value), Array.from(teeLeft.value), Array.from(teeRight.value), teeLeft.value !== teeRight.value, wrongStream, wrongMode, web.ReadableByteStreamController === globalThis.ReadableByteStreamController, web.ReadableStreamBYOBReader === globalThis.ReadableStreamBYOBReader]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_stream_byob_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableStreamByob = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableStreamByob").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,0,[65,66],false,0,true,[1,2],[3],0,true,[9],true,[1,2,3,4],4,[7],[5,6],[5,6],true,"ERR_INVALID_ARG_VALUE","ERR_INVALID_ARG_VALUE",true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn byob_supports_dataview_alignment_and_request_lifecycle() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_stream_byob_views");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var dataInfo, dataStream = new ReadableStream({ type: 'bytes', pull: function(controller) { var request = controller.byobRequest; dataInfo = [request.view.constructor.name, request.view.byteLength]; request.view[0] = 3; request.respond(1); controller.close(); } }), dataResult = await dataStream.getReader({ mode: 'byob' }).read(new DataView(new ArrayBuffer(4))), calls = 0, closeCode, viewCode, staleRequest, typed = new ReadableStream({ type: 'bytes', pull: function(controller) { calls++; var request = controller.byobRequest; staleRequest = request; if (calls === 1) { try { request.respondWithNewView(new Uint8Array(request.view.buffer, request.view.byteOffset + 1, 1)); } catch (error) { viewCode = error.code; } request.view[0] = 1; request.respond(1); try { controller.close(); } catch (error) { closeCode = error.code; } } else { request.view[0] = 2; request.respond(1); controller.close(); } } }), typedResult = await typed.getReader({ mode: 'byob' }).read(new Uint16Array(2)), staleCode; try { staleRequest.respond(0); } catch (error) { staleCode = error.code; } return [dataInfo, dataResult.value.constructor.name, dataResult.value.byteLength, dataResult.value.getUint8(0), calls, typedResult.value.constructor.name, typedResult.value.length, Array.from(new Uint8Array(typedResult.value.buffer, typedResult.value.byteOffset, typedResult.value.byteLength)), viewCode, closeCode, staleCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_stream_byob_views_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableStreamByobViews = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableStreamByobViews")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["Uint8Array",4],"DataView",1,3,2,"Uint16Array",1,[1,2],"ERR_INVALID_ARG_VALUE","ERR_INVALID_STATE","ERR_INVALID_STATE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_stream_default_controllers_manage_errors_and_termination() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_stream_controllers");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); module.exports = async function () { var writableController, writableReason = new Error('writable'), writable = new WritableStream({ start: function(controller) { writableController = controller; } }), writableWriter = writable.getWriter(); writableController.error(writableReason); var writableClosed = await writableWriter.closed.catch(function(error) { return error; }), writableWrite = await writableWriter.write('x').catch(function(error) { return error; }), abortController, abortReason = new Error('abort'), abortSeen, abortable = new WritableStream({ start: function(controller) { abortController = controller; }, abort: function(reason) { abortSeen = reason; } }), abortWriter = abortable.getWriter(); await abortWriter.abort(abortReason); var transformController, started = false, terminated = new TransformStream({ start: function(controller) { transformController = controller; started = true; }, transform: function(value, controller) { controller.enqueue(value + 1); controller.terminate(); } }), terminatedWriter = terminated.writable.getWriter(), terminatedReader = terminated.readable.getReader(), terminatedWrite = terminatedWriter.write(1), transformed = await terminatedReader.read(), transformedDone = await terminatedReader.read(); await terminatedWrite; var laterWrite = await terminatedWriter.write(2).catch(function(error) { return error; }), transformReason = new Error('transform'), failed = new TransformStream({ transform: function(value, controller) { controller.error(transformReason); } }), failedWriter = failed.writable.getWriter(), failedReader = failed.readable.getReader(), failedRead = failedReader.read().catch(function(error) { return error; }); await failedWriter.write(1); var failedValue = await failedRead, failedClosed = await failedWriter.closed.catch(function(error) { return error; }), failedLater = await failedWriter.write(2).catch(function(error) { return error; }); return [writableController.constructor === web.WritableStreamDefaultController, writableController.signal.aborted, writableClosed === writableReason, writableWrite === writableReason, abortController.signal.aborted, abortController.signal.reason === abortReason, abortSeen === abortReason, started, transformController.constructor === web.TransformStreamDefaultController, transformController.desiredSize, transformed.value, transformedDone.done, laterWrite.name, laterWrite.code, failedValue === transformReason, failedClosed === transformReason, failedLater === transformReason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_stream_controllers_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebStreamControllers = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebStreamControllers")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,true,true,true,true,true,true,true,null,2,true,"TypeError","ERR_INVALID_STATE",true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_stream_state_errors_match_node_codes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_stream_state_errors");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { function capture(action) { try { action(); } catch (error) { return [error.name, error.code]; } } async function captureAsync(action) { try { await action(); return ['resolved']; } catch (error) { return [error.name, error.code]; } } var readableController, readable = new ReadableStream({ start: function(controller) { readableController = controller; controller.close(); } }), closeAgain = capture(function() { readableController.close(); }), enqueueClosed = capture(function() { readableController.enqueue(1); }), reader = readable.getReader(), cancelLocked = await captureAsync(function() { return readable.cancel(); }), cancelClosed = await captureAsync(function() { return reader.cancel(); }), writableController, writable = new WritableStream({ start: function(controller) { writableController = controller; } }), writer = writable.getWriter(), firstClose = await captureAsync(function() { return writer.close(); }), secondClose = await captureAsync(function() { return writer.close(); }), writeClosed = await captureAsync(function() { return writer.write(1); }), abortClosed = await captureAsync(function() { return writer.abort('ignored'); }), directLocked = await captureAsync(function() { return writable.close(); }), controllerErrorClosed = capture(function() { writableController.error(new Error('ignored')); }), errorReason = new Error('failure'), erroredController, errored = new WritableStream({ start: function(controller) { erroredController = controller; } }), erroredWriter = errored.getWriter(); erroredController.error(errorReason); var closeError = await erroredWriter.close().catch(function(error) { return error; }), abortErrored = await captureAsync(function() { return erroredWriter.abort(); }); return [closeAgain, enqueueClosed, cancelLocked, cancelClosed, firstClose, secondClose, writeClosed, abortClosed, directLocked, controllerErrorClosed, closeError === errorReason, abortErrored]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_stream_state_errors_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebStreamStateErrors = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebStreamStateErrors")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["TypeError","ERR_INVALID_STATE"],["TypeError","ERR_INVALID_STATE"],["TypeError","ERR_INVALID_STATE"],["resolved"],["resolved"],["TypeError","ERR_INVALID_STATE"],["TypeError","ERR_INVALID_STATE"],["resolved"],["TypeError","ERR_INVALID_STATE"],null,true,["resolved"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn transform_stream_applies_readable_backpressure_and_strategies() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_transform_stream_backpressure");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var events = [], transform = new TransformStream({ transform: function(value, controller) { events.push('transform' + value); controller.enqueue(value); } }, { highWaterMark: 3 }, { highWaterMark: 1 }), writer = transform.writable.getWriter(), reader = transform.readable.getReader(), firstWrite = writer.write(1).then(function() { events.push('write1'); }); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var afterFirst = events.slice(), secondWrite = writer.write(2).then(function() { events.push('write2'); }); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var whileFull = events.slice(), first = await reader.read(); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var afterRead = events.slice(), second = await reader.read(); await Promise.all([firstWrite, secondWrite]); var defaultEvents = [], defaults = new TransformStream({ transform: function(value, controller) { defaultEvents.push('transform'); controller.enqueue(value); } }), defaultWriter = defaults.writable.getWriter(), defaultReader = defaults.readable.getReader(), defaultWrite = defaultWriter.write(3).then(function() { defaultEvents.push('write'); }); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var defaultBlocked = defaultEvents.slice(), defaultRead = defaultReader.read(); await Promise.all([defaultWrite, defaultRead]); var sizes = [], desired = [], strategic = new TransformStream({ transform: function(value, controller) { desired.push(controller.desiredSize); controller.enqueue(value); desired.push(controller.desiredSize); } }, { highWaterMark: 5, size: function(value) { sizes.push(['w', value]); return 2; } }, { highWaterMark: 4, size: function(value) { sizes.push(['r', value]); return 3; } }), strategicWriter = strategic.writable.getWriter(), strategicReader = strategic.readable.getReader(), strategicWrite = strategicWriter.write('x'); await strategicWrite; var strategicValue = await strategicReader.read(), reason = new Error('stop'), cancelled = new TransformStream(), cancelledWriter = cancelled.writable.getWriter(), cancelledReader = cancelled.readable.getReader(), blockedWrite = cancelledWriter.write(1).catch(function(error) { return error; }); await new Promise(function(resolve) { setTimeout(resolve, 0); }); await cancelledReader.cancel(reason); var writeError = await blockedWrite, closedError = await cancelledWriter.closed.catch(function(error) { return error; }); return [afterFirst, whileFull, first.value, afterRead, second.value, writer.desiredSize, defaultBlocked, defaultEvents, sizes, desired, strategicValue.value, strategicWriter.desiredSize, writeError === reason, closedError === reason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_transform_stream_backpressure_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTransformStreamBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseTransformStreamBackpressure")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["transform1","write1"],["transform1","write1"],1,["transform1","write1","transform2","write2"],2,3,[],["transform","write"],[["w","x"],["r","x"]],[4,1],"x",5,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_stream_from_adapts_sync_and_async_iterables() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_stream_from");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); module.exports = async function () { var values = []; for await (var value of ReadableStream.from([Promise.resolve(1), 2])) values.push(value); var asyncCalls = 0, asyncIterable = { next: async function() { asyncCalls++; if (asyncCalls === 1) return { value: 'async', done: false }; return { done: true }; } }; asyncIterable[Symbol.asyncIterator] = function() { return this; }; var asyncValues = []; for await (var asyncValue of web.ReadableStream.from(asyncIterable)) asyncValues.push(asyncValue); var returned = [], endless = { next: function() { return { value: 7, done: false }; }, return: function(reason) { returned.push(reason); return { done: true }; } }; endless[Symbol.iterator] = function() { return this; }; var cancelled = ReadableStream.from(endless), cancelledReader = cancelled.getReader(); await cancelledReader.read(); await cancelledReader.cancel('stop'); var originalController, original = new ReadableStream({ start: function(controller) { originalController = controller; } }), adapted = ReadableStream.from(original), lockedImmediately = original.locked, adaptedReader = adapted.getReader(); originalController.enqueue('source'); originalController.close(); var adaptedValue = await adaptedReader.read(), adaptedDone = await adaptedReader.read(), calls = 0, failure = new Error('boom'), failing = { next: function() { calls++; if (calls === 1) return { value: 'first', done: false }; throw failure; } }; failing[Symbol.iterator] = function() { return this; }; var failingReader = ReadableStream.from(failing).getReader(), first = await failingReader.read(), received = await failingReader.read().catch(function(error) { return error; }), invalid; try { ReadableStream.from(1); } catch (error) { invalid = [error.name, error.code]; } return [values, asyncValues, asyncCalls, returned, lockedImmediately, adaptedValue.value, adaptedDone.done, first.value, received === failure, invalid, web.ReadableStream.from === globalThis.ReadableStream.from]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_stream_from_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableStreamFrom = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableStreamFrom").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[1,2],["async"],2,["stop"],true,"source",true,"first",true,["TypeError","ERR_ARG_NOT_ITERABLE"],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_readable_tracks_strategy_pull_and_delayed_close() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_readable_strategy");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var sizes = [], desiredAtClose, source = new ReadableStream({ start: function(controller) { controller.enqueue('a'); controller.enqueue('bb'); controller.close(); desiredAtClose = controller.desiredSize; } }, { highWaterMark: 4, size: function(value) { sizes.push(value); return value.length; } }), reader = source.getReader(), closed = false; reader.closed.then(function() { closed = true; }); await Promise.resolve(); var closedWithQueue = closed, first = await reader.read(), closedAfterFirst = closed, second = await reader.read(); await reader.closed; var releases = [], pulls = 0, active = 0, maxActive = 0, pulling = new ReadableStream({ pull: function(controller) { pulls++; active++; maxActive = Math.max(maxActive, active); var value = pulls; return new Promise(function(resolve) { releases.push(function() { controller.enqueue(value); active--; resolve(); }); }); } }, { highWaterMark: 2 }); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var initialPulls = pulls; releases.shift()(); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var refilledPulls = pulls; releases.shift()(); await new Promise(function(resolve) { setTimeout(resolve, 0); }); var stoppedAtMark = pulls, pullingReader = pulling.getReader(), pulledFirst = await pullingReader.read(), pulledSecond = await pullingReader.read(), byteController, bytes = new ReadableStream({ type: 'bytes', start: function(controller) { byteController = controller; controller.enqueue(Uint8Array.from([1, 2])); } }, { highWaterMark: 3 }), byteDesiredBefore = byteController.desiredSize, byteReader = bytes.getReader({ mode: 'byob' }), byteValue = await byteReader.read(new Uint8Array(1)), byteDesiredAfter = byteController.desiredSize; byteController.close(); await byteReader.read(new Uint8Array(2)); await byteReader.read(new Uint8Array(1)); var invalidHighWaterMark, invalidByteSize; try { new ReadableStream({}, { highWaterMark: -1 }); } catch (error) { invalidHighWaterMark = error.code; } try { new ReadableStream({ type: 'bytes' }, { size: function() { return 1; } }); } catch (error) { invalidByteSize = error.code; } return [sizes, desiredAtClose, closedWithQueue, first.value, closedAfterFirst, second.value, closed, initialPulls, refilledPulls, stoppedAtMark, maxActive, pulledFirst.value, pulledSecond.value, byteDesiredBefore, Array.from(byteValue.value), byteDesiredAfter, invalidHighWaterMark, invalidByteSize]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_readable_strategy_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebReadableStrategy = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebReadableStrategy")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["a","bb"],1,false,"a",false,"bb",true,1,2,2,1,1,2,1,[1],2,"ERR_INVALID_ARG_VALUE","ERR_INVALID_ARG_VALUE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_queuing_strategies_validate_and_feed_stream_sizes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_queuing_strategies");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); module.exports = function () { function capture(action) { try { action(); } catch (error) { return [error.name, error.code]; } } var byte = new web.ByteLengthQueuingStrategy({ highWaterMark: '4' }), count = new web.CountQueuingStrategy({ highWaterMark: 2 }), byteDesired, byteStream = new ReadableStream({ start: function(controller) { controller.enqueue(new Uint8Array(3)); byteDesired = controller.desiredSize; controller.close(); } }, byte), countDesired, countStream = new ReadableStream({ start: function(controller) { controller.enqueue('a'); controller.enqueue('b'); countDesired = controller.desiredSize; controller.close(); } }, count), negative = new CountQueuingStrategy({ highWaterMark: -1 }); return [byte.highWaterMark, count.highWaterMark, byte.size(new Uint8Array(3)), byte.size({ byteLength: 5 }), count.size(null), Object.keys(byte), Object.keys(ByteLengthQueuingStrategy.prototype), byteDesired, countDesired, byteStream.locked, countStream.locked, capture(function() { new ByteLengthQueuingStrategy(); }), capture(function() { new CountQueuingStrategy({}); }), capture(function() { new ReadableStream({}, negative); }), capture(function() { Object.getOwnPropertyDescriptor(CountQueuingStrategy.prototype, 'highWaterMark').get.call({}); }), web.ByteLengthQueuingStrategy === globalThis.ByteLengthQueuingStrategy]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_queuing_strategies_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebQueuingStrategies = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebQueuingStrategies")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[4,2,3,5,1,[],["highWaterMark","size"],1,0,false,false,["TypeError","ERR_INVALID_ARG_TYPE"],["TypeError","ERR_MISSING_OPTION"],["RangeError","ERR_INVALID_ARG_VALUE"],["TypeError",null],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_pipe_to_propagates_completion_errors_and_abort() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_pipe_to_options");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var writes = [], closed = false, normalSource = new ReadableStream({ start: function(controller) { controller.enqueue('a'); controller.enqueue('b'); controller.close(); } }), normalDestination = new WritableStream({ write: function(value) { writes.push(value); }, close: function() { closed = true; } }); await normalSource.pipeTo(normalDestination, { preventClose: true }); var destinationFailure = new Error('destination'), cancelledWith, failingDestination = new WritableStream({ write: function() { throw destinationFailure; } }), cancellableSource = new ReadableStream({ pull: function(controller) { controller.enqueue('x'); }, cancel: function(error) { cancelledWith = error; } }), destinationResult; try { await cancellableSource.pipeTo(failingDestination); } catch (error) { destinationResult = error; } var sourceFailure = new Error('source'), abortedWith, failedSource = new ReadableStream({ start: function(controller) { controller.error(sourceFailure); } }), abortableDestination = new WritableStream({ abort: function(error) { abortedWith = error; } }), sourceResult; try { await failedSource.pipeTo(abortableDestination); } catch (error) { sourceResult = error; } var controller = new AbortController(), abortReason = new Error('stop'), signalCancelled, signalAborted, pendingSource = new ReadableStream({ pull: function() {}, cancel: function(error) { signalCancelled = error; } }), pendingDestination = new WritableStream({ abort: function(error) { signalAborted = error; } }), signalled = pendingSource.pipeTo(pendingDestination, { signal: controller.signal }).catch(function(error) { return error; }); controller.abort(abortReason); var signalResult = await signalled, preventedController = new AbortController(), preventedCancelled = false, preventedAborted = false, preventedSource = new ReadableStream({ pull: function() {}, cancel: function() { preventedCancelled = true; } }), preventedDestination = new WritableStream({ abort: function() { preventedAborted = true; } }), prevented = preventedSource.pipeTo(preventedDestination, { signal: preventedController.signal, preventCancel: true, preventAbort: true }).catch(function(error) { return error; }); preventedController.abort(abortReason); var preventedResult = await prevented; return [writes, closed, !normalSource.locked, !normalDestination.locked, destinationResult === destinationFailure, cancelledWith === destinationFailure, !cancellableSource.locked, !failingDestination.locked, sourceResult === sourceFailure, abortedWith === sourceFailure, signalResult === abortReason, signalCancelled === abortReason, signalAborted === abortReason, !pendingSource.locked, !pendingDestination.locked, preventedResult === abortReason, preventedCancelled, preventedAborted, !preventedSource.locked, !preventedDestination.locked]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_pipe_to_options_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebPipeTo = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebPipeTo").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["a","b"],false,true,true,true,true,true,true,true,true,true,true,true,true,true,true,false,false,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_pipe_through_validates_locks_and_propagates_errors() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_pipe_through");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { function code(action) { try { action(); } catch (error) { return error.code; } } var lockedSource = new ReadableStream(), lockedReader = lockedSource.getReader(), sourceLockCode = code(function() { lockedSource.pipeThrough(new TransformStream()); }); lockedReader.releaseLock(); var lockedTransform = new TransformStream(), lockedWriter = lockedTransform.writable.getWriter(), destinationLockCode = code(function() { new ReadableStream().pipeThrough(lockedTransform); }); lockedWriter.releaseLock(); var source = new ReadableStream({ start: function(controller) { controller.enqueue('a'); controller.enqueue('b'); controller.close(); } }), upper = new TransformStream({ transform: function(value, controller) { controller.enqueue(value.toUpperCase()); } }), output = source.pipeThrough(upper), values = []; for await (var value of output) values.push(value); var failure = new Error('transform'), cancelledWith, failingSource = new ReadableStream({ start: function(controller) { controller.enqueue('x'); }, cancel: function(error) { cancelledWith = error; } }), failingTransform = new TransformStream({ transform: function() { throw failure; } }), failedOutput = failingSource.pipeThrough(failingTransform), failedReader = failedOutput.getReader(), received; try { await failedReader.read(); } catch (error) { received = error; } await new Promise(function(resolve) { setTimeout(resolve, 0); }); return [sourceLockCode, destinationLockCode, values, !source.locked, !upper.writable.locked, received === failure, cancelledWith === failure, !failingSource.locked]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_pipe_through_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebPipeThrough = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebPipeThrough").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ERR_INVALID_STATE","ERR_INVALID_STATE",["A","B"],true,true,true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_web_adapters_retain_locks_and_await_teardown() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_web_adapter_locks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var normalReadable = new ReadableStream({ start: function(controller) { controller.close(); } }), nodeReadable = stream.Readable.fromWeb(normalReadable); for await (var value of nodeReadable) {} var normalWritable = new WritableStream(), nodeWritable = stream.Writable.fromWeb(normalWritable), normalWritableClosed = new Promise(function(resolve, reject) { nodeWritable.once('error', reject); nodeWritable.once('close', resolve); }); nodeWritable.end(); await normalWritableClosed; var cancelResolve, cancelGate = new Promise(function(resolve) { cancelResolve = resolve; }), readableEvents = [], pendingReadable = new ReadableStream({ cancel: function() { readableEvents.push('cancel'); return cancelGate; } }), destroyedReadable = stream.Readable.fromWeb(pendingReadable), readableClosed = new Promise(function(resolve) { destroyedReadable.once('error', function() { readableEvents.push('error'); }); destroyedReadable.once('close', function() { readableEvents.push('close'); resolve(); }); }), readableReason = new Error('read stop'); destroyedReadable.destroy(readableReason); await Promise.resolve(); var readablePending = [pendingReadable.locked, readableEvents.slice()]; cancelResolve(); await readableClosed; var abortResolve, abortGate = new Promise(function(resolve) { abortResolve = resolve; }), writableEvents = [], pendingWritable = new WritableStream({ abort: function() { writableEvents.push('abort'); return abortGate; } }), destroyedWritable = stream.Writable.fromWeb(pendingWritable), writableClosed = new Promise(function(resolve) { destroyedWritable.once('error', function() { writableEvents.push('error'); }); destroyedWritable.once('close', function() { writableEvents.push('close'); resolve(); }); }), writableReason = new Error('write stop'); destroyedWritable.destroy(writableReason); await Promise.resolve(); await Promise.resolve(); var writablePending = [pendingWritable.locked, writableEvents.slice()]; abortResolve(); await writableClosed; return [normalReadable.locked, normalWritable.locked, readablePending, readableEvents, destroyedReadable.errored === readableReason, pendingReadable.locked, writablePending, writableEvents, destroyedWritable.errored === writableReason, pendingWritable.locked]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_web_adapter_locks_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWebAdapterLocks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWebAdapterLocks").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,[true,["cancel"]],["cancel","error","close"],true,true,[true,["abort"]],["abort","error","close"],true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_to_web_applies_backpressure_and_awaits_cancel() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_to_web_backpressure");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var source = new stream.Readable({ autoDestroy: false, read: function() {} }), web = stream.Readable.toWeb(source), initiallyPaused = source.isPaused(); source.push('a'); source.push('b'); var beforeRead = [source.isPaused(), source.readableLength], reader = web.getReader(), first = await reader.read(), afterFirst = [source.isPaused(), source.readableLength], second = await reader.read(); source.push(null); var done = await reader.read(), reason = new Error('stop'), events = [], cancelledSource = new stream.Readable({ read: function() {} }), cancelledWeb = stream.Readable.toWeb(cancelledSource), cancelledReader = cancelledWeb.getReader(); cancelledSource.once('error', function(error) { events.push(['error', error === reason]); }); cancelledSource.once('close', function() { events.push(['close']); }); var cancellation = cancelledReader.cancel(reason).then(function() { events.push(['cancelled']); }); var beforeCancelSettlement = events.slice(); await cancellation; return [initiallyPaused, beforeRead, first.value.toString(), afterFirst, second.value.toString(), done.done, beforeCancelSettlement, events, cancelledSource.destroyed, cancelledWeb.locked]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_to_web_backpressure_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableToWebBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableToWebBackpressure")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,[true,2],"a",[true,0],"b",true,[],[["error",true],["close"],["cancelled"]],true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn duplex_from_body_functions_are_writable_sinks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_duplex_from_functions");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var values = [], receivedSignal = false, sink = stream.Duplex.from(async function(source, options) { receivedSignal = options.signal instanceof AbortSignal; for await (var value of source) values.push(String(value)); }), sinkReadable = sink.readable, sinkWritable = sink.writable, closed = new Promise(function(resolve, reject) { sink.on('error', reject); sink.once('close', resolve); }); sink.write('a'); sink.end('b'); await closed; var invalid = stream.Duplex.from(async function(source) { for await (var value of source) {} return 42; }), invalidCode = new Promise(function(resolve) { invalid.once('error', function(error) { resolve(error.code); }); }); invalid.end('x'); return [sinkReadable, sinkWritable, values, receivedSignal, await invalidCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_duplex_from_functions_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDuplexFromFunctions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseDuplexFromFunctions")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,true,["a","b"],true,"ERR_INVALID_RETURN_VALUE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_compose_waits_for_functional_sinks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compose_sink");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var values = [], order = []; async function* identity(source) { for await (var value of source) yield value; } async function sink(source) { for await (var value of source) { await Promise.resolve(); values.push(String(value)); } order.push('sink'); } var composed = stream.compose(identity, sink), readable = composed.readable, writable = composed.writable, closed = new Promise(function(resolve, reject) { composed.once('error', reject); composed.once('finish', function() { order.push('finish'); }); composed.once('close', resolve); }); composed.write('a'); composed.end('b'); await closed; var failure = new Error('sink failed'); async function reject(source) { for await (var value of source) throw failure; } var rejected = stream.compose(identity, reject), rejection = new Promise(function(resolve) { rejected.once('error', resolve); }); rejected.end('x'); var received = await rejection; return [readable, writable, values, order, composed.destroyed, received === failure, rejected.destroyed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compose_sink_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamComposeSink = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamComposeSink").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,true,["a","b"],["sink","finish"],true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_compose_and_duplex_from_validate_sources() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compose_validation");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); function code(action) { try { action(); } catch (error) { return error.code; } } module.exports = function () { function pass() { return new stream.PassThrough(); } function writable() { return new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); } function readable() { return new stream.Readable(); } var single = stream.compose(pass()), promised = stream.Duplex.from(Promise.resolve('x')), text = stream.Duplex.from('abc'); return [code(function() { stream.compose(); }), code(function() { stream.compose({}); }), code(function() { stream.compose(writable(), pass()); }), code(function() { stream.compose(pass(), readable()); }), code(function() { stream.Duplex.from(null); }), code(function() { stream.Duplex.from(42); }), code(function() { stream.Duplex.from({}); }), single.readable, single.writable, promised.readable, promised.writable, text.readable, text.writable]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compose_validation_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamComposeValidation = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamComposeValidation")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ERR_MISSING_ARGS","ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_VALUE","ERR_INVALID_ARG_VALUE","ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_TYPE",true,true,true,false,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn duplex_from_normalizes_scalar_chunks_and_null_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_duplex_from_chunks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); async function consume(source) { var duplex = stream.Duplex.from(source), values = []; try { for await (var value of duplex) values.push(Buffer.isBuffer(value) ? 'buffer:' + value.toString() : typeof value + ':' + String(value)); return [values, duplex.readableObjectMode, duplex.writableObjectMode]; } catch (error) { return error.code; } } module.exports = async function () { return [await consume('abc'), await consume(Buffer.from('abc')), await consume(new Uint8Array([97, 98, 99])), await consume(Promise.resolve('abc')), await consume(Promise.resolve(null)), await consume([null])]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_duplex_from_chunks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDuplexFromChunks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseDuplexFromChunks").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[["string:abc"],true,true],[["buffer:abc"],true,true],[["number:97","number:98","number:99"],true,true],[["string:abc"],true,true],[[],true,true],"ERR_STREAM_NULL_VALUES"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn duplex_from_destroy_closes_the_source_iterator() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_duplex_from_iterator_cleanup");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var finalized = false, chunks = [], reason = new Error('stop'); async function* values() { try { yield 'first'; yield 'second'; } finally { finalized = true; } } var duplex = stream.Duplex.from(values()), closed = new Promise(function(resolve) { duplex.once('close', resolve); }); duplex.on('error', function() {}); duplex.on('data', function(chunk) { chunks.push(chunk); if (chunks.length === 1) duplex.destroy(reason); }); await closed; await Promise.resolve(); var returns = 0, count = 0, iterable = { [Symbol.asyncIterator]: function() { return { next: function() { count++; return Promise.resolve({ value: count, done: false }); }, return: function() { returns++; return Promise.resolve({ done: true }); } }; } }, repeated = stream.Duplex.from(iterable), repeatedClosed = new Promise(function(resolve) { repeated.once('close', resolve); }); repeated.on('error', function() {}); repeated.once('data', function() { repeated.destroy(); repeated.destroy(); }); await repeatedClosed; return [chunks, finalized, duplex.errored === reason, duplex.destroyed, returns]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_duplex_from_iterator_cleanup_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDuplexIteratorCleanup = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseDuplexIteratorCleanup")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["first"],true,true,true,1]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_compose_propagates_premature_stage_close() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compose_close");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); async function closeStage(position) { var first = new stream.PassThrough(), last = new stream.PassThrough(), composed = stream.compose(first, last), events = []; first.on('error', function(error) { events.push(['first', error.code]); }); last.on('error', function(error) { events.push(['last', error.code]); }); composed.on('error', function(error) { events.push(['composed', error.code]); }); var closed = new Promise(function(resolve) { composed.once('close', resolve); }); (position === 'first' ? first : last).destroy(); await closed; await Promise.resolve(); await Promise.resolve(); return [events, first.destroyed, last.destroyed, composed.destroyed, composed.errored.code]; } module.exports = async function () { return [await closeStage('first'), await closeStage('last')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compose_close_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamComposeClose = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamComposeClose").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[[["last","ERR_STREAM_PREMATURE_CLOSE"],["composed","ERR_STREAM_PREMATURE_CLOSE"]],true,true,true,"ERR_STREAM_PREMATURE_CLOSE"],[[["first","ERR_STREAM_PREMATURE_CLOSE"],["composed","ERR_STREAM_PREMATURE_CLOSE"]],true,true,true,"ERR_STREAM_PREMATURE_CLOSE"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_state_predicates_track_lifecycle() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_state");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var readable = new stream.Readable(), writable = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); var initial = [stream.isReadable(readable), stream.isWritable(writable), stream.isDestroyed(readable), stream.isDisturbed(readable), stream.isErrored(readable), stream.isReadable(null)]; readable.on('error', function() {}); readable.on('data', function() {}); readable.push('value'); var consumed = stream.isDisturbed(readable); writable.end(); var ended = stream.isWritable(writable); var failure = new Error('failed'); readable.destroy(failure); return [initial, consumed, ended, stream.isDestroyed(readable), stream.isErrored(readable), readable.errored === failure, stream.isReadable(readable)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_state_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamState = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamState").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[true,true,false,false,false,false],true,false,true,true,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_extended_state_unshift_wrap_and_default_encoding_work() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_extended_state");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var ordered = new stream.Readable(); ordered.pause(); ordered.push('b'); ordered.unshift('a'); var unshifted = ordered.read().toString(); var source = new stream.Readable(), wrapped = new stream.Readable().wrap(source), wrappedValues = []; wrapped.on('data', function(value) { wrappedValues.push(value.toString()); }); source.push('wrapped'); source.push(null); var completion, draining = new stream.Writable({ highWaterMark: 1, write: function(chunk, encoding, callback) { completion = callback; } }), writeResult = draining.write('x'), needed = draining.writableNeedDrain; completion(); var afterDrain = draining.writableNeedDrain, encoded = []; var encoder = new stream.Writable({ write: function(chunk, encoding, callback) { encoded.push([chunk.toString(), encoding]); callback(); } }); encoder.setDefaultEncoding('hex').end('6869'); var readableError = new Error('readable failure'), abortedReadable = new stream.Readable(); abortedReadable.on('error', function() {}); abortedReadable.destroy(readableError); var abortedWritable = new stream.Writable(); abortedWritable.destroy(); return [unshifted, wrappedValues, wrapped.readableFlowing, writeResult, needed, afterDrain, encoded, abortedReadable.closed, abortedReadable.readableAborted, abortedReadable.errored === readableError, abortedWritable.writableAborted, ordered.readableObjectMode, encoder.writableObjectMode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_extended_state_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamExtendedState = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamExtendedState")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ab",["wrapped"],true,false,true,false,[["hi","buffer"]],true,true,true,true,false,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_indexed_pairs_static_checks_and_half_open_control_work() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_remaining_lifecycle");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var pairs = await stream.Readable.from(['a', 'b']).asIndexedPairs().toArray(); var readable = stream.Readable.from([1]), before = [stream.Readable.isDisturbed(readable), stream.Stream.isDestroyed(readable), stream.Stream.isReadable(readable), stream.Stream.isWritable(readable)]; await readable.toArray(); var duplex = new stream.Duplex({ allowHalfOpen: false }); duplex.push(null); var disposed = new stream.Writable(), symbolDisposed = false; if (Symbol.asyncDispose && disposed[Symbol.asyncDispose]) { await disposed[Symbol.asyncDispose](); symbolDisposed = disposed.destroyed; } var soon = new stream.Writable(); soon.destroySoon(); return [pairs, before, stream.Readable.isDisturbed(readable), duplex.allowHalfOpen, duplex.writableEnded, duplex.writableFinished, symbolDisposed, soon.writableEnded, soon.writableFinished]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_remaining_lifecycle_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamRemainingLifecycle = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamRemainingLifecycle")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[[0,"a"],[1,"b"]],[false,false,true,false],true,false,true,true,true,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_destroy_hooks_finish_error_and_close_once() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_destroy_hooks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var events = [], replacement = new Error('replacement'), readable = new stream.Readable({ destroy: function(error, callback) { events.push(['hook', error.message]); queueMicrotask(function() { callback(replacement); callback(new Error('ignored')); }); } }); readable.on('error', function(error) { events.push(['error', error.message]); }); var closed = new Promise(function(resolve) { readable.on('close', function() { events.push(['close']); resolve(); }); }); readable.destroy(new Error('original')); var immediate = [readable.destroyed, readable.closed, readable.readableAborted]; await closed; var writableError, writable = new stream.Writable({ destroy: function(error, callback) { callback(null); } }); writable.on('error', function(error) { writableError = error; }); var writableClosed = new Promise(function(resolve) { writable.on('close', resolve); }); writable.destroy(new Error('writable-original')); var thrownError, throwing = new stream.Readable({ destroy: function() { throw new Error('hook-threw'); } }); throwing.on('error', function(error) { thrownError = error; }); var throwingClosed = new Promise(function(resolve) { throwing.on('close', resolve); }); throwing.destroy(); await Promise.all([writableClosed, throwingClosed]); return [immediate, events, readable.closed, readable.errored === replacement, writableError.message, writable.closed, thrownError.message, throwing.closed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_destroy_hooks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamDestroyHooks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamDestroyHooks").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[true,false,true],[["hook","original"],["error","replacement"],["close"]],true,true,"writable-original",true,"hook-threw",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_construct_hooks_gate_io_and_propagate_failures() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_construct_hooks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var writableEvents = [], writable = new stream.Writable({ construct: function(callback) { writableEvents.push('construct'); queueMicrotask(function() { writableEvents.push('constructed'); callback(); }); }, write: function(chunk, encoding, callback) { writableEvents.push('write:' + chunk.toString()); callback(); }, final: function(callback) { writableEvents.push('final'); callback(); } }); writable.on('finish', function() { writableEvents.push('finish'); }); writable.end('value'); var beforeConstruct = writableEvents.slice(); await new Promise(function(resolve, reject) { writable.on('finish', resolve); writable.on('error', reject); }); var readableEvents = [], readable = new stream.Readable({ construct: function(callback) { readableEvents.push('construct'); queueMicrotask(callback); }, read: function() { readableEvents.push('read'); this.push('data'); this.push(null); } }), values = []; readable.on('data', function(chunk) { values.push(chunk.toString()); }); await new Promise(function(resolve, reject) { readable.on('end', resolve); readable.on('error', reject); }); var failure = new Error('construct-failed'), failedEvents = [], failed = new stream.Writable({ construct: function(callback) { queueMicrotask(function() { callback(failure); }); }, write: function(chunk, encoding, callback) { failedEvents.push('write'); callback(); }, destroy: function(error, callback) { failedEvents.push('destroy:' + error.message); callback(error); } }); failed.on('error', function(error) { failedEvents.push('error:' + error.message); }); var writeError, failedClosed = new Promise(function(resolve) { failed.on('close', resolve); }); failed.write('never', function(error) { writeError = error; }); await failedClosed; return [beforeConstruct, writableEvents, readableEvents, values, failedEvents, writeError === failure, failed.closed, failed.errored === failure]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_construct_hooks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamConstructHooks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamConstructHooks")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[],["construct","constructed","write:value","final","finish"],["construct","read"],["data"],["destroy:construct-failed","error:construct-failed"],true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn streams_expose_event_emitter_listener_management() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_event_emitter");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var value = new stream.PassThrough(), symbol = Symbol('event'), calls = []; function regular(input) { calls.push('regular:' + input); } function prepended(input) { calls.push('prepended:' + input); } function once(input) { calls.push('once:' + input); } value.on(symbol, regular); value.prependListener(symbol, prepended); value.prependOnceListener(symbol, once); var before = [value.listenerCount(symbol), value.listenerCount(symbol, regular), value.listeners(symbol)[1] === prepended, value.rawListeners(symbol).length, value.eventNames()[0] === symbol]; value.emit(symbol, 1); value.emit(symbol, 2); var afterOnce = value.listenerCount(symbol); value.removeListener(symbol, regular); var afterRemove = value.listenerCount(symbol); value.removeAllListeners(symbol); var afterAll = [value.listenerCount(symbol), value.eventNames().length]; var chained = value.setMaxListeners(25) === value, maximum = value.getMaxListeners(); return [before, calls, afterOnce, afterRemove, afterAll, chained, maximum, value.addListener === value.on]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_event_emitter_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamEvents = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamEvents").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[3,1,true,3,true],["once:1","prepended:1","regular:1","prepended:2","regular:2"],2,1,[0,0],true,25,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_listener_meta_events_track_single_and_bulk_removal() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_listener_meta_events");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var value = new stream.Stream(), meta = [], calls = []; value.on('newListener', function(name, listener) { if (name !== 'newListener') meta.push('new:' + String(name) + ':' + listener.name); }); value.on('removeListener', function(name, listener) { meta.push('remove:' + String(name) + ':' + listener.name); }); function duplicate() { calls.push('duplicate'); } value.on('target', duplicate); value.on('target', duplicate); var before = value.listenerCount('target', duplicate); value.removeListener('target', duplicate); var afterOne = value.listenerCount('target', duplicate); value.emit('target'); function first() {} function second() {} value.on('bulk', first); value.on('bulk', second); value.removeAllListeners('bulk'); var bulkCount = value.listenerCount('bulk'), removeTail = meta.slice(-2); function oneTime() { calls.push('once'); } value.once('once', oneTime); var raw = value.rawListeners('once')[0], rawWrapper = raw !== oneTime && raw.listener === oneTime; raw(); return [before, afterOne, calls, bulkCount, removeTail, meta.indexOf('remove:once:oneTime') >= 0, rawWrapper, value.listenerCount('once')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_listener_meta_events_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamMetaEvents = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamMetaEvents").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[2,1,["duplicate","once"],0,["remove:bulk:second","remove:bulk:first"],true,true,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn streams_defer_destroy_errors_and_still_close_once() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_unhandled_errors");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var expected = new Error('expected'), direct, contextError; try { new stream.Stream().emit('error', expected); } catch (error) { direct = error === expected; } var context = { value: 1 }; try { new stream.Stream().emit('error', context); } catch (error) { contextError = [error.code, error.context === context]; } var handledValue, handled = new stream.Stream(); handled.on('error', function(error) { handledValue = error; }); var handledResult = handled.emit('error', expected); var events = [], destroyed = new stream.Readable(); destroyed.on('error', function(error) { events.push('error:' + error.message); }); destroyed.on('close', function() { events.push('close'); }); destroyed.destroy(expected); events.push('after'); var immediate = [destroyed.destroyed, destroyed.closed, destroyed.errored === expected]; var hooked = new stream.Readable({ destroy: function(error, callback) { callback(error); callback(new Error('ignored')); } }); hooked.on('error', function(error) { events.push('hook-error:' + error.message); }); hooked.on('close', function() { events.push('hook-close'); }); hooked.destroy(expected); await new Promise(function(resolve) { queueMicrotask(resolve); }); return [direct, contextError, handledResult, handledValue === expected, immediate, hooked.closed, hooked.errored === expected, events]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_unhandled_errors_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamUnhandledErrors = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamUnhandledErrors")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,["ERR_UNHANDLED_ERROR",true],true,true,[true,true,true],true,true,["after","error:expected","close","hook-error:expected","hook-close"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_set_encoding_preserves_split_multibyte_characters() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_incremental_encoding");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var snow = Buffer.from('雪'), flowing = new stream.Readable().setEncoding('utf8'), events = []; flowing.on('data', function(value) { events.push(value); }); flowing.push(snow.subarray(0, 1)); flowing.push(snow.subarray(1, 2)); flowing.push(snow.subarray(2)); flowing.push(null); var buffered = new stream.Readable(); buffered.push(Buffer.from([0x41, snow[0]])); buffered.push(snow.subarray(1)); buffered.setEncoding('utf-8'); var bufferedLength = buffered.readableLength, first = buffered.read(1), second = buffered.read(); var face = Buffer.from('😀', 'utf16le'), wide = new stream.Readable().setEncoding('utf16le'), wideEvents = []; wide.on('data', function(value) { wideEvents.push(value); }); wide.push(face.subarray(0, 2)); wide.push(face.subarray(2)); wide.push(null); var incomplete = new stream.Readable().setEncoding('utf8'), incompleteValues = []; incomplete.on('data', function(value) { incompleteValues.push(value); }); incomplete.push(Buffer.from([0xe9])); incomplete.push(null); var invalid = false; try { new stream.Readable().setEncoding('missing'); } catch (error) { invalid = error instanceof TypeError; } return [events, bufferedLength, first, second, wideEvents, incompleteValues, invalid, flowing.readableEncoding]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_incremental_encoding_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamEncoding = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamEncoding").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["雪"],2,"A","雪",["😀"],["�"],true,"utf8"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_exposes_node_compatibility_helpers_and_duplex_pairs() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_compatibility_helpers");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var bytes = new Uint8Array([1, 2]), shared = stream._uint8ArrayToBuffer(bytes); bytes[0] = 9; var views = [stream._isUint8Array(shared), stream._isUint8Array(new Uint16Array(1)), stream._isArrayBufferView(new DataView(new ArrayBuffer(2))), stream._isArrayBufferView(new ArrayBuffer(2))]; var pair = stream.duplexPair({ objectMode: true }), leftValues = [], rightValues = []; pair[0].on('data', function(value) { leftValues.push(value); }); pair[1].on('data', function(value) { rightValues.push(value); }); pair[0].write({ side: 'right' }); pair[1].write({ side: 'left' }); pair[0].end(); pair[1].end(); var destroyed = new stream.Readable(), destroyError; destroyed.on('error', function(error) { destroyError = [error.name, error.code]; }); var destroyResult = stream.destroy(destroyed); await Promise.resolve(); var promiseResult = await stream.promises.pipeline([1, 2], async function(source) { var total = 0; for await (var value of source) total += value; return total; }); return [[].slice.call(shared), views, leftValues, rightValues, pair[0].readableEnded, pair[1].readableEnded, destroyResult === undefined, destroyed.destroyed, destroyError, promiseResult]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_compatibility_helpers_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamHelpers = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamHelpers").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[9,2],[true,false,true,false],[{"side":"left"}],[{"side":"right"}],true,true,true,true,["AbortError","ABORT_ERR"],3]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_pipe_reports_node_error_and_streams_can_be_undestroyed() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_pipe_error_undestroy");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var writable = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), destination = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), observed; writable.on('error', function(error) { observed = [error.code, error.message]; }); var returned = writable.pipe(destination); var immediate = observed; await Promise.resolve(); await Promise.resolve(); var destroyed = writable.destroyed; writable._undestroy(); var restored = [writable.destroyed, writable.closed, writable.errored, writable.writable, writable.writableAborted]; var readable = new stream.Readable(); readable.destroy(); readable._undestroy(); return [returned === destination, immediate === undefined, observed, destroyed, restored, readable.destroyed, readable.closed, readable.readable, readable.readableAborted]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_pipe_error_undestroy_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWritablePipe = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWritablePipe").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,["ERR_STREAM_CANNOT_PIPE","Cannot pipe, not readable"],true,[false,false,null,true,false],false,false,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn streams_expose_buffer_state_and_readable_compose() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_buffer_state_compose");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var readable = new stream.Readable(); readable.push('a'); readable.push('b'); var before = [readable.readableDidRead, readable.readableBuffer.length, readable.readableBuffer[0].toString()]; var value = readable.read().toString(), after = [readable.readableDidRead, readable.readableBuffer.length]; var callbacks = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { callbacks.push(callback); } }); writable.write('first'); writable.write('second'); var queued = [writable.writableBuffer.length, writable.writableBuffer[0].chunk.toString(), writable.writableBuffer[0].encoding]; callbacks.shift()(); callbacks.shift()(); var doubled = stream.Readable.from([1, 2]).compose(new stream.Transform({ objectMode: true, transform: function(item, encoding, callback) { callback(null, item * 2); } })); var composed = await doubled.toArray(); return [before, value, after, queued, composed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_buffer_state_compose_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamBufferState = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamBufferState").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[false,2,"a"],"ab",[true,0],[1,"second","buffer"],[2,4]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn legacy_stream_pipe_forwards_manual_events_and_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_legacy_stream_pipe");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var source = new stream.Stream(), values = [], pipeSource, destination = new stream.Writable({ write: function(chunk, encoding, callback) { values.push(chunk.toString()); callback(); } }); destination.on('pipe', function(value) { pipeSource = value; }); var returned = source.pipe(destination); source.emit('data', Buffer.from('a')); source.emit('data', Buffer.from('b')); source.emit('end'); var openSource = new stream.Stream(), openEnded = false, openDestination = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); openDestination.on('finish', function() { openEnded = true; }); openSource.pipe(openDestination, { end: false }); openSource.emit('end'); return [returned === destination, pipeSource === source, values, destination.writableEnded, destination.writableFinished, openDestination.writableEnded, openEnded, source.listenerCount('data'), openSource.listenerCount('end')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_legacy_stream_pipe_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseLegacyStreamPipe = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseLegacyStreamPipe").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,["a","b"],true,false,false,false,0,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_subclasses_keep_prototype_lifecycle_hooks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_subclass_hooks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { class Source extends stream.Readable { constructor() { super(); this.sent = false; } _read() { if (!this.sent) { this.sent = true; this.push('source'); this.push(null); } } } class Sink extends stream.Writable { constructor() { super(); this.values = []; this.finalized = false; this.cleaned = false; } _write(chunk, encoding, callback) { this.values.push(chunk.toString()); callback(); } _final(callback) { this.finalized = true; callback(); } _destroy(error, callback) { this.cleaned = true; callback(error); } } class Upper extends stream.Transform { _transform(chunk, encoding, callback) { callback(null, chunk.toString().toUpperCase()); } _flush(callback) { callback(null, '!'); } } var source = new Source(), read = source.read().toString(), sink = new Sink(); sink.write('a'); sink.end('b'); sink.destroy(); var upper = new Upper(), transformed = []; upper.on('data', function(chunk) { transformed.push(chunk.toString()); }); upper.end('thaw'); var missing = new stream.Writable(), missingCode, callbackCode; missing.on('error', function(error) { missingCode = error.code; }); missing.write('x', function(error) { callbackCode = error.code; }); await Promise.resolve(); return [read, sink.values, sink.finalized, sink.cleaned, transformed, missingCode, callbackCode, missing.destroyed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_subclass_hooks_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamSubclassHooks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamSubclassHooks")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["source",["a","b"],true,true,["THAW","!"],"ERR_METHOD_NOT_IMPLEMENTED","ERR_METHOD_NOT_IMPLEMENTED",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_subclass_construct_hooks_gate_io_and_fail_normally() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_subclass_construct");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var readableEvents = [], releaseReadable; class Source extends stream.Readable { _construct(callback) { readableEvents.push('construct'); releaseReadable = callback; } _read() { readableEvents.push('read'); this.push('ready'); this.push(null); } } var source = new Source(), values = []; source.on('data', function(value) { values.push(value.toString()); }); await Promise.resolve(); var readableBefore = readableEvents.slice(); releaseReadable(); await Promise.resolve(); var writableEvents = [], releaseWritable; class Sink extends stream.Writable { _construct(callback) { writableEvents.push('construct'); releaseWritable = callback; } _write(chunk, encoding, callback) { writableEvents.push(chunk.toString()); callback(); } } var sink = new Sink(); sink.end('queued'); await Promise.resolve(); var writableBefore = writableEvents.slice(); releaseWritable(); await Promise.resolve(); class Broken extends stream.Readable { _construct(callback) { callback(new Error('construct-failed')); } } var broken = new Broken(), failure; broken.on('error', function(error) { failure = error.message; }); broken.resume(); await Promise.resolve(); await Promise.resolve(); return [readableBefore, readableEvents, values, writableBefore, writableEvents, broken.destroyed, broken.closed, failure]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_subclass_construct_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamSubclassConstruct = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamSubclassConstruct")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["construct"],["construct","read"],["ready"],["construct"],["construct","queued"],true,true,"construct-failed"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn streams_auto_destroy_after_completion_and_honor_close_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_auto_destroy");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var readableEvents = [], readable = new stream.Readable({ read: function() { this.push(null); } }); readable.on('end', function() { readableEvents.push('end'); }); readable.on('close', function() { readableEvents.push('close'); }); readable.resume(); var writableEvents = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); writable.on('finish', function() { writableEvents.push('finish'); }); writable.on('close', function() { writableEvents.push('close'); }); writable.end(); var persistent = new stream.Readable({ autoDestroy: false, read: function() { this.push(null); } }); persistent.resume(); var silentEvents = [], silent = new stream.Writable({ emitClose: false, write: function(chunk, encoding, callback) { callback(); } }); silent.on('finish', function() { silentEvents.push('finish'); }); silent.on('close', function() { silentEvents.push('close'); }); silent.end(); var duplex = new stream.Duplex({ read: function() {}, write: function(chunk, encoding, callback) { callback(); } }); duplex.end(); var afterOneSide = duplex.destroyed; duplex.push(null); await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); return [readableEvents, readable.destroyed, readable.closed, writableEvents, writable.destroyed, writable.closed, persistent.destroyed, persistent.closed, silentEvents, silent.destroyed, silent.closed, afterOneSide, duplex.destroyed, duplex.closed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_auto_destroy_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamAutoDestroy = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamAutoDestroy").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["end","close"],true,true,["finish","close"],true,true,false,false,["finish"],true,true,false,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_decode_strings_and_chunk_validation_match_node() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_writable_decode_strings");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var release, preserved, writable = new stream.Writable({ decodeStrings: false, write: function(chunk, encoding, callback) { preserved = [typeof chunk, chunk, encoding]; release = callback; } }); writable.write('雪'); var preservedLength = writable.writableLength; release(); var converted, normal = new stream.Writable({ write: function(chunk, encoding, callback) { converted = [Buffer.isBuffer(chunk), chunk.toString(), encoding]; callback(); } }); normal.end('thaw'); var binary = [], binarySink = new stream.Writable({ write: function(chunk, encoding, callback) { binary.push([].slice.call(chunk)); callback(); } }); binarySink.write(new Uint16Array([258])); binarySink.end(new DataView(Uint8Array.from([3, 4]).buffer)); var nullCode, typeCode; try { new stream.Writable().write(null); } catch (error) { nullCode = error.code; } try { new stream.Writable().write({}); } catch (error) { typeCode = error.code; } var objects = [], objectSink = new stream.Writable({ objectMode: true, write: function(value, encoding, callback) { objects.push(value); callback(); } }); objectSink.end({ valid: true }); return [preserved, preservedLength, converted, binary, nullCode, typeCode, objects]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_writable_decode_strings_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWritableDecoding = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWritableDecoding").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["string","雪","utf8"],1,[true,"thaw","buffer"],[[2,1],[3,4]],"ERR_STREAM_NULL_VALUES","ERR_INVALID_ARG_TYPE",[{"valid":true}]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_rejects_writes_after_end_or_destroy_asynchronously() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_writable_late_writes");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var endedEvents = [], ended = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); ended.on('error', function(error) { endedEvents.push(['error', error.code]); }); ended.end(); var endedReturn = ended.write('late', function(error) { endedEvents.push(['callback', error.code]); }); var endedImmediate = endedEvents.slice(); var destroyedEvents = [], writes = 0, destroyed = new stream.Writable({ write: function(chunk, encoding, callback) { writes++; callback(); } }); destroyed.on('error', function(error) { destroyedEvents.push(['error', error.code]); }); destroyed.destroy(); var destroyedReturn = destroyed.write('late', function(error) { destroyedEvents.push(['callback', error.code]); }); var destroyedImmediate = destroyedEvents.slice(); await Promise.resolve(); await Promise.resolve(); return [endedReturn, endedImmediate, endedEvents, ended.errored.code, destroyedReturn, destroyedImmediate, destroyedEvents, writes]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_writable_late_writes_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWritableLateWrites = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWritableLateWrites").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,[],[["callback","ERR_STREAM_WRITE_AFTER_END"],["error","ERR_STREAM_WRITE_AFTER_END"]],"ERR_STREAM_WRITE_AFTER_END",false,[],[["callback","ERR_STREAM_DESTROYED"]],0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_rejects_invalid_and_post_eof_chunks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_chunk_validation");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var events = [], ended = new stream.Readable({ autoDestroy: false }); ended.on('error', function(error) { events.push(error.code); }); var eof = ended.push(null), late = ended.push('late'); var invalidEvents = [], invalid = new stream.Readable({ autoDestroy: false }); invalid.on('error', function(error) { invalidEvents.push(error.code); }); var invalidReturn = invalid.push({}); var binary = new stream.Readable({ autoDestroy: false }), first = binary.push(new Uint16Array([258])), second = binary.push(new DataView(Uint8Array.from([3, 4]).buffer)), bytes = binary.read(); var destroyed = new stream.Readable(); destroyed.destroy(); var destroyedReturn = destroyed.push('ignored'); var empty = new stream.Readable({ autoDestroy: false }), undefinedReturn = empty.push(undefined); return [eof, late, events, ended.errored.code, ended.readableAborted, ended.destroyed, invalidReturn, invalidEvents, invalid.errored.code, first, second, [].slice.call(bytes), destroyedReturn, destroyed.readableLength, undefinedReturn, empty.readableLength]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_chunk_validation_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableValidation = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableValidation").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,false,["ERR_STREAM_PUSH_AFTER_EOF"],"ERR_STREAM_PUSH_AFTER_EOF",true,false,false,["ERR_INVALID_ARG_TYPE"],"ERR_INVALID_ARG_TYPE",true,true,[2,1,3,4],false,0,true,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_read_waits_for_requested_size_and_preserves_modes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_sized_reads");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var buffered = new stream.Readable({ autoDestroy: false }); buffered.push('ab'); var zero = buffered.read(0), tooSoon = buffered.read(3), before = buffered.readableLength; buffered.push('c'); var exact = buffered.read(3).toString(); var ended = new stream.Readable({ autoDestroy: false }); ended.push('xy'); ended.push(null); var final = ended.read(3).toString(), endedLength = ended.readableLength; var encoded = new stream.Readable({ autoDestroy: false }).setEncoding('utf8'); encoded.push('A雪'); encoded.push(null); var first = encoded.read(1), encodedBefore = encoded.readableLength, rest = encoded.read(2); var objects = new stream.Readable({ autoDestroy: false, objectMode: true }), object = { value: 1 }; objects.push(object); var objectRead = objects.read(99) === object; var requested = [], generated = new stream.Readable({ autoDestroy: false, read: function(size) { requested.push(size); if (requested.length === 1) this.push('a'); else this.push('bc'); } }); var generatedFirst = generated.read(3), generatedSecond = generated.read(3).toString(); return [zero, tooSoon, before, exact, final, endedLength, first, encodedBefore, rest, objectRead, requested, generatedFirst, generatedSecond]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_sized_reads_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableSizes = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableSizes").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[null,null,2,"abc","xy",0,"A",1,"雪",true,[3,3],null,"abc"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_unshift_validates_chunks_and_distinguishes_end_event() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_unshift_validation");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var beforeEnd = new stream.Readable({ autoDestroy: false }); beforeEnd.push('b'); beforeEnd.push(null); var beforeReturn = beforeEnd.unshift('a'), beforeValue = beforeEnd.read().toString(); var afterEnd = new stream.Readable({ autoDestroy: false }), afterCode; afterEnd.on('error', function(error) { afterCode = error.code; }); var ended = new Promise(function(resolve) { afterEnd.on('end', resolve); }); afterEnd.push('x'); afterEnd.push(null); afterEnd.resume(); await ended; var afterReturn = afterEnd.unshift('z'); var binary = new stream.Readable({ autoDestroy: false }); var typed = binary.unshift(new Uint16Array([258])), view = binary.unshift(new DataView(Uint8Array.from([3, 4]).buffer)), bytes = binary.read(); var invalid = new stream.Readable({ autoDestroy: false }), invalidCode; invalid.on('error', function(error) { invalidCode = error.code; }); var invalidReturn = invalid.unshift({}), undefinedReturn = invalid.unshift(undefined); var destroyed = new stream.Readable(); destroyed.destroy(); return [beforeReturn, beforeValue, afterReturn, afterCode, afterEnd.readableLength, typed, view, [].slice.call(bytes), invalidReturn, invalidCode, undefinedReturn, invalid.readableLength, destroyed.unshift('ignored')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_unshift_validation_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableUnshift = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableUnshift").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,"ab",false,"ERR_STREAM_UNSHIFT_AFTER_END_EVENT",0,true,true,[3,4,2,1],false,"ERR_INVALID_ARG_TYPE",true,0,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_unpipe_detaches_destinations_and_tracks_backpressure() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_unpipe");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { function sink(values, unpipes) { var output = new stream.Writable({ write: function(chunk, encoding, callback) { values.push(chunk.toString()); callback(); } }); output.on('unpipe', function(source, info) { unpipes.push([source === readable, info.hasUnpiped]); }); return output; } var readable = new stream.Readable({ autoDestroy: false }), left = [], right = [], leftUnpipes = [], rightUnpipes = [], leftSink = sink(left, leftUnpipes), rightSink = sink(right, rightUnpipes); readable.pipe(leftSink); readable.pipe(rightSink); readable.push('x'); readable.unpipe(leftSink); readable.push('y'); var individualListeners = readable.listenerCount('data'); var allReturn = readable.unpipe(); readable.push('z'); var allListeners = readable.listenerCount('data'); var releases = [], slowValues = [], fastValues = [], pressured = new stream.Readable({ autoDestroy: false }), slow = new stream.Writable({ highWaterMark: 1, write: function(chunk, encoding, callback) { slowValues.push(chunk.toString()); releases.push(callback); } }), fast = new stream.Writable({ write: function(chunk, encoding, callback) { fastValues.push(chunk.toString()); callback(); } }); pressured.pipe(slow); pressured.pipe(fast); pressured.push('a'); pressured.push('b'); var paused = pressured.isPaused(), buffered = pressured.readableLength; releases.shift()(); await Promise.resolve(); var resumed = !pressured.isPaused(); releases.shift()(); return [left, right, leftUnpipes, rightUnpipes, individualListeners, allReturn === readable, allListeners, readable.readableLength, slowValues, fastValues, paused, buffered, resumed, pressured.readableLength]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_unpipe_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableUnpipe = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableUnpipe").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["x"],["x","y"],[[true,false]],[[true,false]],1,true,0,1,["a","b"],["a","b"],true,1,false,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_pause_resume_transitions_are_idempotent() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_pause_resume");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var readable = new stream.Readable({ autoDestroy: false }), events = [], values = []; readable.on('pause', function() { events.push('pause'); }); readable.on('resume', function() { events.push('resume'); }); var initial = [readable.readableFlowing, readable.isPaused()]; var pauseOne = readable.pause() === readable, pauseTwo = readable.pause() === readable, paused = [readable.readableFlowing, readable.isPaused(), events.slice()]; var resumeOne = readable.resume() === readable, resumeTwo = readable.resume() === readable, immediate = [readable.readableFlowing, readable.isPaused(), events.slice()]; await Promise.resolve(); var resumedEvents = events.slice(); readable.on('data', function(value) { values.push(value.toString()); }); readable.pause(); readable.push('buffered'); var buffered = [values.slice(), readable.readableLength]; readable.resume(); var drained = [values.slice(), readable.readableLength]; await Promise.resolve(); return [initial, pauseOne, pauseTwo, paused, resumeOne, resumeTwo, immediate, resumedEvents, buffered, drained, events]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_pause_resume_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableFlowing = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableFlowing").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[null,false],true,true,[false,true,["pause"]],true,true,[true,false,["pause"]],["pause","resume"],[[],8],[["buffered"],0],["pause","resume","pause","resume"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_object_modes_preserve_values_and_directional_state() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_object_mode");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var objects = [], doubled = new stream.Transform({ objectMode: true, transform: function(value, encoding, callback) { callback(null, { value: value.value * 2 }); } }); doubled.on('data', function(value) { objects.push(value); }); var first = doubled.write({ value: 2 }); doubled.end({ value: 3 }); var encoded = [], encoder = new stream.Transform({ writableObjectMode: true, transform: function(value, encoding, callback) { callback(null, String(value.name)); } }); encoder.on('data', function(value) { encoded.push(value.toString()); }); encoder.end({ name: 'thaw' }); var decoded = [], decoder = new stream.Transform({ readableObjectMode: true, transform: function(value, encoding, callback) { callback(null, { text: value.toString() }); } }); decoder.on('data', function(value) { decoded.push(value); }); decoder.end('node'); var sinkValues = [], sink = new stream.Writable({ objectMode: true, write: function(value, encoding, callback) { sinkValues.push(value); callback(); } }); sink.end({ final: true }); return [first, doubled.readableHighWaterMark, doubled.writableHighWaterMark, objects, encoder._writableObjectMode, encoder._readableObjectMode, encoded, decoder._writableObjectMode, decoder._readableObjectMode, decoded, sinkValues]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_object_mode_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamObjectMode = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamObjectMode").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,16,16,[{"value":4},{"value":6}],true,false,["thaw"],false,true,[{"text":"node"}],[{"final":true}]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_default_high_water_marks_apply_to_new_streams() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_default_high_water_mark");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { var before = [stream.getDefaultHighWaterMark(false), stream.getDefaultHighWaterMark(true)]; stream.setDefaultHighWaterMark(false, 7); stream.setDefaultHighWaterMark(true, 3); var readable = new stream.Readable(), writable = new stream.Writable({ objectMode: true }), zero = new stream.Duplex({ highWaterMark: 0 }); var invalid = false; try { stream.setDefaultHighWaterMark(false, -1); } catch (error) { invalid = error instanceof RangeError; } var values = [before, readable.readableHighWaterMark, writable.writableHighWaterMark, zero.readableHighWaterMark, zero.writableHighWaterMark, stream.getDefaultHighWaterMark(false), stream.getDefaultHighWaterMark(true), invalid]; stream.setDefaultHighWaterMark(false, before[0]); stream.setDefaultHighWaterMark(true, before[1]); return values; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_default_high_water_mark_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamHighWaterMark = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamHighWaterMark")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[[16384,16],7,3,0,0,7,3,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_web_adapters_transfer_readable_writable_and_duplex_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_web_adapters");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var source = new stream.Readable({ objectMode: true }), webReadable = stream.Readable.toWeb(source), reader = webReadable.getReader(); source.push({ value: 1 }); source.push(null); var first = await reader.read(), last = await reader.read(); reader.releaseLock(); var webSource = new ReadableStream({ start: function(controller) { controller.enqueue({ value: 2 }); controller.close(); } }), nodeReadable = stream.Readable.fromWeb(webSource, { objectMode: true }), fromWeb = []; for await (var value of nodeReadable) fromWeb.push(value); var nodeWrites = [], nodeWritable = new stream.Writable({ objectMode: true, write: function(value, encoding, callback) { nodeWrites.push(value); callback(); } }), webWritable = stream.Writable.toWeb(nodeWritable), writer = webWritable.getWriter(); await writer.write({ value: 3 }); await writer.close(); writer.releaseLock(); var webWrites = [], nativeWebWritable = new WritableStream({ write: function(value) { webWrites.push(value); }, close: function() { webWrites.push('closed'); } }), fromWritable = stream.Writable.fromWeb(nativeWebWritable, { objectMode: true }); await new Promise(function(resolve, reject) { fromWritable.on('error', reject); fromWritable.end({ value: 4 }, resolve); }); var pass = new stream.PassThrough({ objectMode: true }), webPair = stream.Duplex.toWeb(pass), pairReader = webPair.readable.getReader(), pairWriter = webPair.writable.getWriter(); await pairWriter.write({ value: 5 }); var pairValue = await pairReader.read(); await pairWriter.close(); await pairReader.read(); pairWriter.releaseLock(); pairReader.releaseLock(); var pairWrites = [], fromPair = stream.Duplex.fromWeb({ readable: new ReadableStream({ start: function(controller) { controller.enqueue({ value: 6 }); controller.close(); } }), writable: new WritableStream({ write: function(value) { pairWrites.push(value); } }) }, { objectMode: true }), pairOutput = []; fromPair.on('data', function(value) { pairOutput.push(value); }); fromPair.write({ value: 7 }); fromPair.end(); await new Promise(function(resolve, reject) { fromPair.on('end', resolve); fromPair.on('error', reject); }); return [first.value, last.done, fromWeb, nodeWrites, webWrites, pairValue.value, pairOutput, pairWrites]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_web_adapters_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamWebAdapters = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamWebAdapters").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[{"value":1},true,[{"value":2}],[{"value":3}],[{"value":4},"closed"],{"value":5},[{"value":6}],[{"value":7}]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_collection_helpers_transform_and_reduce_async_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_collection_helpers");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var transformed = await stream.Readable.from([1, 2, 3, 4]).map(async function(value) { await Promise.resolve(); return value * 2; }).filter(function(value) { return value > 2; }).flatMap(function(value) { return [value, value + 1]; }).drop(1).take(4).toArray(); var reduced = await stream.Readable.from([1, 2, 3]).reduce(async function(total, value) { return total + value; }, 10); var some = await stream.Readable.from([1, 2, 3]).some(function(value) { return value === 2; }), every = await stream.Readable.from([2, 4]).every(function(value) { return value % 2 === 0; }), found = await stream.Readable.from([1, 2, 3]).find(function(value) { return value > 1; }), visited = []; await stream.Readable.from(['a', 'b']).forEach(function(value, context) { visited.push([value, context.index]); }); var controller = new AbortController(); controller.abort('stop'); var aborted; try { await stream.Readable.from([1]).toArray({ signal: controller.signal }); } catch (error) { aborted = error; } var emptyError = false; try { await stream.Readable.from([]).reduce(function(a, b) { return a + b; }); } catch (error) { emptyError = error instanceof TypeError; } return [transformed, reduced, some, every, found, visited, aborted, emptyError, stream.Readable.from(['value'])._readableObjectMode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_collection_helpers_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableHelpers = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableHelpers").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[5,6,7,8],16,true,true,2,[["a",0],["b",1]],"stop",true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_from_normalizes_scalar_iterables_and_awaits_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_readable_from");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var text = await stream.Readable.from('abc').toArray(); var bytes = await stream.Readable.from(Buffer.from([1, 2])).toArray(); var promised = await stream.Readable.from([Promise.resolve(3), Promise.resolve(4)]).toArray(); var nullCode; try { await stream.Readable.from([1, null]).toArray(); } catch (error) { nullCode = error.code; } var invalidCode; try { stream.Readable.from(42); } catch (error) { invalidCode = error.code; } return [text, bytes.length, Buffer.isBuffer(bytes[0]), Array.from(bytes[0]), promised, nullCode, invalidCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_readable_from_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableFrom = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableFrom").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["abc"],1,true,[1,2],[3,4],"ERR_STREAM_NULL_VALUES","ERR_INVALID_ARG_TYPE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_iterator_can_preserve_or_destroy_its_source() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_iterator_options");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var preserved = stream.Readable.from([1, 2, 3]), iterator = preserved.iterator({ destroyOnReturn: false }), first = await iterator.next(); await iterator.return(); var afterReturn = preserved.destroyed, rest = []; for await (var value of preserved) rest.push(value); var destroyed = stream.Readable.from([4, 5]), destructive = destroyed.iterator(); await destructive.next(); await destructive.return(); var disposable = stream.Readable.from([6]); if (Symbol.asyncDispose && disposable[Symbol.asyncDispose]) await disposable[Symbol.asyncDispose](); return [first, afterReturn, rest, destroyed.destroyed, disposable.destroyed, stream.isDisturbed(preserved)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_iterator_options_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableIterator = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableIterator").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[{"value":1,"done":false},false,[2,3],true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_helpers_honor_bounded_concurrency_and_order() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_helper_concurrency");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var active = 0, maximum = 0, started = []; var mapped = await stream.Readable.from([1, 2, 3, 4]).map(async function(value, context) { active++; maximum = Math.max(maximum, active); started.push([value, context.index]); await new Promise(function(resolve) { setTimeout(resolve, (5 - value) * 2); }); active--; return value * 10; }, { concurrency: 2 }).toArray(); var filtered = await stream.Readable.from([1, 2, 3, 4]).filter(async function(value) { await Promise.resolve(); return value % 2 === 0; }, { concurrency: 3 }).toArray(); var visited = [], forEachActive = 0, forEachMaximum = 0; await stream.Readable.from([1, 2, 3]).forEach(async function(value) { forEachActive++; forEachMaximum = Math.max(forEachMaximum, forEachActive); await new Promise(function(resolve) { setTimeout(resolve, 1); }); visited.push(value); forEachActive--; }, { concurrency: 2 }); var invalid = false; try { stream.Readable.from([1]).map(function(value) { return value; }, { concurrency: 0 }); } catch (error) { invalid = error instanceof RangeError; } return [mapped, maximum, started, filtered, forEachMaximum, visited.sort(), invalid]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_helper_concurrency_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableConcurrency = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableConcurrency")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[10,20,30,40],2,[[1,0],[2,1],[3,2],[4,3]],[2,4],2,[1,2,3],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_pipelines_propagate_abort_and_original_errors() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_pipeline_abort");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); function delayed() { return new stream.Transform({ transform: function(chunk, encoding, callback) { setTimeout(function() { callback(null, chunk); }, 2); } }); } module.exports = async function () { var source = new stream.Readable(), transform = delayed(), sink = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), controller = new AbortController(), pending = promises.pipeline(source, transform, sink, { signal: controller.signal }); source.push('value'); controller.abort('stop'); var aborted; try { await pending; } catch (error) { aborted = [error.name, error.cause]; } var callbackSource = new stream.Readable(), callbackTransform = delayed(), callbackSink = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), callbackController = new AbortController(), callbackResult = new Promise(function(resolve) { stream.pipeline(callbackSource, callbackTransform, callbackSink, { signal: callbackController.signal }, function(error) { resolve([error.name, error.cause]); }); }); callbackSource.push('value'); callbackController.abort('callback-stop'); var waiting = new stream.Readable(), finishedController = new AbortController(), finished = promises.finished(waiting, { signal: finishedController.signal }); finishedController.abort('finished-stop'); var finishedError; try { await finished; } catch (error) { finishedError = [error.name, error.cause, waiting.destroyed]; } var errorSource = new stream.Readable(), errorTransform = new stream.Transform({ transform: function(chunk, encoding, callback) { setTimeout(function() { callback(new Error('transform-failed')); }, 1); } }), errorSink = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), failed = promises.pipeline(errorSource, errorTransform, errorSink); errorSource.push('value'); var original; try { await failed; } catch (error) { original = error.message; } return [aborted, source.destroyed, transform.destroyed, sink.destroyed, await callbackResult, callbackSource.destroyed, callbackTransform.destroyed, callbackSink.destroyed, finishedError, original, errorSource.destroyed, errorTransform.destroyed, errorSink.destroyed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_pipeline_abort_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamPipelineAbort = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamPipelineAbort")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["AbortError","stop"],true,true,true,["AbortError","callback-stop"],true,true,true,["AbortError","finished-stop",false],"transform-failed",true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_pipelines_reject_premature_close_once() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_pipeline_premature_close");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); module.exports = async function () { var source = new stream.Readable(), sink = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), callbackCount = 0; var callbackResult = new Promise(function(resolve) { stream.pipeline(source, sink, function(error) { callbackCount++; queueMicrotask(function() { resolve([error.code, error.message, callbackCount]); }); }); }); source.destroy(); var first = await callbackResult; var sourceTwo = new stream.Readable(), sinkTwo = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }), pending = promises.pipeline(sourceTwo, sinkTwo); sinkTwo.destroy(); var second; try { await pending; } catch (error) { second = [error.code, error.message]; } return [first, source.destroyed, sink.destroyed, second, sourceTwo.destroyed, sinkTwo.destroyed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_pipeline_premature_close_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exercisePipelinePrematureClose = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exercisePipelinePrematureClose")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["ERR_STREAM_PREMATURE_CLOSE","Premature close",1],true,true,["ERR_STREAM_PREMATURE_CLOSE","Premature close"],true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_completion_helpers_validate_required_arguments() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_completion_validation");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = function () { function code(action) { try { action(); } catch (error) { return error.code; } } var source = function() { return new stream.Readable(); }, sink = function() { return new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); }; return [code(function() { stream.pipeline(source(), sink()); }), code(function() { stream.pipeline(source(), function() {}); }), code(function() { stream.pipeline([], function() {}); }), code(function() { stream.finished(source()); }), code(function() { stream.finished({}, function() {}); }), typeof stream.pipeline([source(), sink()], function() {})]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_completion_validation_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseCompletionValidation = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseCompletionValidation")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["ERR_INVALID_ARG_TYPE","ERR_MISSING_ARGS","ERR_MISSING_ARGS","ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_TYPE","object"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn finished_defers_callbacks_and_honors_cleanup_and_error_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_finished_options");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var order = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); } }); stream.finished(writable, { cleanup: true }, function() { order.push('finished'); }); writable.on('finish', function() { order.push('finish'); }); writable.end(); order.push('after'); await Promise.resolve(); await Promise.resolve(); var observed = [], ignored = new stream.Readable(); ignored.on('error', function(error) { observed.push('own:' + error.message); }); var ignoredDone = new Promise(function(resolve) { stream.finished(ignored, { error: false, cleanup: true }, function(error) { observed.push(error ? error.code : 'ok'); resolve(); }); }); ignored.destroy(new Error('ignored')); await ignoredDone; var controller = new AbortController(); controller.abort('stop'); var abortedOrder = [], waiting = new stream.Readable(); var abortedDone = new Promise(function(resolve) { stream.finished(waiting, { signal: controller.signal, cleanup: true }, function(error) { abortedOrder.push([error.name, error.code, error.cause]); resolve(); }); }); abortedOrder.push('after'); await abortedDone; var cleaned = new stream.Readable(), before, after; var cleanedDone = new Promise(function(resolve) { stream.finished(cleaned, { cleanup: true }, function() { after = ['end', 'finish', 'error', 'close'].map(function(name) { return cleaned.listenerCount(name); }); resolve(); }); }); before = ['end', 'finish', 'error', 'close'].map(function(name) { return cleaned.listenerCount(name); }); cleaned.push(null); cleaned.resume(); await cleanedDone; return [order, observed, abortedOrder, before, after]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_finished_options_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFinishedOptions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFinishedOptions").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["after","finish","finished"],["own:ignored","ok"],["after",["AbortError","ABORT_ERR","stop"]],[1,1,1,1],[0,0,0,0]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn promise_pipeline_can_leave_destination_open() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_pipeline_end_false");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); module.exports = async function () { var values = [], destination = new stream.Writable({ objectMode: true, write: function(value, encoding, callback) { values.push(value); callback(); } }); var result = await promises.pipeline(stream.Readable.from([1, 2]), destination, { end: false }); var open = [result, values, destination.writableEnded, destination.writableFinished, destination.destroyed]; destination.write(3); await new Promise(function(resolve, reject) { destination.on('error', reject); destination.end(resolve); }); return [open, values, destination.writableFinished]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_pipeline_end_false_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exercisePipelineEndFalse = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exercisePipelineEndFalse").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[[null,[1,2,3],false,false,false],[1,2,3],true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_pipelines_accept_iterables_and_function_stages() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_function_pipeline");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); module.exports = async function () { var callbackValue = await new Promise(function(resolve, reject) { stream.pipeline([1, 2, 3], async function*(source) { for await (var value of source) yield value * 2; }, async function(source) { var total = 0; for await (var value of source) total += value; return total; }, function(error, value) { if (error) reject(error); else resolve(value); }); }); var promiseValue = await promises.pipeline(async function* source(options) { yield options.signal ? 'signal' : 'missing'; yield 'source'; }, async function*(source) { for await (var value of source) yield value.toUpperCase(); }, async function(source) { var values = []; for await (var value of source) values.push(value); return values.join(':'); }); var transformed = new stream.Transform({ objectMode: true, transform: function(value, encoding, callback) { callback(null, value + 1); } }); var mixedValue = await promises.pipeline([4, 5], transformed, async function(source) { return (await source.toArray()).join(','); }); var controller = new AbortController(), aborted = promises.pipeline(async function*() { await new Promise(function(resolve) { setTimeout(resolve, 5); }); yield 1; }, async function(source) { for await (var value of source) return value; }, { signal: controller.signal }); controller.abort('pipeline-stop'); var abortResult; try { await aborted; } catch (error) { abortResult = [error.name, error.cause]; } return [callbackValue, promiseValue, mixedValue, abortResult]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_function_pipeline_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFunctionPipeline = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFunctionPipeline").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[12,"SIGNAL:SOURCE","5,6",["AbortError","pipeline-stop"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn functional_pipeline_errors_abort_sources_and_destroy_stages() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_function_pipeline_cleanup");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); module.exports = async function () { var signalAborted = false, sourceClosed = false, middle = new stream.PassThrough({ objectMode: true }), expected = new Error('destination-failed'), observed; try { await promises.pipeline(async function* source(options) { options.signal.addEventListener('abort', function() { signalAborted = true; }, { once: true }); try { yield 1; await new Promise(function(resolve) { options.signal.addEventListener('abort', resolve, { once: true }); }); yield 2; } finally { sourceClosed = true; } }, middle, async function(destination) { for await (var value of destination) throw expected; }); } catch (error) { observed = error; } await Promise.resolve(); await Promise.resolve(); var callbackMiddle = new stream.PassThrough({ objectMode: true }), callbackError = await new Promise(function(resolve) { stream.pipeline([1], callbackMiddle, async function() { throw new Error('callback-failed'); }, function(error) { resolve(error); }); }); return [observed === expected, observed.message, signalAborted, sourceClosed, middle.destroyed, middle.errored === expected, callbackError.message, callbackMiddle.destroyed, callbackMiddle.errored === callbackError]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_function_pipeline_cleanup_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFunctionPipelineCleanup = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFunctionPipelineCleanup")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"destination-failed",true,true,true,true,"callback-failed",true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn finished_waits_for_both_duplex_sides_and_reports_early_close() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_finished_duplex");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), promises = require('node:stream/promises'); module.exports = async function () { var duplex = new stream.Duplex(), resolved = false, pending = promises.finished(duplex).then(function() { resolved = true; }); duplex.end(); await Promise.resolve(); var afterFinish = resolved; duplex.push(null); await pending; var early = new stream.Readable(), earlyPending = promises.finished(early); early.destroy(); var earlyCode; try { await earlyPending; } catch (error) { earlyCode = error.code; } var writableOnly = new stream.Duplex(), writablePending = promises.finished(writableOnly, { readable: false }); writableOnly.end(); await writablePending; var callbackDuplex = new stream.Duplex(), callbackCount = 0, cleanup = stream.finished(callbackDuplex, { cleanup: true }, function() { callbackCount++; }); cleanup(); callbackDuplex.end(); callbackDuplex.push(null); await Promise.resolve(); return [afterFinish, resolved, earlyCode, writableOnly.writableFinished, writableOnly.readableEnded, callbackCount]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_finished_duplex_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFinishedDuplex = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFinishedDuplex").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,true,"ERR_STREAM_PREMATURE_CLOSE",true,false,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_writables_serialize_async_work_and_signal_backpressure() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_backpressure");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var events = [], sink = new stream.Writable({ highWaterMark: 3, write: function(chunk, encoding, callback) { var value = chunk.toString(); events.push('start:' + value); setTimeout(function() { events.push('end:' + value); callback(); }, 1); } }); sink.on('drain', function() { events.push('drain:' + sink.writableNeedDrain); }); sink.on('finish', function() { events.push('finish'); }); var first = sink.write('a', function() { events.push('callback:a'); }), second = sink.write('bb', function() { events.push('callback:bb'); }), completion = new Promise(function(resolve, reject) { sink.on('error', reject); sink.on('finish', resolve); }); sink.end('c', function() { events.push('end-callback'); }); await completion; var transformed = [], transform = new stream.Transform({ highWaterMark: 2, transform: function(chunk, encoding, callback) { setTimeout(function() { callback(null, chunk.toString().toUpperCase()); }, 1); } }); transform.on('data', function(chunk) { transformed.push(chunk.toString()); }); var transformFirst = transform.write('x'), transformSecond = transform.write('y'), transformCompletion = new Promise(function(resolve, reject) { transform.on('error', reject); transform.on('finish', resolve); }); transform.end('z'); await transformCompletion; return [first, second, sink.writableLength, sink.writableFinished, events, transformFirst, transformSecond, transformed.join(''), transform.readableEnded]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_backpressure_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamBackpressure").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,0,true,["start:a","end:a","start:bb","callback:a","end:bb","drain:false","start:c","callback:bb","end:c","end-callback","finish"],true,false,"XYZ",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_end_callbacks_settle_once_before_finish() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_repeated_end");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var events = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { events.push('write:' + chunk.toString()); callback(); }, final: function(callback) { events.push('final'); callback(); } }); writable.on('finish', function() { events.push('finish'); }); writable.end('x', function(error) { events.push('first:' + (error && error.code || 'ok')); }); events.push('after-first'); writable.end(function(error) { events.push('second:' + (error && error.code || 'ok')); }); events.push('after-second'); var immediate = writable.writableFinished; await new Promise(function(resolve) { queueMicrotask(resolve); }); writable.end(function(error) { events.push('late:' + (error && error.code || 'ok')); }); events.push('after-late'); await new Promise(function(resolve) { queueMicrotask(resolve); }); return [immediate, writable.writableFinished, events]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_repeated_end_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseRepeatedEnd = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseRepeatedEnd").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,true,["write:x","final","after-first","after-second","first:ok","second:ok","finish","after-late","late:ok"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn synchronous_writable_callbacks_are_deferred_and_guarded() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_sync_write_callbacks");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var events = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { events.push('write'); callback(); } }); var result = writable.write('x', function(error) { events.push('callback:' + (error && error.code || 'ok')); }); events.push('after:' + result + ':' + writable.writableLength); var before = events.slice(); await Promise.resolve(); var vectorEvents = [], vector = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); }, writev: function(chunks, callback) { vectorEvents.push('writev:' + chunks.length); callback(); } }); vector.cork(); vector.write('a', function() { vectorEvents.push('a'); }); vector.write('b', function() { vectorEvents.push('b'); }); vector.uncork(); vectorEvents.push('after'); var vectorBefore = vectorEvents.slice(); await Promise.resolve(); var repeatedEvents = [], repeated = new stream.Writable({ write: function(chunk, encoding, callback) { repeatedEvents.push('write'); callback(); callback(); } }); repeated.on('error', function(error) { repeatedEvents.push('error:' + error.code); }); repeated.on('close', function() { repeatedEvents.push('close'); }); var repeatedResult = repeated.write('z', function(error) { repeatedEvents.push('callback:' + (error && error.code || 'ok')); }); repeatedEvents.push('after:' + repeatedResult + ':' + repeated.destroyed); await Promise.resolve(); return [before, events, vectorBefore, vectorEvents, repeatedEvents]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_sync_write_callbacks_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseSyncWriteCallbacks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseSyncWriteCallbacks").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["write","after:true:0"],["write","after:true:0","callback:ok"],["writev:2","after"],["writev:2","after","a","b"],["write","after:false:true","callback:ok","error:ERR_MULTIPLE_CALLBACK","close"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_final_exceptions_and_repeated_callbacks_destroy_stream() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_final_failures");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); function exercise(finalizer) { return new Promise(function(resolve) { var events = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { callback(); }, final: finalizer }); writable.on('error', function(error) { events.push('error:' + error.code + ':' + error.message); }); writable.on('close', function() { events.push('close'); queueMicrotask(function() { resolve([events, writable.destroyed, writable.closed, writable.writableFinished]); }); }); writable.end('x', function(error) { events.push('end:' + (error && (error.code || error.message))); }); events.push('after:' + writable.destroyed); }); } module.exports = async function () { var thrown = await exercise(function() { throw new Error('final-threw'); }); var repeated = await exercise(function(callback) { callback(); callback(); }); return [thrown, repeated]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_final_failures_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFinalFailures = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFinalFailures").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[["after:true","error:undefined:final-threw","close","end:final-threw"],true,true,false],[["after:true","error:ERR_MULTIPLE_CALLBACK:Callback called multiple times","close","end:ERR_MULTIPLE_CALLBACK"],true,true,false]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_streams_cork_and_batch_writev_chunks() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_writev");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var batches = [], singles = [], callbacks = [], writable = new stream.Writable({ write: function(chunk, encoding, callback) { singles.push(chunk.toString()); callback(); }, writev: function(chunks, callback) { batches.push(chunks.map(function(entry) { return entry.chunk.toString(); })); setTimeout(callback, 1); } }); writable.cork(); writable.cork(); writable.write('a', function() { callbacks.push('a'); }); writable.write('b', function() { callbacks.push('b'); }); var nested = [writable.writableCorked, batches.length, writable.writableLength]; writable.uncork(); var stillCorked = [writable.writableCorked, batches.length]; writable.uncork(); writable.cork(); writable.write('c', function() { callbacks.push('c'); }); var completion = new Promise(function(resolve, reject) { writable.on('error', reject); writable.on('finish', resolve); }); writable.end('d', function() { callbacks.push('end'); }); await completion; return [nested, stillCorked, writable.writableCorked, batches, singles, callbacks, writable.writableLength, writable.writableFinished]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_writev_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamWritev = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamWritev").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[2,0,2],[1,0],0,[["a","b"],["c","d"]],[],["a","b","c","end"],0,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn writable_destroy_rejects_pending_work_exactly_once() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_destroy_pending");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var events = [], release, writable = new stream.Writable({ write: function(chunk, encoding, callback) { events.push('write:' + chunk.toString()); release = callback; } }); writable.on('error', function(error) { events.push('error:' + error.message); }); writable.on('close', function() { events.push('close'); }); writable.on('finish', function() { events.push('finish'); }); writable.write('a', function(error) { events.push('callback:a:' + (error ? error.message : 'ok')); }); writable.write('b', function(error) { events.push('callback:b:' + error.message); }); writable.end('c', function(error) { events.push('end:' + error.message); }); writable.destroy(new Error('boom')); writable.destroy(new Error('again')); release(); await new Promise(function(resolve) { queueMicrotask(resolve); }); var silentEvents = [], silentRelease, silent = new stream.Writable({ write: function(chunk, encoding, callback) { silentRelease = callback; } }); silent.write('x', function(error) { silentEvents.push(error ? error.code : 'ok'); }); silent.on('error', function() { silentEvents.push('unexpected-error'); }); silent.destroy(); silentRelease(); await new Promise(function(resolve) { queueMicrotask(resolve); }); return [events, writable.writableLength, writable.destroyed, writable.writableFinished, silentEvents, silent.destroyed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_destroy_pending_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamDestroyPending = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseStreamDestroyPending")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["write:a","callback:a:ok","callback:b:boom","end:boom","error:boom","close"],0,true,false,["ok"],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_streams_track_buffers_and_pause_for_pipe_backpressure() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_backpressure");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var readable = new stream.Readable({ highWaterMark: 3 }), first = readable.push('ab'), second = readable.push('cd'), before = readable.readableLength, partial = readable.read(3).toString(), after = readable.readableLength, rest = readable.read().toString(), objects = new stream.Readable({ objectMode: true, highWaterMark: 2 }), object = { value: 1 }, objectFirst = objects.push(object), objectSecond = objects.push({ value: 2 }), sameObject = objects.read() === object; var values = [], source = new stream.Readable({ highWaterMark: 2 }), sink = new stream.Writable({ highWaterMark: 2, write: function(chunk, encoding, callback) { values.push(chunk.toString()); setTimeout(callback, 1); } }), finished = new Promise(function(resolve, reject) { sink.on('error', reject); sink.on('finish', resolve); }); source.pipe(sink); source.push('a'); source.push('b'); var paused = source.isPaused(); source.push('c'); var queued = source.readableLength; source.push(null); await finished; return [first, second, before, partial, after, rest, objects.readableLength, objectFirst, objectSecond, sameObject, paused, queued, source.isPaused(), source.readableLength, values.join(''), sink.writableFinished]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_backpressure_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableBackpressure")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,4,"abc",1,"d",1,true,false,true,true,1,false,0,"abc",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readable_async_iterators_handle_buffering_waits_and_cleanup() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readable_async_iterator");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var buffered = new stream.Readable(); buffered.push('a'); buffered.push('b'); buffered.push(null); var values = []; for await (var value of buffered) values.push(value.toString()); var future = new stream.Readable({ objectMode: true }), iterator = future[Symbol.asyncIterator](), pending = iterator.next(), object = { answer: 42 }; setTimeout(function() { future.push(object); }, 1); var arrived = await pending; future.push(null); var ended = await iterator.next(); var broken = new stream.Readable(), brokenIterator = broken[Symbol.asyncIterator](), rejected = brokenIterator.next(); setTimeout(function() { broken.destroy(new Error('broken')); }, 1); var message; try { await rejected; } catch (error) { message = error.message; } var early = new stream.Readable(); early.push('first'); early.push('second'); var earlyValues = []; for await (var item of early) { earlyValues.push(item.toString()); break; } return [values, arrived.value === object, arrived.done, ended.done, message, earlyValues, early.destroyed, early.readableLength]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_readable_async_iterator_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadableAsyncIterator = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseReadableAsyncIterator")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["a","b"],true,false,true,"broken",["first"],true,6]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn diagnostics_channels_publish_bind_stores_and_trace() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_diagnostics_channel");
    fs::write(
            dir.join("index.js"),
            "var diagnostics = require('node:diagnostics_channel');\n\
             module.exports = async function () {\n\
             \x20 var events = []; var work = diagnostics.channel('work'); var same = work === diagnostics.channel('work');\n\
             \x20 function subscriber(data, name) { events.push(name + ':' + data.value); } diagnostics.subscribe('work', subscriber); diagnostics.subscribe('work', subscriber);\n\
             \x20 var store = { run: function(value, callback) { events.push('store:' + value); return callback(); } }; work.bindStore(store, function(data) { return data.value * 2; });\n\
             \x20 var storeResult = work.runStores({ value: 2 }, function(left, right) { return left + right; }, null, 3, 4); work.publish({ value: 5 }); var removed = diagnostics.unsubscribe('work', subscriber); work.publish({ value: 6 }); work.unbindStore(store);\n\
             \x20 var trace = diagnostics.tracingChannel('operation'); ['start', 'end', 'asyncStart', 'asyncEnd', 'error'].forEach(function(name) { trace[name].subscribe(function(context) { events.push(name + ':' + (context.result || context.error && context.error.message || '')); }); });\n\
             \x20 var sync = trace.traceSync(function(value) { return value + 1; }, {}, null, 4); var promised = await trace.tracePromise(async function(value) { return value * 2; }, {}, null, 3); var failed; try { trace.traceSync(function() { throw new Error('bad'); }, {}); } catch (error) { failed = error.message; }\n\
             \x20 return [same, storeResult, removed, diagnostics.hasSubscribers('work'), sync, promised, failed, events];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_diagnostics_channel_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseDiagnostics = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseDiagnostics").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,7,true,false,5,6,"bad",["store:4","store:10","work:5","store:12","start:","end:5","start:","asyncStart:6","asyncEnd:6","end:6","start:","error:bad","end:bad"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn async_hooks_preserve_storage_and_resource_scope() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_async_hooks");
    fs::write(
            dir.join("index.js"),
            "var hooks = require('node:async_hooks');\n\
             module.exports = async function () {\n\
             \x20 var storage = new hooks.AsyncLocalStorage({ defaultValue: 'default', name: 'request' }); var events = [storage.getStore()]; var bound; var snapshot;\n\
             \x20 var result = await storage.run('outer', async function(value) { events.push(storage.getStore() + ':' + value); bound = hooks.AsyncLocalStorage.bind(function(suffix) { return storage.getStore() + suffix; }); snapshot = hooks.AsyncLocalStorage.snapshot(); await Promise.resolve(); events.push(storage.getStore()); var nested = storage.run('inner', function() { return storage.getStore(); }); events.push(nested + ':' + storage.getStore()); var exited = storage.exit(function() { return storage.getStore(); }); events.push(String(exited) + ':' + storage.getStore()); return 'done'; }, 4);\n\
             \x20 storage.enterWith('changed'); var rebound = bound('!'); var snapped = snapshot(function() { return storage.getStore(); });\n\
             \x20 var resource = new hooks.AsyncResource('work'); var outside = hooks.executionAsyncId(); var inside = resource.runInAsyncScope(function(left, right) { return [hooks.executionAsyncId(), hooks.triggerAsyncId(), hooks.executionAsyncResource() === resource, left + right]; }, null, 2, 3); var reboundResource = resource.bind(function() { return hooks.executionAsyncId(); })(); resource.emitDestroy();\n\
             \x20 storage.disable(); return [events, result, rebound, snapped, storage.getStore() === undefined, outside, inside, reboundResource, resource.asyncId(), resource.triggerAsyncId(), resource._destroyed, hooks.createHook({}).enable().disable().callbacks];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_async_hooks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseAsyncHooks = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseAsyncHooks").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["default","outer:4","outer","inner:outer","undefined:outer"],"done","outer!","outer",true,1,[2,1,true,5],2,2,1,true,{}]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn tty_builtin_reports_capabilities_and_emits_ansi_sequences() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_tty");
    fs::write(
            dir.join("index.js"),
            "var tty = require('node:tty');\n\
             module.exports = async function () {\n\
             \x20 var input = new tty.ReadStream(0); var same = input.setRawMode(true) === input; input.setRawMode(false);\n\
             \x20 var output = new tty.WriteStream(1); var callbacks = []; output.write('text'); output.cursorTo(2, 3, function() { callbacks.push('cursor'); }); output.moveCursor(-1, 2, function() { callbacks.push('move'); }); output.clearLine(0); output.clearScreenDown(); await Promise.resolve();\n\
             \x20 return [tty.isatty(1), input.isTTY, input.isRaw, same, output.getWindowSize(), output.getColorDepth({ FORCE_COLOR: '3' }), output.getColorDepth({ TERM: 'xterm-256color' }), output.hasColors(256, { TERM: 'xterm-256color' }), output.hasColors(16, {}), output._output, callbacks.sort().join(',')];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_tty_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseTty = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseTty").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
            result,
            "[false,true,false,true,[80,24],24,8,true,false,\"text\\u001b[4;3H\\u001b[1D\\u001b[2B\\u001b[2K\\u001b[0J\",\"cursor,move\"]"
        );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn module_create_require_loads_relative_and_builtin_dependencies() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_module_create_require");
    fs::write(
            dir.join("index.js"),
            "import { createRequire, isBuiltin, builtinModules, registerHooks } from 'node:module';\n\
             const localRequire = createRequire('pkg/index.js');\n\
             export default function () {\n\
             \x20 const dependency = localRequire('./dependency'); const path = localRequire('node:path'); const hooks = registerHooks({}); hooks.deregister();\n\
             \x20 return [dependency.value, path.basename('/tmp/file.txt'), localRequire.resolve('./dependency'), Object.keys(localRequire.cache).length >= 3, isBuiltin('node:path'), isBuiltin('missing'), builtinModules.includes('stream'), hooks.active];\n\
             }",
        )
        .unwrap();
    fs::write(dir.join("dependency.js"), "module.exports = { value: 42 };").unwrap();
    let empty_node_modules = temp_registry("builtin_module_create_require_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseModule = module.exports.default;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseModule").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[42,"file.txt","pkg/dependency.js",true,true,false,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn console_builtin_shares_global_console_and_constructor() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_console");
    fs::write(
            dir.join("index.js"),
            "var consoleModule = require('node:console');\n\
             module.exports = function () { var output = []; var instance = new consoleModule.Console({ write: function(value) { output.push(value); } }); instance.log('%s:%d', 'value', 2); instance.warn({ ok: true }); return [consoleModule === globalThis.console, consoleModule.console === globalThis.console, output.join('')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_console_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseConsole = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseConsole").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,true,"value:2\n{\"ok\":true}\n"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn crypto_builtin_shares_native_hash_and_random_implementations() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_crypto");
    fs::write(
            dir.join("index.js"),
            "var crypto = require('node:crypto');\n\
             module.exports = async function () { var callbackLength = await new Promise(function(resolve, reject) { crypto.randomBytes(7, function(error, value) { if (error) reject(error); else resolve(value.length); }); }); var uuid = crypto.randomUUID(); return [crypto === globalThis.__thaw_crypto_module, crypto.createHash('sha256').update('abc').digest('hex'), crypto.createHmac('sha512', 'key').update('value').digest().length, callbackLength, /^[0-9a-f-]{36}$/.test(uuid), crypto.webcrypto === globalThis.crypto, crypto.timingSafeEqual(Buffer.from('x'), Buffer.from('x'))]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_crypto_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseCrypto = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseCrypto").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",64,7,true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn perf_hooks_builtin_shares_the_performance_timeline() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_perf_hooks");
    fs::write(
            dir.join("index.js"),
            "var hooks = require('node:perf_hooks');\n\
             module.exports = function () { hooks.performance.clearMarks(); hooks.performance.clearMeasures(); hooks.performance.mark('start', { startTime: 2 }); hooks.performance.mark('end', { startTime: 7 }); var measure = hooks.performance.measure('elapsed', 'start', 'end'); var histogram = hooks.monitorEventLoopDelay({ resolution: 10 }); var enabled = histogram.enable(); var disabled = histogram.disable(); var utilization = hooks.performance.eventLoopUtilization(); return [hooks.performance === globalThis.performance, measure.duration, hooks.PerformanceObserver === globalThis.PerformanceObserver, enabled, disabled, histogram.percentile(99), typeof histogram.percentileBigInt(99), utilization.idle, utilization.active >= 0, utilization.utilization >= 0, hooks.performance.nodeTiming.name, hooks.constants.NODE_PERFORMANCE_GC_MAJOR]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_perf_hooks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exercisePerformanceHooks = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exercisePerformanceHooks").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,5,true,true,true,0,"bigint",0,true,true,"node",4]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn v8_builtin_serializes_graphs_and_exposes_runtime_statistics() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_v8");
    fs::write(
            dir.join("index.js"),
            "var v8 = require('node:v8');\n\
             module.exports = function () { var source = { bigint: 42n, bytes: Buffer.from('thaw'), map: new Map([['answer', 42]]), set: new Set(['x']), missing: undefined }; source.self = source; var encoded = v8.serialize(source); var copy = v8.deserialize(encoded); var heap = v8.getHeapStatistics(); var code = v8.getHeapCodeStatistics(); return [Buffer.isBuffer(encoded), copy !== source, copy.self === copy, copy.bigint === 42n, copy.bytes.toString(), copy.map.get('answer'), copy.set.has('x'), Object.prototype.hasOwnProperty.call(copy, 'missing'), copy.missing === undefined, heap.number_of_native_contexts, heap.heap_size_limit > 0, v8.getHeapSpaceStatistics().length, code.code_and_metadata_size, v8.cachedDataVersionTag()]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_v8_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseV8 = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseV8").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,true,"thaw",42,true,true,true,1,true,0,0,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn zlib_builtin_compresses_sync_and_callback_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_zlib");
    fs::write(dir.join("index.js"), "var zlib = require('node:zlib'), stream = require('node:stream'); module.exports = async function () { var source = Buffer.from('thaw compression '.repeat(8)); var gzip = zlib.gzipSync(source); var raw = zlib.deflateRawSync(source); var brotli = zlib.brotliCompressSync(source); var callback = await new Promise(function(resolve, reject) { zlib.gunzip(gzip, function(error, value) { error ? reject(error) : resolve(value); }); }); var brotliCallback = await new Promise(function(resolve, reject) { zlib.brotliDecompress(brotli, function(error, value) { error ? reject(error) : resolve(value); }); }); var compressor = zlib.createGzip(), compressed = []; compressor.on('data', function(value) { compressed.push(value); }); var streamDone = new Promise(function(resolve, reject) { compressor.on('error', reject); compressor.on('end', resolve); }); compressor.write(source.subarray(0, 7)); compressor.end(source.subarray(7)); await streamDone; var streamed = Buffer.concat(compressed), decompressor = zlib.createGunzip(), restored = []; decompressor.on('data', function(value) { restored.push(value); }); var restoreDone = new Promise(function(resolve, reject) { decompressor.on('error', reject); decompressor.on('end', resolve); }); stream.Readable.from([streamed.subarray(0, 5), streamed.subarray(5)]).pipe(decompressor); await restoreDone; var brotliStream = zlib.createBrotliCompress(), brotliChunks = []; brotliStream.on('data', function(value) { brotliChunks.push(value); }); var brotliDone = new Promise(function(resolve, reject) { brotliStream.on('error', reject); brotliStream.on('end', resolve); }); brotliStream.end(source); await brotliDone; var flushed = await new Promise(function(resolve) { compressor.flush(resolve); }); return [gzip[0], gzip[1], zlib.gunzipSync(gzip).toString() === source.toString(), zlib.inflateRawSync(raw).toString() === source.toString(), callback.toString() === source.toString(), zlib.constants.Z_OK, zlib.constants.Z_SYNC_FLUSH, compressor instanceof zlib.Gzip, compressor instanceof stream.Transform, compressor.bytesWritten, Buffer.concat(restored).toString() === source.toString(), flushed, zlib.brotliDecompressSync(brotli).toString() === source.toString(), brotliCallback.toString() === source.toString(), brotliStream instanceof zlib.BrotliCompress, zlib.brotliDecompressSync(Buffer.concat(brotliChunks)).toString() === source.toString(), zlib.constants.BROTLI_OPERATION_FINISH]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_zlib_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseZlib = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseZlib").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[31,139,true,true,true,0,2,true,true,136,true,null,true,true,true,true,2]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_threads_builtin_exchanges_cloned_messages() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_threads");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var channel = new workers.MessageChannel(); var original = { value: 7 }; channel.port1.postMessage(original); original.value = 9; var received = workers.receiveMessageOnPort(channel.port2); var pending = new Promise(function(resolve) { channel.port2.once('message', resolve); }); channel.port1.postMessage(new Map([['answer', 42]])); var asynchronous = await pending; workers.setEnvironmentData('config', { enabled: true }); var environment = workers.getEnvironmentData('config'); environment.enabled = false; var freshEnvironment = workers.getEnvironmentData('config'); var protectedBuffer = new ArrayBuffer(4); workers.markAsUntransferable(protectedBuffer); var transferRejected = false; try { channel.port1.postMessage('x', [protectedBuffer]); } catch (error) { transferRejected = error.name === 'DataCloneError'; } var protectedObject = { value: 1 }; workers.markAsUncloneable(protectedObject); var cloneRejected = false; try { structuredClone({ nested: protectedObject }); } catch (error) { cloneRejected = error.name === 'DataCloneError'; } var extra = new workers.MessageChannel(); channel.port1.postMessage({ port: extra.port1 }, [extra.port1]); var transferred = workers.receiveMessageOnPort(channel.port2); transferred.message.port.postMessage('through'); var through = workers.receiveMessageOnPort(extra.port2).message; var broadcastSender = new workers.BroadcastChannel('sync-receive'), broadcastReceiver = new workers.BroadcastChannel('sync-receive'); broadcastSender.postMessage({ value: 5 }); var broadcast = workers.receiveMessageOnPort(broadcastReceiver); broadcastSender.close(); broadcastReceiver.close(); extra.port1.close(); extra.port2.close(); channel.port1.unref(); var refed = channel.port1.hasRef(); channel.port1.ref(); channel.port1.close(); channel.port2.close(); return [workers.isMainThread, workers.threadId, workers.parentPort, received.message.value, asynchronous.get('answer'), freshEnvironment.enabled, refed, channel.port1.hasRef(), workers.SHARE_ENV === Symbol.for('nodejs.worker_threads.SHARE_ENV'), workers.receiveMessageOnPort(channel.port2) === undefined, workers.isMarkedAsUntransferable(protectedBuffer), transferRejected, cloneRejected, broadcast.message.value, through]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_worker_threads_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkers = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseWorkers").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,0,null,7,42,true,false,true,true,true,true,true,true,5,"through"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_threads_eval_worker_isolates_state_and_exchanges_messages() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_eval");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var events = []; delete process.env.THAW_WORKER_TEST; var source = \"var wt = require('node:worker_threads'); globalThis.workerOnly = 99; wt.parentPort.on('message', function(value) { var args = process.argv.slice(-2).join(','); var environment = process.env.THAW_WORKER_TEST; process.env.THAW_WORKER_TEST = 'mutated'; wt.parentPort.postMessage({ answer: wt.workerData.base + value, main: wt.isMainThread, threadId: wt.threadId, threadName: wt.threadName, isolated: globalThis.workerOnly, args: args, execArgv: process.execArgv.join(','), environment: environment, native: typeof __thaw_worker_spawn }); wt.parentPort.close(); });\"; var worker = new workers.Worker(source, { eval: true, name: 'alpha', workerData: { base: 40 }, argv: ['one', 2], execArgv: ['--trace-warnings'], env: { THAW_WORKER_TEST: 'child' }, resourceLimits: { maxOldGenerationSizeMb: 64 } }); var listenerOrder = []; function regular() { listenerOrder.push('regular'); } worker.addListener('probe', regular).prependOnceListener('probe', function() { listenerOrder.push('first'); }); var listenerCount = worker.listenerCount('probe'); var hasProbe = worker.eventNames().indexOf('probe') >= 0; worker.emit('probe'); worker.emit('probe'); worker.removeListener('probe', regular).setMaxListeners(20); var completed = new Promise(function(resolve) { worker.on('online', function() { events.push('online'); }); worker.on('message', function(value) { events.push('message:' + value.answer + ':' + value.main + ':' + (value.threadId > 0) + ':' + value.threadName + ':' + value.isolated + ':' + value.args + ':' + value.execArgv + ':' + value.environment + ':' + value.native); }); worker.on('error', function(error) { events.push('error:' + error.stack); resolve(); }); worker.on('exit', function(code) { events.push('exit:' + code); resolve(); }); }); worker.postMessage(2); await completed; delete process.env.THAW_WORKER_SHARED; var shared = new workers.Worker(\"var wt = require('node:worker_threads'); process.env.THAW_WORKER_SHARED = 'shared'; wt.parentPort.close();\", { eval: true, env: workers.SHARE_ENV }); await new Promise(function(resolve, reject) { shared.on('error', reject); shared.on('exit', resolve); }); var sharedValue = process.env.THAW_WORKER_SHARED; delete process.env.THAW_WORKER_SHARED; return [events, worker.threadName, listenerOrder, listenerCount, hasProbe, worker.listenerCount('probe'), worker.getMaxListeners(), typeof globalThis.workerOnly, process.env.THAW_WORKER_TEST, sharedValue, worker.resourceLimits.maxOldGenerationSizeMb, worker.threadId > 0, worker.ref() === worker, worker.unref() === worker, await worker.terminate()]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_worker_eval_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorker = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseWorker").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["online","message:42:false:true:alpha:99:one,2:--trace-warnings:child:function","exit:0"],"alpha",["first","regular","regular"],2,true,0,20,"undefined",null,"shared",64,true,true,true,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_threads_share_environment_across_native_workers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_native_share_env");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } function result(worker) { return new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); }); } module.exports = async function () { delete process.env.THAW_PARENT_SHARED; delete process.env.THAW_WORKER_SHARED; delete process.env.THAW_SECOND_SHARED; var first = new workers.Worker(\"var wt = require('node:worker_threads'); wt.parentPort.on('message', function() { var parent = process.env.THAW_PARENT_SHARED; process.env.THAW_WORKER_SHARED = 'worker'; wt.parentPort.postMessage(parent); wt.parentPort.close(); });\", { eval: true, env: workers.SHARE_ENV }); await online(first); process.env.THAW_PARENT_SHARED = 'parent'; var firstResult = result(first), firstExit = new Promise(function(resolve) { first.on('exit', resolve); }); first.postMessage('go'); var parentSeen = await firstResult; await firstExit; var second = new workers.Worker(\"var wt = require('node:worker_threads'); wt.parentPort.postMessage(process.env.THAW_WORKER_SHARED); process.env.THAW_SECOND_SHARED = 'second'; wt.parentPort.close();\", { eval: true, env: workers.SHARE_ENV }); var secondResult = result(second), secondExit = new Promise(function(resolve) { second.on('exit', resolve); }); var workerSeen = await secondResult; await secondExit; var secondSeen = process.env.THAW_SECOND_SHARED, native = [typeof first._nativeHandle, typeof second._nativeHandle]; delete process.env.THAW_PARENT_SHARED; delete process.env.THAW_WORKER_SHARED; delete process.env.THAW_SECOND_SHARED; return [parentSeen, workerSeen, secondSeen, native]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_share_env_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeShareEnv = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseNativeShareEnv").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["parent","worker","second",["number","number"]]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_moves_message_ports_to_vm_contexts() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_move_port_context");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); var vm = require('node:vm'); module.exports = function () { var channel = new workers.MessageChannel(); var context = vm.createContext({}); var moved = workers.moveMessagePortToContext(channel.port1, context); moved.postMessage('moved'); var received = workers.receiveMessageOnPort(channel.port2); var invalidContext = false; try { workers.moveMessagePortToContext(channel.port2, {}); } catch (error) { invalidContext = error instanceof TypeError; } return [received.message, channel.port1.__thawClosed, moved !== channel.port1, invalidContext]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_move_port_context_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseMovePortToContext = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseMovePortToContext").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, r#"["moved",true,true,true]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_transfers_worker_data_message_ports() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_data_transfer");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var channel = new workers.MessageChannel(); var source = \"var wt = require('node:worker_threads'); wt.workerData.port.postMessage('from-worker'); wt.workerData.port.on('message', function(value) { wt.workerData.port.postMessage(value.toUpperCase()); wt.workerData.port.close(); wt.parentPort.close(); });\"; var worker = new workers.Worker(source, { eval: true, workerData: { port: channel.port1 }, transferList: [channel.port1] }); var first = new Promise(function(resolve) { channel.port2.once('message', resolve); }); var detached = channel.port1.__thawClosed && channel.port1.__thawPeer === null; var exit = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); var initial = await first, second = new Promise(function(resolve) { channel.port2.once('message', resolve); }); channel.port2.postMessage('through'); return [initial, await second, await exit, detached, typeof worker._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_data_transfer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerDataTransfer = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseWorkerDataTransfer").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, r#"["from-worker","THROUGH",0,true,"number"]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_transfer_message_ports_from_native_workers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_native_outbound_port");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'), channel = new wt.MessageChannel(); channel.port2.on('message', function(value) { channel.port2.postMessage(value.toUpperCase()); channel.port2.close(); wt.parentPort.close(); }); wt.parentPort.postMessage({ port: channel.port1 }, [channel.port1]);\"; var worker = new Worker(source, { eval: true }); var exit = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); var transferred = await new Promise(function(resolve) { worker.once('message', function(value) { resolve(value.port); }); }); var response = new Promise(function(resolve) { transferred.once('message', resolve); }); transferred.postMessage('worker-port'); var value = await response, code = await exit; transferred.close(); return [value, code, typeof worker._nativeHandle, transferred.__thawHostPortId.startsWith('w:')]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_outbound_port_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeOutboundPort = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseNativeOutboundPort").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, r#"["WORKER-PORT",0,"number",true]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_redirects_stdin_stdout_and_stderr() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_stdio");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'); process.stdin.setEncoding('utf8'); process.stdin.on('data', function(value) { console.log('stdout', value); console.error('stderr', value); wt.parentPort.postMessage(value.toUpperCase()); wt.parentPort.close(); });\"; var worker = new Worker(source, { eval: true, stdin: true, stdout: true, stderr: true }); worker.stdout.setEncoding('utf8'); worker.stderr.setEncoding('utf8'); var output = '', errors = '', message; worker.stdout.on('data', function(value) { output += value; }); worker.stderr.on('data', function(value) { errors += value; }); worker.on('message', function(value) { message = value; }); var exited = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); worker.stdin.end('hello'); var code = await exited; return [message, output, errors, code, worker.stdin.writableFinished, worker.stdout.readableEnded, worker.stderr.readableEnded, typeof worker._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_stdio_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerStdio = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseWorkerStdio").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["HELLO","stdout hello\n","stderr hello\n",0,true,true,true,"number"]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_terminate_resolves_with_one_shared_exit_code() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_terminate");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var worker = new Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); var exits = []; worker.on('exit', function(code) { exits.push(code); }); var first = worker.terminate(); var second = worker.terminate(); var code = await first; return [first === second, code, await second, await worker.terminate(), exits, Symbol.asyncDispose ? worker[Symbol.asyncDispose] === worker.terminate : true]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_terminate_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerTerminate = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseWorkerTerminate").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, "[true,1,1,1,[1],true]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_diagnostics_follow_running_state() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_diagnostics");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var worker = new Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); await new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); var cpu = await worker.cpuUsage(); var heap = await worker.getHeapStatistics(); var snapshot = await worker.getHeapSnapshot(); snapshot.setEncoding('utf8'); var text = ''; await new Promise(function(resolve, reject) { snapshot.on('data', function(chunk) { text += chunk; }); snapshot.on('end', resolve); snapshot.on('error', reject); }); var profile = await worker.startCpuProfile(); var stopped = await profile.stop(); await worker.terminate(); var stoppedError; try { await worker.cpuUsage(); } catch (error) { stoppedError = error.code; } return [cpu, heap.number_of_native_contexts, JSON.parse(text).snapshot.node_count, stopped.nodes.length, stopped.samples.length, stoppedError]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_diagnostics_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerDiagnostics = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseWorkerDiagnostics").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[{"user":0,"system":0},1,0,0,0,"ERR_WORKER_NOT_RUNNING"]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_routes_messages_by_thread_id() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_direct_messages");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } module.exports = async function () { var fromWorker = []; function mainListener(value, source) { fromWorker.push([value.kind, source]); } process.on('workerMessage', mainListener); var source = \"var wt = require('node:worker_threads'); process.on('workerMessage', function(value, source) { wt.parentPort.postMessage(['direct:' + value.kind, source]); wt.parentPort.close(); }); wt.postMessageToThread(0, { kind: 'hello' }).then(function() { wt.parentPort.postMessage(['ready', wt.threadId]); });\"; var worker = new workers.Worker(source, { eval: true }); var messages = []; var direct; var completed = new Promise(function(resolve, reject) { worker.on('message', function(value) { messages.push(value); if (value[0] === 'ready') workers.postMessageToThread(worker.threadId, { kind: 'ping' }).catch(reject); else direct = value; }); worker.on('error', reject); worker.on('exit', resolve); }); var exitCode = await completed; process.off('workerMessage', mainListener); var same, missing; try { await workers.postMessageToThread(0, 'x'); } catch (error) { same = error.code; } try { await workers.postMessageToThread(999999, 'x'); } catch (error) { missing = error.code; } var silent = new workers.Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); await online(silent); var noListener; try { await workers.postMessageToThread(silent.threadId, 'x'); } catch (error) { noListener = error.code; } await silent.terminate(); var throwing = new workers.Worker(\"var wt = require('node:worker_threads'); process.on('workerMessage', function() { throw new Error('listener failed'); });\", { eval: true }); await online(throwing); var listenerError; try { await workers.postMessageToThread(throwing.threadId, 'x'); } catch (error) { listenerError = [error.code, error.cause.message]; } var native = [typeof worker._nativeHandle, typeof silent._nativeHandle, typeof throwing._nativeHandle]; await throwing.terminate(); return [fromWorker, direct, exitCode, same, missing, noListener, listenerError, native]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_direct_messages_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDirectWorkerMessages = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseDirectWorkerMessages").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[["hello",1]],["direct:ping",0],0,"ERR_WORKER_MESSAGING_SAME_THREAD","ERR_WORKER_MESSAGING_FAILED","ERR_WORKER_MESSAGING_FAILED",["ERR_WORKER_MESSAGING_ERRORED","listener failed"],["number","number","number"]]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_route_direct_messages_between_native_workers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_direct_between_workers");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } module.exports = async function () { var receiver = new workers.Worker(\"var wt = require('node:worker_threads'); process.on('workerMessage', function(value, source) { wt.parentPort.postMessage([value.answer, source]); wt.parentPort.close(); });\", { eval: true }); await online(receiver); var sender = new workers.Worker(\"var wt = require('node:worker_threads'); wt.postMessageToThread(wt.workerData.target, { answer: 42 }).then(function() { wt.parentPort.postMessage('sent'); wt.parentPort.close(); });\", { eval: true, workerData: { target: receiver.threadId } }); var received = new Promise(function(resolve, reject) { receiver.on('message', resolve); receiver.on('error', reject); }); var sent = new Promise(function(resolve, reject) { sender.on('message', resolve); sender.on('error', reject); }); var receiverExit = new Promise(function(resolve) { receiver.on('exit', resolve); }), senderExit = new Promise(function(resolve) { sender.on('exit', resolve); }); var values = await Promise.all([received, sent, receiverExit, senderExit]); return [values, typeof receiver._nativeHandle, typeof sender._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_direct_between_workers_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeWorkerRouting = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseNativeWorkerRouting").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, r#"[[[42,2],"sent",0,0],"number","number"]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_data_url_worker_decodes_javascript() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_data_url");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var parentThread = globalThis.__thaw_os_thread_token; var source = \"var wt = require('node:worker_threads'); wt.parentPort.postMessage({ value: wt.workerData.value, main: wt.isMainThread, native: typeof __thaw_worker_spawn, thread: globalThis.__thaw_os_thread_token }); wt.parentPort.close();\"; var worker = new workers.Worker('data:text/javascript,' + encodeURIComponent(source), { workerData: { value: 17 } }); var events = []; await new Promise(function(resolve, reject) { worker.on('online', function() { events.push('online'); }); worker.on('message', function(value) { events.push('message:' + value.value + ':' + value.main + ':' + value.native + ':' + (value.thread !== parentThread)); }); worker.on('error', reject); worker.on('exit', function(code) { events.push('exit:' + code); resolve(); }); }); var rejected = false; try { new workers.Worker('data:text/plain,not-javascript'); } catch (error) { rejected = error instanceof TypeError; } return [events, rejected]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_worker_data_url_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDataWorker = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseDataWorker").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["online","message:17:false:function:true","exit:0"],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_threads_load_runtime_computed_absolute_paths() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_runtime_path");
    let worker_dir = dir.join("workers/nested");
    fs::create_dir_all(&worker_dir).unwrap();
    fs::create_dir_all(dir.join("internal/features")).unwrap();
    fs::write(
            dir.join("package.json"),
            r##"{"imports":{"#worker-tool":{"node":"./internal/tool.cjs","default":"./internal/wrong.cjs"},"#features/*":{"require":"./internal/features/*.js"},"#escape":"../outside.js"}}"##,
        )
        .unwrap();
    fs::write(dir.join("internal/tool.cjs"), "module.exports = 11;").unwrap();
    fs::write(
        dir.join("internal/features/math.js"),
        "module.exports = 12;",
    )
    .unwrap();
    let worker_path = worker_dir.join("dynamic-worker.js");
    fs::write(
        worker_dir.join("worker-value.js"),
        "module.exports = require('./worker-package') + 1;",
    )
    .unwrap();
    fs::create_dir_all(worker_dir.join("worker-package/lib")).unwrap();
    fs::write(
        worker_dir.join("worker-package/package.json"),
        r#"{"main":"lib/value"}"#,
    )
    .unwrap();
    fs::write(
        worker_dir.join("worker-package/lib/value.cjs"),
        "module.exports = require('../worker-data.json').value;",
    )
    .unwrap();
    fs::write(
        worker_dir.join("worker-package/worker-data.json"),
        r#"{"value":41}"#,
    )
    .unwrap();
    fs::create_dir_all(dir.join("node_modules/runtime-worker-dependency")).unwrap();
    fs::write(
            dir.join("node_modules/runtime-worker-dependency/package.json"),
            r#"{"main":"wrong.cjs","exports":{".":{"node":"./entry.cjs","default":"./wrong.cjs"},"./feature":{"require":"./feature.js"},"./features/*":{"require":"./dist/*.cjs"},"./escape":"../outside.js"}}"#,
        )
        .unwrap();
    fs::write(
        dir.join("node_modules/runtime-worker-dependency/entry.cjs"),
        "module.exports = 8;",
    )
    .unwrap();
    fs::write(
        dir.join("node_modules/runtime-worker-dependency/feature.js"),
        "module.exports = 9;",
    )
    .unwrap();
    fs::create_dir_all(dir.join("node_modules/runtime-worker-dependency/dist")).unwrap();
    fs::write(
        dir.join("node_modules/runtime-worker-dependency/dist/math.cjs"),
        "module.exports = 10;",
    )
    .unwrap();
    fs::write(
            &worker_path,
            "var wt = require('node:worker_threads'); var hiddenCode; try { require('runtime-worker-dependency/hidden'); } catch (error) { hiddenCode = error.code; } var missingImportCode; try { require('#missing'); } catch (error) { missingImportCode = error.code; } var exportEscapeCode; try { require('runtime-worker-dependency/escape'); } catch (error) { exportEscapeCode = error.code; } var importEscapeCode; try { require('#escape'); } catch (error) { importEscapeCode = error.code; } wt.parentPort.postMessage([wt.workerData, __filename, __dirname, typeof __thaw_worker_spawn, require('./worker-value'), require('runtime-worker-dependency'), require('runtime-worker-dependency/feature'), require('runtime-worker-dependency/features/math'), hiddenCode, require('#worker-tool'), require('#features/math'), missingImportCode, exportEscapeCode, importEscapeCode]); wt.parentPort.close();",
        )
        .unwrap();
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function (runtimePath) { var worker = new Worker(runtimePath, { workerData: 23 }); var exit = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); var message = await new Promise(function(resolve) { worker.on('message', resolve); }); return [message, await exit, typeof worker._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_runtime_path_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseRuntimeWorkerPath = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseRuntimeWorkerPath").unwrap();
    let path = worker_path.to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&vec![&path]).unwrap()).unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    let expected = serde_json::json!([
        [
            23,
            path,
            worker_dir.to_string_lossy(),
            "function",
            42,
            8,
            9,
            10,
            "ERR_PACKAGE_PATH_NOT_EXPORTED",
            11,
            12,
            "ERR_PACKAGE_IMPORT_NOT_DEFINED",
            "ERR_INVALID_PACKAGE_TARGET",
            "ERR_INVALID_PACKAGE_TARGET"
        ],
        0,
        "number"
    ])
    .to_string();
    assert_eq!(result, expected);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_runtime_round_trips_parent_messages() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_native_round_trip");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'); wt.parentPort.on('message', function(value) { wt.parentPort.postMessage([value * wt.workerData, globalThis.__thaw_os_thread_token]); wt.parentPort.close(); });\"; var parentThread = globalThis.__thaw_os_thread_token; var worker = new Worker(source, { eval: true, workerData: 7 }); var exit = new Promise(function(resolve) { worker.on('exit', resolve); }); var result = await new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); worker.postMessage(6); }); var code = await exit; return [result[0], result[1] !== parentThread, code, typeof worker._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_round_trip_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeWorkerRoundTrip = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseNativeWorkerRoundTrip").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, r#"[42,true,0,"number"]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_runtime_transfers_structured_array_buffers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_native_structured_transfer");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var first = new ArrayBuffer(3), second = new ArrayBuffer(2); new Uint8Array(first).set([1, 2, 3]); new Uint8Array(second).set([4, 5]); var data = { buffer: first, map: new Map([['answer', 42]]) }; data.self = data; var source = \"var wt = require('node:worker_threads'); wt.parentPort.on('message', function(value) { var reply = { first: Array.from(new Uint8Array(wt.workerData.buffer)), second: Array.from(new Uint8Array(value.buffer)), answer: wt.workerData.map.get('answer'), workerCycle: wt.workerData.self === wt.workerData, messageCycle: value.self === value }; reply.self = reply; wt.parentPort.postMessage(reply); wt.parentPort.close(); });\"; var worker = new Worker(source, { eval: true, workerData: data, transferList: [first] }); var message = { buffer: second }; message.self = message; var exit = new Promise(function(resolve) { worker.on('exit', resolve); }); var reply = new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); }); worker.postMessage(message, [second]); var value = await reply, code = await exit; return [first.byteLength, second.byteLength, value.first, value.second, value.answer, value.workerCycle, value.messageCycle, value.self === value, code, typeof worker._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_structured_transfer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeWorkerTransfer = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseNativeWorkerTransfer").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[0,0,[1,2,3],[4,5],42,true,true,true,0,"number"]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn bundler_embeds_static_file_url_worker_sources() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_file_url");
    fs::write(
        dir.join("double.js"),
        "module.exports = function(value) { return value * 2; };",
    )
    .unwrap();
    fs::write(
            dir.join("worker.js"),
            "import { parentPort, workerData } from 'node:worker_threads'; var double = require('./double'); parentPort.postMessage(double(workerData)); parentPort.close();",
        )
        .unwrap();
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; var path = require('node:path'); const workerFile = './' + 'worker.js'; const joinedWorkerFile = path.join(__dirname, 'worker.js'); function run(worker, events) { return new Promise(function(resolve, reject) { worker.on('message', function(value) { events.push(value); }); worker.on('error', reject); worker.on('exit', function(code) { events.push(code); resolve(); }); }); } module.exports = async function () { var events = []; await run(new Worker(new URL('./worker.js', import.meta.url), { workerData: 21 }), events); await run(new Worker('./worker.js', { workerData: 11 }), events); await run(new Worker(`./${'worker'}.js`, { workerData: 5 }), events); await run(new Worker(workerFile, { workerData: 3 }), events); await run(new Worker(joinedWorkerFile, { workerData: 2 }), events); return events; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_worker_file_url_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 6);
    fs::remove_dir_all(&dir).unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFileWorker = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseFileWorker").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "[42,0,22,0,10,0,6,0,4,0]");
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_url_rewrite_ignores_unrelated_worker_bindings() {
    let dir = temp_registry("unrelated_worker_url");
    let source = "class Worker {}\nnew Worker(new URL('./missing.js', import.meta.url));";
    let rewritten = rewrite_static_worker_urls(source, &dir.join("index.js"), "pkg", &dir)
        .expect("an unrelated Worker must not attempt to read its URL");
    assert_eq!(rewritten, source);
    let eval_source = "var Worker = require('node:worker_threads').Worker; new Worker('./missing.js', { eval: true });";
    let rewritten = rewrite_static_worker_urls(eval_source, &dir.join("index.js"), "pkg", &dir)
        .expect("eval Worker source must not be treated as a file");
    assert_eq!(rewritten, eval_source);
    let shadowed_source = "var Worker = require('node:worker_threads').Worker; const workerFile = './missing.js'; function start(workerFile) { return new Worker(workerFile); }";
    let rewritten = rewrite_static_worker_urls(shadowed_source, &dir.join("index.js"), "pkg", &dir)
        .expect("a shadowed constant Worker path must remain dynamic");
    assert_eq!(rewritten, shadowed_source);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn worker_threads_broadcast_channel_clones_between_matching_names() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_broadcast_channel");
    fs::write(
            dir.join("index.js"),
            "var BroadcastChannel = require('node:worker_threads').BroadcastChannel; module.exports = async function () { var sender = new BroadcastChannel('room'); var first = new BroadcastChannel('room'); var second = new BroadcastChannel('room'); var isolated = new BroadcastChannel('elsewhere'); var source = { nested: { value: 4 } }; var firstMessage = new Promise(function(resolve) { first.onmessage = function(event) { event.data.nested.value = 8; resolve(event.data.nested.value); }; }); var secondMessage = new Promise(function(resolve) { second.addEventListener('message', function(event) { resolve(event.data.nested.value); }, { once: true }); }); var isolatedCalled = false; isolated.onmessage = function() { isolatedCalled = true; }; sender.postMessage(source); source.nested.value = 9; var values = await Promise.all([firstMessage, secondMessage]); first.close(); var closedError = false; try { first.postMessage('x'); } catch (error) { closedError = error.name === 'InvalidStateError'; } sender.close(); second.close(); isolated.close(); return [values[0], values[1], isolatedCalled, closedError, sender.name, sender.ref() === sender, sender.unref() === sender]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_broadcast_channel_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseBroadcast = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseBroadcast").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[8,4,false,true,"room",true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_sync_and_promise_apis_operate_on_the_host_filesystem() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_promises");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); var promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/work', original = work + '/value.txt', renamed = work + '/renamed.txt'; fs.mkdirSync(work, { recursive: true }); await new Promise(function(resolve, reject) { fs.writeFile(original, 'one', function(error) { error ? reject(error) : resolve(); }); }); await new Promise(function(resolve, reject) { fs.appendFile(original, Buffer.from('two'), function(error) { error ? reject(error) : resolve(); }); }); var text = await promises.readFile(original, 'utf8'), callbackText = await new Promise(function(resolve, reject) { fs.readFile(original, 'utf8', function(error, value) { error ? reject(error) : resolve(value); }); }), stat = await new Promise(function(resolve, reject) { fs.stat(original, function(error, value) { error ? reject(error) : resolve(value); }); }), entries = fs.readdirSync(work, { withFileTypes: true }); await promises.rename(original, renamed); var renamedExists = fs.existsSync(renamed), missingCode; try { await promises.readFile(work + '/missing'); } catch (error) { missingCode = [error.code, error.path, error.syscall]; } await promises.unlink(renamed); await promises.rm(work, { recursive: true }); return [text, callbackText, stat.isFile(), stat.isDirectory(), stat.size, entries[0].name, entries[0].isFile(), renamedExists, fs.existsSync(renamed), fs.existsSync(work), missingCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_promises_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsPromises = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseFsPromises").unwrap();
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(
            r#"["onetwo","onetwo",true,false,6,"value.txt",true,true,false,false,["ENOENT","{}/work/missing","read"]]"#,
            dir.to_string_lossy()
        )
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_copy_realpath_and_mkdtemp_work_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_paths");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source.txt', copied = root + '/copied.txt', prefix = root + '/temporary-'; fs.writeFileSync(source, 'copy-value'); fs.copyFileSync(source, copied); var copiedValue = fs.readFileSync(copied, 'utf8'), resolved = await promises.realpath(copied); var temporary = await new Promise(function(resolve, reject) { fs.mkdtemp(prefix, function(error, path) { error ? reject(error) : resolve(path); }); }); var callbackPath = await new Promise(function(resolve, reject) { fs.realpath.native(source, function(error, path) { error ? reject(error) : resolve(path); }); }); await promises.unlink(source); await promises.unlink(copied); await promises.rmdir(temporary); return [copiedValue, resolved.slice(-10), callbackPath.slice(-10), temporary.indexOf(prefix) === 0, fs.existsSync(temporary), typeof fs.realpathSync.native]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_paths_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsPaths = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsPaths").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["copy-value","copied.txt","source.txt",true,false,"function"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_disposable_temporary_directories_remove_nested_contents_once() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_disposable_mkdtemp");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var sync = fs.mkdtempDisposableSync(root + '/sync-'); fs.mkdirSync(sync.path + '/nested'); fs.writeFileSync(sync.path + '/nested/value.txt', 'sync'); var syncPath = sync.path, syncSymbol = Symbol.dispose ? sync[Symbol.dispose] === sync.remove : true; sync.remove(); sync.remove(); var disposable = await promises.mkdtempDisposable(root + '/async-', { encoding: 'buffer' }), asyncPath = disposable.path.toString(), asyncSymbol = Symbol.asyncDispose ? disposable[Symbol.asyncDispose] === disposable.remove : true; fs.mkdirSync(asyncPath + '/nested'); fs.writeFileSync(asyncPath + '/nested/value.txt', 'async'); var first = disposable.remove(), sameRemoval = first === disposable.remove(); await first; await disposable.remove(); return [syncPath.indexOf(root + '/sync-') === 0, fs.existsSync(syncPath), syncSymbol, Buffer.isBuffer(disposable.path), asyncPath.indexOf(root + '/async-') === 0, fs.existsSync(asyncPath), asyncSymbol, sameRemoval]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_disposable_mkdtemp_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDisposableMkdtemp = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseDisposableMkdtemp").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,false,true,true,true,false,true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_open_as_blob_exposes_blob_reads_and_detects_file_changes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_open_as_blob");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (root) { var path = root + '/blob.txt'; fs.writeFileSync(path, 'abcdef'); var blob = await fs.openAsBlob(path, { type: 'Text/Plain' }), slice = blob.slice(1, 4, 'APPLICATION/X'), bytes = Array.from(await blob.bytes()), buffer = Array.from(new Uint8Array(await blob.arrayBuffer())), reader = blob.stream().getReader(), streamed = await reader.read(); reader.releaseLock(); var missingCode; try { await fs.openAsBlob(root + '/missing'); } catch (error) { missingCode = error.code; } fs.writeFileSync(path, 'changed-value'); var changedName; try { await blob.text(); } catch (error) { changedName = error.name; } var sliceChangedName; try { await slice.text(); } catch (error) { sliceChangedName = error.name; } return [blob instanceof Blob, blob.size, blob.type, Buffer.from(bytes).toString(), buffer.length, Buffer.from(streamed.value).toString(), streamed.done, slice.size, slice.type, missingCode, changedName, sliceChangedName]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_open_as_blob_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseOpenAsBlob = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseOpenAsBlob").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,6,"text/plain","abcdef",6,"abcdef",false,3,"application/x","ENOENT","NotReadableError","NotReadableError"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_cp_recursively_copies_directories_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_cp");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source', sync = root + '/sync', asyncPath = root + '/async', callbackPath = root + '/callback'; fs.mkdirSync(source + '/nested', { recursive: true }); fs.writeFileSync(source + '/nested/value.txt', 'copied'); fs.cpSync(source, sync, { recursive: true }); await promises.cp(source, asyncPath, { recursive: true }); await new Promise(function(resolve, reject) { fs.cp(source, callbackPath, { recursive: true }, function(error) { error ? reject(error) : resolve(); }); }); fs.writeFileSync(sync + '/nested/value.txt', 'kept'); fs.cpSync(source, sync, { recursive: true, force: false }); var existingCode; try { fs.cpSync(source, sync, { recursive: true, force: false, errorOnExist: true }); } catch (error) { existingCode = error.code; } return [fs.readFileSync(sync + '/nested/value.txt', 'utf8'), fs.readFileSync(asyncPath + '/nested/value.txt', 'utf8'), fs.readFileSync(callbackPath + '/nested/value.txt', 'utf8'), existingCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_cp_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsCp = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsCp").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["kept","copied","copied","EEXIST"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_cp_honors_filters_links_and_timestamp_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_cp_options");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source', sync = root + '/sync', promised = root + '/promised', callback = root + '/callback'; fs.mkdirSync(source + '/nested', { recursive: true }); fs.writeFileSync(source + '/keep.txt', 'keep'); fs.writeFileSync(source + '/skip.txt', 'skip'); fs.writeFileSync(source + '/nested/value.txt', 'nested'); fs.symlinkSync('keep.txt', source + '/link'); fs.utimesSync(source + '/keep.txt', new Date(1000000), new Date(2000000)); var syncSeen = []; fs.cpSync(source, sync, { recursive: true, verbatimSymlinks: true, preserveTimestamps: true, filter: function(src) { syncSeen.push(src); return !src.endsWith('/skip.txt'); } }); var asyncSeen = []; await promises.cp(source, promised, { recursive: true, dereference: true, filter: async function(src) { asyncSeen.push(src); await Promise.resolve(); return !src.endsWith('/nested'); } }); var callbackSeen = []; await new Promise(function(resolve, reject) { fs.cp(source, callback, { recursive: true, filter: function(src) { callbackSeen.push(src); return Promise.resolve(!src.endsWith('/skip.txt')); } }, function(error) { error ? reject(error) : resolve(); }); }); var directoryCode; try { fs.cpSync(source, root + '/not-recursive'); } catch (error) { directoryCode = error.code; } var asyncFilterError; try { fs.cpSync(source + '/keep.txt', root + '/invalid-filter', { filter: function() { return Promise.resolve(true); } }); } catch (error) { asyncFilterError = error.name; } return [fs.existsSync(sync + '/keep.txt'), fs.existsSync(sync + '/skip.txt'), fs.readlinkSync(sync + '/link'), Math.abs(fs.statSync(sync + '/keep.txt').mtimeMs - 2000000) < 2, syncSeen.length, fs.readFileSync(promised + '/link', 'utf8'), fs.existsSync(promised + '/nested'), asyncSeen.length, fs.existsSync(callback + '/keep.txt'), fs.existsSync(callback + '/skip.txt'), callbackSeen.length, directoryCode, asyncFilterError]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_cp_options_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsCpOptions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsCpOptions").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,"keep.txt",true,6,"keep",false,5,true,false,6,"ERR_FS_EISDIR","TypeError"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_promise_file_handles_manage_repeated_operations_and_streams() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_file_handle");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'), stream = require('node:stream'); module.exports = async function (path) { var handle = await promises.open(path, 'w+'), originalFd = handle.fd; await handle.writeFile('abcdef'); await handle.appendFile('ghi'); var before = await handle.stat(); await handle.truncate(5); var value = await handle.readFile('utf8'), reader = handle.createReadStream({ highWaterMark: 2 }), chunks = []; reader.on('data', function(chunk) { chunks.push(chunk.toString()); }); await new Promise(function(resolve, reject) { reader.on('error', reject); reader.on('end', resolve); }); await handle.close(); var closedCode; try { await handle.readFile(); } catch (error) { closedCode = error.code; } return [originalFd >= 10, before.size, value, chunks, reader instanceof fs.ReadStream, reader instanceof stream.Readable, handle.fd, handle.closed, closedCode, Symbol.asyncDispose ? typeof handle[Symbol.asyncDispose] : 'unavailable']; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_file_handle_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsFileHandle = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("handle.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsFileHandle").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,9,"abcde",["ab","cd","e"],true,true,-1,true,"EBADF","function"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_file_handles_support_positioned_buffer_reads_and_writes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_file_handle_positions");
    fs::write(
            dir.join("index.js"),
            "var promises = require('node:fs/promises'); module.exports = async function (path) { var handle = await promises.open(path, 'w+'); await handle.writeFile('abcdef'); var first = Buffer.alloc(4, 46), read = await handle.read(first, 1, 2, 2); var written = await handle.write(Buffer.from('XYZ'), 1, 2, 4); var textWrite = await handle.write('!', 1, 'utf8'); var sequential = Buffer.alloc(3), sequentialRead = await handle.read(sequential, 0, 3, null); var value = await handle.readFile('utf8'); await handle.close(); return [read.bytesRead, read.buffer.toString(), written.bytesWritten, written.buffer.toString(), textWrite.bytesWritten, textWrite.buffer, sequentialRead.bytesRead, sequential.toString(), value]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_file_handle_positions_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsFileHandlePositions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("positioned.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsFileHandlePositions")
            .unwrap()
            .as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[2,".cd.",2,"XYZ",1,"!",3,"a!c","a!cdYZ"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_directory_handles_support_reads_callbacks_and_async_iteration() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_directory_handles");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/entries'; fs.mkdirSync(work); fs.writeFileSync(work + '/a.txt', 'a'); fs.mkdirSync(work + '/nested'); var sync = fs.opendirSync(work), first = sync.readSync(), second = sync.readSync(); sync.closeSync(); var closedCode; try { sync.readSync(); } catch (error) { closedCode = error.code; } var callbackName = await new Promise(function(resolve, reject) { fs.opendir(work, function(error, opened) { if (error) return reject(error); opened.read(function(readError, entry) { if (readError) return reject(readError); opened.close(function(closeError) { closeError ? reject(closeError) : resolve(entry.name); }); }); }); }); var opened = await promises.opendir(work), iterated = []; for await (var entry of opened) iterated.push([entry.name, entry.isFile(), entry.isDirectory()]); iterated.sort(function(left, right) { return left[0].localeCompare(right[0]); }); return [[first.name, second.name].sort(), sync instanceof fs.Dir, sync.closed, closedCode, typeof callbackName, iterated, opened.closed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_directory_handles_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsDirectoryHandles = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsDirectoryHandles").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["a.txt","nested"],true,true,"ERR_DIR_CLOSED","string",[["a.txt",true,false],["nested",false,true]],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_links_and_permissions_use_host_filesystem_semantics() {
    use std::ffi::{CStr, CString};
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_registry("builtin_fs_links");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source.txt', hard = root + '/hard.txt', symbolic = root + '/symbolic.txt'; fs.writeFileSync(source, 'linked'); fs.linkSync(source, hard); await promises.symlink(source, symbolic); var target = await new Promise(function(resolve, reject) { fs.readlink(symbolic, function(error, value) { error ? reject(error) : resolve(value); }); }); await promises.chmod(source, 384); return [fs.readFileSync(hard, 'utf8'), fs.readFileSync(symbolic, 'utf8'), target, fs.readlinkSync(symbolic, 'buffer').toString(), fs.existsSync(hard), fs.existsSync(symbolic)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_links_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsLinks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsLinks").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let source_path = dir.join("source.txt").to_string_lossy().into_owned();
    assert_eq!(
        result,
        format!(r#"["linked","linked","{0}","{0}",true,true]"#, source_path)
    );
    assert_eq!(
        fs::metadata(dir.join("source.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_stats_and_utimes_use_host_timestamps_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_timestamps");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { fs.writeFileSync(path, 'time'); fs.utimesSync(path, 100, 200); var sync = fs.statSync(path); await new Promise(function(resolve, reject) { fs.utimes(path, new Date(300000), new Date(400000), function(error) { error ? reject(error) : resolve(); }); }); var callback = await promises.stat(path); await promises.utimes(path, 500, 600); var handle = await promises.open(path, 'r+'); await handle.utimes(700, 800); var finalStats = await handle.stat(); await handle.close(); return [Math.round(sync.atimeMs), Math.round(sync.mtimeMs), sync.atime.getTime(), sync.mtime.getTime(), sync.ctimeMs > 0, (sync.mode & 32768) !== 0, Math.round(callback.atimeMs), Math.round(callback.mtimeMs), Math.round(finalStats.atimeMs), Math.round(finalStats.mtimeMs), finalStats.atime.getTime(), finalStats.mtime.getTime()]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_timestamps_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsTimestamps = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("timestamps.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsTimestamps").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[100000,200000,100000,200000,true,true,300000,400000,700000,800000,700000,800000]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_ownership_apis_use_host_uid_and_gid() {
    use std::ffi::{CStr, CString};
    use std::os::unix::fs::MetadataExt;
    let dir = temp_registry("builtin_fs_ownership");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root, uid, gid) { var source = root + '/owned.txt', link = root + '/owned-link'; fs.writeFileSync(source, 'owned'); fs.symlinkSync(source, link); fs.chownSync(source, uid, gid); await promises.chown(source, uid, gid); await new Promise(function(resolve, reject) { fs.lchown(link, uid, gid, function(error) { error ? reject(error) : resolve(); }); }); var handle = await promises.open(source, 'r+'); await handle.chown(uid, gid); var stats = await handle.stat(); await handle.close(); return [stats.uid, stats.gid, fs.statSync(source).uid, fs.statSync(source).gid]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_ownership_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsOwnership = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let owner = fs::metadata(&dir).unwrap();
    let uid = owner.uid();
    let gid = owner.gid();
    let arguments = CString::new(
        serde_json::to_string(&[
            serde_json::json!(dir.to_string_lossy()),
            serde_json::json!(uid),
            serde_json::json!(gid),
        ])
        .unwrap(),
    )
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsOwnership").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, format!("[{uid},{gid},{uid},{gid}]"));
    let link_metadata = fs::symlink_metadata(dir.join("owned-link")).unwrap();
    assert_eq!((link_metadata.uid(), link_metadata.gid()), (uid, gid));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_watch_file_reports_host_changes_and_stops_cleanly() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_watch_file");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (path) { fs.writeFileSync(path, 'one'); var ignoredCalls = 0, ignored = function() { ignoredCalls++; }, watcher; var changed = new Promise(function(resolve) { watcher = fs.watchFile(path, { interval: 1, persistent: false }, function(current, previous) { fs.unwatchFile(path); resolve([previous.size, current.size, current.isFile(), watcher.hasRef(), ignoredCalls]); }); fs.watchFile(path, { interval: 1 }, ignored); fs.unwatchFile(path, ignored); }); setTimeout(function() { fs.writeFileSync(path, 'changed-value'); }, 3); var result = await changed; return [result, watcher._timer, watcher._listeners.length]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_watch_file_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsWatchFile = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("watched.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsWatchFile").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[[3,13,true,false,0],null,0]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_watch_reports_directory_rename_and_change_events() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_watch");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (root) { var work = root + '/watched'; fs.mkdirSync(work); var events = [], closeCount = 0, watcher; var completed = new Promise(function(resolve) { watcher = fs.watch(work, { interval: 1, persistent: false }, function(type, name) { events.push([type, name]); if (events.length === 2) { watcher.close(); resolve(); } }); watcher.once('close', function() { closeCount++; }); }); setTimeout(function() { fs.writeFileSync(work + '/value.txt', 'a'); }, 3); setTimeout(function() { fs.writeFileSync(work + '/value.txt', 'expanded'); }, 12); await completed; var controller = new AbortController(), aborted = fs.watch(work, { interval: 1, signal: controller.signal, encoding: 'buffer' }); controller.abort(); return [watcher instanceof fs.FSWatcher, events, watcher.closed, watcher._timer, watcher.hasRef(), closeCount, aborted.closed, aborted._timer]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_watch_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsWatch = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsWatch").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,[["rename","value.txt"],["change","value.txt"]],true,null,false,1,true,null]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_glob_supports_patterns_exclusions_and_async_iteration() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_glob");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { fs.mkdirSync(root + '/src/nested', { recursive: true }); fs.writeFileSync(root + '/root.txt', 'root'); fs.writeFileSync(root + '/src/main.js', 'js'); fs.writeFileSync(root + '/src/nested/value.txt', 'txt'); fs.writeFileSync(root + '/src/nested/skip.txt', 'skip'); var sync = fs.globSync('**/*.txt', { cwd: root, exclude: '**/skip.txt' }).sort(); var callback = await new Promise(function(resolve, reject) { fs.glob(['src/*.js', 'root.?xt'], { cwd: root }, function(error, values) { error ? reject(error) : resolve(values.sort()); }); }); var asyncValues = []; for await (var value of promises.glob('src/**', { cwd: root })) asyncValues.push(value); asyncValues.sort(); var dirents = fs.globSync('src/*', { cwd: root, withFileTypes: true }); return [sync, callback, asyncValues, dirents.map(function(entry) { return [entry.name, entry.isFile(), entry.isDirectory()]; }).sort()]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_glob_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsGlob = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsGlob").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["root.txt","src/nested/value.txt"],["root.txt","src/main.js"],["src/main.js","src/nested","src/nested/skip.txt","src/nested/value.txt"],[["main.js",true,false],["nested",false,true]]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_access_checks_host_permissions_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_access");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var path = root + '/access.txt', missing = root + '/missing.txt'; fs.writeFileSync(path, 'access'); fs.accessSync(path, fs.constants.R_OK | fs.constants.W_OK); await promises.access(root, fs.constants.X_OK); var callback = await new Promise(function(resolve, reject) { fs.access(path, fs.constants.F_OK, function(error) { error ? reject(error) : resolve(true); }); }); var syncError, callbackError, promiseError; try { fs.accessSync(missing); } catch (error) { syncError = [error.code, error.path, error.syscall]; } await new Promise(function(resolve) { fs.access(missing, function(error) { callbackError = [error.code, error.path, error.syscall]; resolve(); }); }); try { await promises.access(missing, fs.constants.R_OK); } catch (error) { promiseError = [error.code, error.path, error.syscall]; } return [callback, syncError, callbackError, promiseError]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_access_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsAccess = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsAccess").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let missing = dir.join("missing.txt").to_string_lossy().into_owned();
    assert_eq!(
        result,
        format!(
            r#"[true,["ENOENT","{0}","access"],["ENOENT","{0}","access"],["ENOENT","{0}","access"]]"#,
            missing
        )
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_numeric_descriptors_support_sync_and_callback_io() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_numeric_descriptors");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (path) { var fd = fs.openSync(path, 'w+'), source = Buffer.from('abcdef'), firstWrite = fs.writeSync(fd, source, 1, 3, 0), secondWrite = fs.writeSync(fd, '!', 3, 'utf8'); fs.fsyncSync(fd); fs.fdatasyncSync(fd); var buffer = Buffer.alloc(6, 46), firstRead = fs.readSync(fd, buffer, 1, 4, 0); fs.ftruncateSync(fd, 3); var size = fs.fstatSync(fd).size; fs.closeSync(fd); var closedCode; try { fs.fstatSync(fd); } catch (error) { closedCode = error.code; } var callbackResult = await new Promise(function(resolve, reject) { fs.open(path, 'r+', function(openError, callbackFd) { if (openError) return reject(openError); fs.write(callbackFd, Buffer.from('XYZ'), 0, 3, 0, function(writeError, written, original) { if (writeError) return reject(writeError); var target = Buffer.alloc(3); fs.read(callbackFd, target, 0, 3, 0, function(readError, read, returned) { if (readError) return reject(readError); fs.fstat(callbackFd, function(statError, stats) { if (statError) return reject(statError); fs.close(callbackFd, function(closeError) { closeError ? reject(closeError) : resolve([written, original.toString(), read, returned === target, target.toString(), stats.size]); }); }); }); }); }); }); return [firstWrite, secondWrite, firstRead, buffer.toString(), size, closedCode, callbackResult, fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_numeric_descriptors_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsNumericDescriptors = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("descriptor.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsNumericDescriptors")
            .unwrap()
            .as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[3,1,4,".bcd!.",3,"EBADF",[3,"XYZ",3,true,"XYZ",3],"XYZ"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_scatter_gather_io_works_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_scatter_gather");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { var fd = fs.openSync(path, 'w+'), written = fs.writevSync(fd, [Buffer.from('ab'), Buffer.from('cd')], 0), first = Buffer.alloc(2), second = Buffer.alloc(3), read = fs.readvSync(fd, [first, second], 0); var callback = await new Promise(function(resolve, reject) { var output = [Buffer.from('e'), Buffer.from('f')]; fs.writev(fd, output, 4, function(writeError, bytesWritten, returnedWrite) { if (writeError) return reject(writeError); var input = [Buffer.alloc(3), Buffer.alloc(3)]; fs.readv(fd, input, 0, function(readError, bytesRead, returnedRead) { readError ? reject(readError) : resolve([bytesWritten, returnedWrite === output, bytesRead, returnedRead === input, input[0].toString(), input[1].toString()]); }); }); }); fs.closeSync(fd); var handle = await promises.open(path, 'r+'), handleWriteBuffers = [Buffer.from('X'), Buffer.from('Y')], handleWrite = await handle.writev(handleWriteBuffers, 1), handleReadBuffers = [Buffer.alloc(3), Buffer.alloc(3)], handleRead = await handle.readv(handleReadBuffers, 0); await handle.close(); return [written, read, first.toString(), second.subarray(0, 2).toString(), callback, handleWrite.bytesWritten, handleWrite.buffers === handleWriteBuffers, handleRead.bytesRead, handleRead.buffers === handleReadBuffers, handleReadBuffers[0].toString(), handleReadBuffers[1].toString(), fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_scatter_gather_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsScatterGather = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("vectors.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsScatterGather").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[4,4,"ab","cd",[2,true,6,true,"abc","def"],2,true,6,true,"aXY","def","aXYdef"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_statfs_reports_host_capacity_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_statfs");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { var sync = fs.statfsSync(path), bigint = fs.statfsSync(path, { bigint: true }); var callback = await new Promise(function(resolve, reject) { fs.statfs(path, function(error, value) { error ? reject(error) : resolve(value); }); }); var promised = await promises.statfs(path), handle = await promises.open(path + '/value.txt', 'w+'), handled = await handle.statfs({ bigint: true }); await handle.close(); return [sync instanceof fs.StatFs, sync.bsize > 0, sync.blocks > 0, sync.bfree >= sync.bavail, sync.files >= sync.ffree, typeof bigint.bsize, bigint.bsize.toString() === String(sync.bsize), callback.bsize, promised.bsize, typeof handled.blocks, handled.blocks.toString() === String(sync.blocks)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_statfs_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsStatfs = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsStatfs").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let values = parsed.as_array().unwrap();
    assert_eq!(
        &values[..7],
        &[
            serde_json::json!(true),
            serde_json::json!(true),
            serde_json::json!(true),
            serde_json::json!(true),
            serde_json::json!(true),
            serde_json::json!("bigint"),
            serde_json::json!(true)
        ]
    );
    assert_eq!(values[7], values[8]);
    assert_eq!(
        &values[9..],
        &[serde_json::json!("bigint"), serde_json::json!(true)]
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_lstat_and_dirents_identify_symbolic_links() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_lstat_symlinks");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var target = root + '/target.txt', link = root + '/target-link'; fs.writeFileSync(target, 'data'); fs.symlinkSync(target, link); var followed = fs.statSync(link), own = fs.lstatSync(link), callback = await new Promise(function(resolve, reject) { fs.lstat(link, function(error, value) { error ? reject(error) : resolve(value); }); }), promised = await promises.lstat(link), entry = fs.readdirSync(root, { withFileTypes: true }).filter(function(value) { return value.name === 'target-link'; })[0]; return [followed.isFile(), followed.isSymbolicLink(), own.isFile(), own.isSymbolicLink(), (own.mode & fs.constants.S_IFMT) === fs.constants.S_IFLNK, own.size === target.length, callback.isSymbolicLink(), promised.isSymbolicLink(), entry.isFile(), entry.isSymbolicLink()]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_lstat_symlinks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsLstatSymlinks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsLstatSymlinks").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,false,true,true,true,true,true,false,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_lutimes_updates_links_without_touching_targets() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_lutimes");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var target = root + '/target.txt', link = root + '/target-link'; fs.writeFileSync(target, 'data'); fs.symlinkSync(target, link); fs.utimesSync(target, 100, 200); fs.lutimesSync(link, 300, 400); var sync = fs.lstatSync(link); await new Promise(function(resolve, reject) { fs.lutimes(link, 500, 600, function(error) { error ? reject(error) : resolve(); }); }); var callback = await promises.lstat(link); await promises.lutimes(link, new Date(700000), new Date(800000)); var promised = await promises.lstat(link), followed = await promises.stat(link); return [Math.round(sync.atimeMs), Math.round(sync.mtimeMs), Math.round(callback.atimeMs), Math.round(callback.mtimeMs), Math.round(promised.atimeMs), Math.round(promised.mtimeMs), Math.round(followed.atimeMs), Math.round(followed.mtimeMs)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_lutimes_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsLutimes = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsLutimes").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[300000,400000,500000,600000,700000,800000,100000,200000]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_copy_exclusive_and_forced_removal_match_node_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_operation_options");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source.txt', destination = root + '/destination.txt', missing = root + '/missing'; fs.writeFileSync(source, 'source'); fs.writeFileSync(destination, 'existing'); var syncError, callbackError, promiseError, missingError; try { fs.copyFileSync(source, destination, fs.constants.COPYFILE_EXCL); } catch (error) { syncError = [error.code, error.path, error.syscall]; } await new Promise(function(resolve) { fs.copyFile(source, destination, fs.constants.COPYFILE_EXCL, function(error) { callbackError = [error.code, error.path, error.syscall]; resolve(); }); }); try { await promises.copyFile(source, destination, fs.constants.COPYFILE_EXCL); } catch (error) { promiseError = [error.code, error.path, error.syscall]; } fs.rmSync(missing, { force: true }); await new Promise(function(resolve, reject) { fs.rm(missing, { force: true }, function(error) { error ? reject(error) : resolve(); }); }); await promises.rm(missing, { force: true }); try { fs.rmSync(missing); } catch (error) { missingError = error.code; } return [syncError, callbackError, promiseError, fs.readFileSync(destination, 'utf8'), missingError]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_operation_options_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsOperationOptions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsOperationOptions").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let destination = dir.join("destination.txt").to_string_lossy().into_owned();
    assert_eq!(
        result,
        format!(
            r#"[["EEXIST","{0}","copyfile"],["EEXIST","{0}","copyfile"],["EEXIST","{0}","copyfile"],"existing","ENOENT"]"#,
            destination
        )
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_readdir_supports_recursive_and_buffer_results() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_recursive_readdir");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/tree'; fs.mkdirSync(work + '/nested/deep', { recursive: true }); fs.writeFileSync(work + '/root.txt', 'root'); fs.writeFileSync(work + '/nested/value.txt', 'value'); fs.writeFileSync(work + '/nested/deep/end.txt', 'end'); fs.symlinkSync(work + '/nested', work + '/linked'); var sync = fs.readdirSync(work, { recursive: true }).sort(); var callback = await new Promise(function(resolve, reject) { fs.readdir(work, { recursive: true, withFileTypes: true }, function(error, entries) { if (error) return reject(error); resolve(entries.map(function(entry) { return [entry.name, entry.parentPath.slice(work.length), entry.isFile(), entry.isDirectory(), entry.isSymbolicLink()]; }).sort()); }); }); var buffers = (await promises.readdir(work, { recursive: true, encoding: 'buffer' })).map(function(value) { return [Buffer.isBuffer(value), value.toString()]; }).sort(function(left, right) { return left[1].localeCompare(right[1]); }); return [sync, callback, buffers]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_recursive_readdir_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsRecursiveReaddir = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsRecursiveReaddir").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let values = parsed.as_array().unwrap();
    assert_eq!(
        values[0],
        serde_json::json!([
            "linked",
            "nested",
            "nested/deep",
            "nested/deep/end.txt",
            "nested/value.txt",
            "root.txt"
        ])
    );
    assert_eq!(values[1].as_array().unwrap().len(), 6);
    assert_eq!(
        values[1][0],
        serde_json::json!(["deep", "/nested", false, true, false])
    );
    assert_eq!(
        values[1][1],
        serde_json::json!(["end.txt", "/nested/deep", true, false, false])
    );
    assert_eq!(
        values[1][2],
        serde_json::json!(["linked", "", false, false, true])
    );
    assert!(values[2]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry[0] == serde_json::json!(true)));
    assert_eq!(values[2].as_array().unwrap().len(), 6);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_write_flags_honor_append_and_exclusive_creation() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_write_flags");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var path = root + '/flags.txt', exclusive = root + '/exclusive.txt', opened = root + '/opened.txt'; fs.writeFileSync(path, 'one'); fs.writeFileSync(path, '-two', { flag: 'a' }); await new Promise(function(resolve, reject) { fs.writeFile(path, '-three', { flag: 'a' }, function(error) { error ? reject(error) : resolve(); }); }); await promises.writeFile(path, '-four', { flag: 'a' }); fs.appendFileSync(exclusive, 'created', { flag: 'ax' }); var errors = []; try { fs.writeFileSync(path, 'lost', { flag: 'wx' }); } catch (error) { errors.push([error.code, error.path, error.syscall]); } await new Promise(function(resolve) { fs.appendFile(exclusive, 'lost', { flag: 'ax' }, function(error) { errors.push([error.code, error.path, error.syscall]); resolve(); }); }); try { await promises.writeFile(path, 'lost', { flag: 'ax' }); } catch (error) { errors.push([error.code, error.path, error.syscall]); } try { fs.openSync(path, 'wx'); } catch (error) { errors.push([error.code, error.path, error.syscall]); } try { await promises.open(path, 'ax'); } catch (error) { errors.push([error.code, error.path, error.syscall]); } var fd = fs.openSync(opened, 'wx'); fs.closeSync(fd); return [fs.readFileSync(path, 'utf8'), fs.readFileSync(exclusive, 'utf8'), fs.existsSync(opened), fs.statSync(opened).size, errors]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_write_flags_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsWriteFlags = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsWriteFlags").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let path = dir.join("flags.txt").to_string_lossy().into_owned();
    let exclusive = dir.join("exclusive.txt").to_string_lossy().into_owned();
    assert_eq!(
        result,
        format!(
            r#"["one-two-three-four","created",true,0,[["EEXIST","{0}","open"],["EEXIST","{1}","open"],["EEXIST","{0}","open"],["EEXIST","{0}","open"],["EEXIST","{0}","open"]]]"#,
            path, exclusive
        )
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_creation_apis_honor_requested_modes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_creation_modes");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var sync = root + '/sync.txt', callback = root + '/callback.txt', promised = root + '/promised.txt', opened = root + '/opened.txt', asyncOpened = root + '/async-opened.txt', streamed = root + '/streamed.txt', directory = root + '/directory'; fs.writeFileSync(sync, 'sync', { mode: 384 }); await new Promise(function(resolve, reject) { fs.appendFile(callback, 'callback', { mode: '600' }, function(error) { error ? reject(error) : resolve(); }); }); await promises.writeFile(promised, 'promised', { mode: 384 }); var fd = fs.openSync(opened, 'w', 384); fs.closeSync(fd); var handle = await promises.open(asyncOpened, 'w', 384); await handle.close(); fs.mkdirSync(directory, { mode: 448 }); var writer = fs.createWriteStream(streamed, { mode: 384 }); await new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); writer.end('stream'); }); return [sync, callback, promised, opened, asyncOpened, streamed, directory].map(function(path) { return fs.statSync(path).mode & 511; }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_creation_modes_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsCreationModes = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsCreationModes").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[384,384,384,384,384,384,448]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_truncate_and_file_handle_sync_methods_work() {
    use std::ffi::{CStr, CString};
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_registry("builtin_fs_truncate_sync");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { fs.writeFileSync(path, 'abcdef'); fs.truncateSync(path, 3); await new Promise(function(resolve, reject) { fs.truncate(path, 5, function(error) { error ? reject(error) : resolve(); }); }); await promises.truncate(path, 4); var handle = await promises.open(path, 'r+'); await handle.chmod(384); await handle.sync(); await handle.datasync(); await handle.close(); var closedCode; try { await handle.sync(); } catch (error) { closedCode = error.code; } return [fs.statSync(path).size, fs.readFileSync(path).toString('hex'), closedCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_truncate_sync_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsTruncateSync = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("truncate.txt");
    let arguments =
        CString::new(serde_json::to_string(&[path.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsTruncateSync").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[4,"61626300","EBADF"]"#);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_bigint_stats_include_host_identity_and_nanoseconds() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_bigint_stats");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { fs.writeFileSync(path, 'bigint'); var sync = fs.statSync(path, { bigint: true }), callback = await new Promise(function(resolve, reject) { fs.stat(path, { bigint: true }, function(error, value) { error ? reject(error) : resolve(value); }); }), promised = await promises.lstat(path, { bigint: true }), fd = fs.openSync(path, 'r'), descriptor = fs.fstatSync(fd, { bigint: true }); fs.closeSync(fd); var handle = await promises.open(path, 'r'), handled = await handle.stat({ bigint: true }); await handle.close(); function summarize(value) { return [typeof value.size, value.size.toString(), typeof value.ino, value.ino > 0n, typeof value.blocks, typeof value.atimeMs, typeof value.atimeNs, value.atimeNs / 1000000n === value.atimeMs, value.atime instanceof Date]; } return [summarize(sync), callback.size === sync.size, promised.ino === sync.ino, descriptor.dev === sync.dev, handled.nlink === sync.nlink]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_bigint_stats_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsBigintStats = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("bigint.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsBigintStats").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["bigint","6","bigint",true,"bigint","bigint","bigint",true,true],true,true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_promises_watch_iterates_changes_and_honors_abort() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_promises_watch");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/watched'; fs.mkdirSync(work); var iterator = promises.watch(work, { interval: 1, encoding: 'buffer' }); setTimeout(function() { fs.writeFileSync(work + '/value.txt', 'value'); }, 3); var event = await iterator.next(), returned = await iterator.return(), after = await iterator.next(); var controller = new AbortController(), aborted = promises.watch(work, { interval: 1, signal: controller.signal }), pending = aborted.next(), abortName, abortCause; controller.abort('reason'); try { await pending; } catch (error) { abortName = error.name; abortCause = error.cause; } return [event.done, event.value.eventType, Buffer.isBuffer(event.value.filename), event.value.filename.toString(), returned.done, after.done, abortName, abortCause]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_promises_watch_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsPromisesWatch = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsPromisesWatch").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,"rename",true,"value.txt",true,true,"AbortError","reason"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn util_types_identifies_standard_and_typed_objects() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_util_types");
    fs::write(dir.join("index.js"), "var types = require('node:util/types'); module.exports = async function () { return [types.isDate(new Date()), types.isRegExp(/x/), types.isMap(new Map()), types.isSet(new Set()), types.isWeakMap(new WeakMap()), types.isPromise(Promise.resolve()), types.isArrayBuffer(new ArrayBuffer(2)), types.isDataView(new DataView(new ArrayBuffer(2))), types.isTypedArray(new Uint8Array(2)), types.isUint8Array(Buffer.from('x')), types.isNativeError(new TypeError('x')), types.isBoxedPrimitive(Object(4)), types.isAsyncFunction(async function() {}), types.isGeneratorFunction(function*() {}), types.isProxy(new Proxy({}, {}))]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_util_types_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseUtilTypes = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseUtilTypes").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,true,true,true,true,true,true,true,true,true,true,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn node_constants_are_shared_with_filesystem_constants() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_constants");
    fs::write(dir.join("index.js"), "var constants = require('node:constants'); var fs = require('node:fs'); module.exports = function () { return [constants === fs.constants, constants.F_OK, constants.R_OK, constants.W_OK, constants.X_OK, constants.O_CREAT, constants.O_APPEND, constants.S_IFREG, constants.S_IFDIR, constants.COPYFILE_FICLONE_FORCE]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_constants_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseConstants = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseConstants").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,0,4,2,1,64,1024,32768,16384,4]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readline_splits_lines_and_supports_callback_and_promise_questions() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readline");
    fs::write(dir.join("index.js"), "var readline = require('node:readline'); var promiseReadline = require('node:readline/promises'); module.exports = async function () { var writes = []; var output = { write: function(value) { writes.push(String(value)); return true; } }; var rl = readline.createInterface({ output: output, historySize: 2, removeHistoryDuplicates: true }); var lines = []; rl.on('line', function(line) { lines.push(line); }); var callbackAnswer = new Promise(function(resolve) { rl.question('name? ', resolve); }); rl.write('alice\\nnext\\r\\n'); var answer = await callbackAnswer; rl.setPrompt('ready> '); rl.prompt(); rl.pause(); rl.resume(); readline.cursorTo(output, 2, 3); readline.clearLine(output, 0); rl.close(); var prl = promiseReadline.createInterface({ output: output }); var promised = prl.question('age? '); prl.write('42\\n'); var age = await promised; var iterator = prl[Symbol.asyncIterator](); var next = iterator.next(); prl.write('tail\\n'); var iterated = await next; prl.close(); return [answer, lines, rl.history, rl.closed, age, iterated.value, writes, readline.ReadLine === readline.Interface, prl instanceof promiseReadline.Interface]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_readline_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadline = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseReadline").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["alice",["alice","next"],["next","alice"],true,"42","42",["\u001b[4;3H","\u001b[2K"],true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readline_questions_write_to_configured_output() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readline_output");
    fs::write(dir.join("index.js"), "var readline = require('node:readline'); var promises = require('node:readline/promises'); module.exports = async function () { var writes = []; var output = { write: function(value) { writes.push(String(value)); return true; } }; var input = { on: function() {}, off: function() {} }; var callbackInterface = readline.createInterface({ input: input, output: output }); var callbackAnswer = new Promise(function(resolve) { callbackInterface.question('first? ', resolve); }); callbackInterface.write('yes\\n'); var first = await callbackAnswer; var promiseInterface = promises.createInterface({ input: input, output: output }); var secondPromise = promiseInterface.question('second? '); promiseInterface.write('ok\\n'); var second = await secondPromise; callbackInterface.close(); promiseInterface.close(); return [first, second, writes]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_readline_output_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadlineOutput = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseReadlineOutput").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["yes","ok",["first? ","second? "]]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn dns_modules_resolve_local_names_and_report_not_found() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_dns");
    fs::write(dir.join("index.js"), "var dns = require('node:dns'); var promises = require('node:dns/promises'); module.exports = async function () { var callbackLookup = await new Promise(function(resolve, reject) { dns.lookup('localhost', { family: 4 }, function(error, address, family) { error ? reject(error) : resolve([address, family]); }); }); var all = await dns.promises.lookup('localhost', { all: true }); var ipv6 = await promises.resolve6('localhost'); var reverse = await promises.reverse('127.0.0.1'); var errorCode; try { await promises.lookup('does-not-exist.invalid'); } catch (error) { errorCode = [error.code, error.syscall, error.hostname]; } dns.setDefaultResultOrder('ipv4first'); var resolver = new promises.Resolver(); resolver.setServers(['127.0.0.1']); return [callbackLookup, all, ipv6, reverse, errorCode, promises.getDefaultResultOrder(), resolver.getServers()]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_dns_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDns = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseDns").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["127.0.0.1",4],[{"address":"127.0.0.1","family":4},{"address":"::1","family":6}],["::1"],["localhost"],["ENOTFOUND","getaddrinfo","does-not-exist.invalid"],"ipv4first",["127.0.0.1"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn net_builtin_validates_addresses_and_block_lists() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_net");
    fs::write(dir.join("index.js"), "var net = require('node:net'); module.exports = function () { var block = new net.BlockList(); block.addAddress('127.0.0.1'); block.addRange('10.0.0.2', '10.0.0.5'); block.addSubnet('192.168.1.0', 24); block.addAddress('::1', 'ipv6'); var ipv4 = new net.SocketAddress({ address: '127.0.0.1', port: 8080 }); var ipv6 = new net.SocketAddress({ address: '::1', family: 'ipv6' }); var invalidPort = false; try { new net.SocketAddress({ address: '127.0.0.1', port: 70000 }); } catch (error) { invalidPort = error instanceof RangeError; } return [net.isIP('127.0.0.1'), net.isIP('2001:db8::1'), net.isIP('999.0.0.1'), net.isIPv4('01.2.3.4'), net.isIPv6('::ffff:192.0.2.1'), block.check('127.0.0.1'), block.check('10.0.0.4'), block.check('10.0.0.9'), block.check('192.168.1.88'), block.check('::1'), ipv4.toJSON(), ipv6.family, invalidPort]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_net_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNet = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNet").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[4,6,0,false,true,true,true,false,true,true,{"address":"127.0.0.1","port":8080,"family":"ipv4","flowlabel":0},"ipv6",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn net_socket_exchanges_bytes_with_a_real_tcp_peer() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
        assert_eq!(request, b"ping");
        stream.write_all(b"pong").unwrap();
    });

    let dir = temp_registry("builtin_net_socket");
    fs::write(dir.join("index.js"), "var net = require('node:net'); module.exports = async function (port) { var events = []; var socket = new net.Socket(); socket.on('connect', function() { events.push('connect'); }); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); }); socket.on('end', function() { events.push('end'); }); var completed = new Promise(function(resolve, reject) { socket.on('error', reject); socket.on('close', function(hadError) { events.push('close:' + hadError); resolve(); }); }); socket.connect(port, '127.0.0.1'); socket.end('ping'); await completed; return [events, socket.bytesWritten, socket.bytesRead, socket.destroyed, socket.remotePort, socket.remoteFamily]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_net_socket_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetSocket = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNetSocket").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(r#"[["connect","data:pong","end","close:false"],4,4,true,{port},"IPv4"]"#)
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_client_requests_and_parses_a_real_chunked_response() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with("GET /items?q=thaw HTTP/1.1\r\n"));
        assert!(request.to_ascii_lowercase().contains("x-thaw: enabled\r\n"));
        stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nX-Reply: yes\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nConnection: close\r\n\r\n4\r\nthaw\r\n3\r\n-ok\r\n0\r\n\r\n",
                )
                .unwrap();
    });

    let dir = temp_registry("builtin_http_client");
    fs::write(
            dir.join("index.js"),
            "var http = require('node:http'); module.exports = async function (port) { return await new Promise(function(resolve, reject) { var request = http.get({ hostname: '127.0.0.1', port: port, path: '/items?q=thaw', headers: { 'X-Thaw': 'enabled' } }, function(response) { var chunks = []; response.setEncoding('utf8'); response.on('data', function(chunk) { chunks.push(chunk); }); response.on('end', function() { resolve([response.statusCode, response.statusMessage, response.httpVersion, response.headers['x-reply'], response.headers['set-cookie'], response.rawHeaders.length, chunks.join(''), response.complete, request.finished, request.destroyed, http.METHODS.indexOf('GET') >= 0, http.STATUS_CODES[200]]); }); }); request.on('error', reject); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http_client_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpClient = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpClient").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[200,"OK","1.1","yes",["a=1","b=2"],10,"thaw-ok",true,true,false,true,"OK"]"#
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_client_emits_informational_responses_and_parses_trailers() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
        stream
                .write_all(b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\nHTTP/1.1 100 Continue\r\nX-Interim: yes\r\n\r\nHTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTrailer: X-Check, Set-Cookie\r\nConnection: close\r\n\r\n5\r\nhello\r\n0\r\nX-Check: one\r\nX-Check: two\r\nSet-Cookie: t=1\r\n\r\n")
                .unwrap();
    });

    let dir = temp_registry("builtin_http_information_trailers");
    fs::write(
            dir.join("index.js"),
            "var http = require('node:http'); module.exports = async function (port) { return new Promise(function(resolve, reject) { var information = [], continued = 0, request = http.get({ hostname: '127.0.0.1', port: port, path: '/' }, function(response) { var chunks = []; response.on('data', function(chunk) { chunks.push(chunk); }); response.on('end', function() { resolve([information, continued, Buffer.concat(chunks).toString(), response.trailers['x-check'], response.trailers['set-cookie'], response.trailersDistinct['x-check'], response.rawTrailers.length, response.complete]); }); }); request.on('information', function(info) { information.push([info.statusCode, info.statusMessage, info.headers.link || info.headers['x-interim'], info.httpVersionMajor, info.httpVersionMinor]); }); request.on('continue', function() { continued++; }); request.on('error', reject); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http_information_trailers_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpInformation = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpInformation").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[[103,"Early Hints","</style.css>; rel=preload",1,1],[100,"Continue","yes",1,1]],1,"hello","one, two",["t=1"],["one","two"],6,true]"#
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn global_fetch_sends_requests_follows_redirects_and_returns_responses() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut brotli = brotli::CompressorWriter::new(Vec::new(), 4096, 5, 22);
    brotli.write_all(b"compressed").unwrap();
    let brotli = brotli.into_inner();
    let server = std::thread::spawn(move || {
        for expected in [
            "GET /redirect ",
            "GET /final ",
            "POST /echo ",
            "POST /multipart ",
            "POST /post-redirect ",
            "GET /post-final ",
            "GET /gzip ",
            "GET /deflate ",
            "GET /br ",
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            stream.read_to_end(&mut request).unwrap();
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with(expected), "{request}");
            if expected.contains("post-redirect") {
                assert!(request.ends_with("again"));
                stream
                        .write_all(b"HTTP/1.1 302 Found\r\nLocation: /post-final#ignored\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .unwrap();
            } else if expected.contains("redirect") {
                stream
                        .write_all(b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .unwrap();
            } else if expected == "GET /final " {
                stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}")
                        .unwrap();
            } else if expected.contains("echo") {
                assert!(request.to_ascii_lowercase().contains("x-thaw: enabled\r\n"));
                assert!(request.ends_with("payload"));
                stream
                        .write_all(b"HTTP/1.1 201 Created\r\nContent-Type: text/plain\r\nContent-Length: 7\r\nConnection: close\r\n\r\npayload")
                        .unwrap();
            } else if expected.contains("multipart") {
                let lower = request.to_ascii_lowercase();
                assert!(lower
                    .contains("content-type: multipart/form-data; boundary=----thaw-formdata-"));
                assert!(request.contains("name=\"title\"\r\n\r\nthaw"));
                assert!(request.contains("name=\"asset\"; filename=\"note.txt\""));
                assert!(request.ends_with("file-body\r\n------thaw-formdata-1--\r\n"));
                stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: multipart/form-data; boundary=reply-boundary\r\nConnection: close\r\n\r\n--reply-boundary\r\nContent-Disposition: form-data; name=\"answer\"\r\n\r\n42\r\n--reply-boundary\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"reply.txt\"\r\nContent-Type: text/plain\r\n\r\nreply-body\r\n--reply-boundary--\r\n")
                        .unwrap();
            } else if expected.contains("gzip")
                || expected.contains("deflate")
                || expected.contains("/br")
            {
                assert!(request
                    .to_ascii_lowercase()
                    .contains("accept-encoding: gzip, deflate, br\r\n"));
                let (encoding, compressed): (&str, &[u8]) = if expected.contains("gzip") {
                    (
                        "gzip",
                        &[
                            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x4b, 0xce,
                            0xcf, 0x2d, 0x28, 0x4a, 0x2d, 0x2e, 0x4e, 0x4d, 0x01, 0x00, 0x1e, 0x4b,
                            0x56, 0x97, 0x0a, 0x00, 0x00, 0x00,
                        ],
                    )
                } else if expected.contains("deflate") {
                    (
                        "deflate",
                        &[
                            0x78, 0x9c, 0x4b, 0xce, 0xcf, 0x2d, 0x28, 0x4a, 0x2d, 0x2e, 0x4e, 0x4d,
                            0x01, 0x00, 0x17, 0x3f, 0x04, 0x36,
                        ],
                    )
                } else {
                    ("br", &brotli)
                };
                write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Encoding: {encoding}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        compressed.len()
                    )
                    .unwrap();
                stream.write_all(compressed).unwrap();
            } else {
                assert!(!request.to_ascii_lowercase().contains("content-type:"));
                assert!(!request.to_ascii_lowercase().contains("content-length:"));
                stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nrewritten")
                        .unwrap();
            }
        }
    });

    let dir = temp_registry("global_fetch");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function (port) { var base = 'http://127.0.0.1:' + port; var redirected = await fetch(base + '/redirect'), cookies = redirected.headers.getSetCookie(), json = await redirected.json(); var posted = await fetch(new Request(base + '/echo', { method: 'POST', headers: { 'X-Thaw': 'enabled' }, body: 'payload' })), before = posted.bodyUsed, text = await posted.text(); var data = new FormData(); data.append('title', 'thaw'); data.append('asset', new Blob(['file-body'], { type: 'text/plain' }), 'note.txt'); var multipartRequest = new Request(base + '/multipart', { method: 'POST', body: data }), parsedRequest = await multipartRequest.clone().formData(), multipart = await fetch(multipartRequest), parsed = await multipart.formData(), upload = parsed.get('upload'); var rewritten = await fetch(base + '/post-redirect#source', { method: 'POST', body: 'again' }), rewrittenText = await rewritten.text(), gzip = await fetch(base + '/gzip'), gzipText = await gzip.text(), deflate = await fetch(base + '/deflate'), deflateText = await deflate.text(), br = await fetch(base + '/br'), brText = await br.text(); var controller = new AbortController(), abortReason; controller.abort('stop'); try { await fetch(base + '/unused', { signal: controller.signal }); } catch (error) { abortReason = error; } var schemeError; try { await fetch('file:///tmp/value'); } catch (error) { schemeError = error instanceof TypeError; } return [redirected.status, redirected.ok, redirected.redirected, redirected.url, cookies, json.ok, posted.status, posted.statusText, posted.headers.get('content-type'), before, posted.bodyUsed, text, typeof fetch, abortReason, schemeError, parsedRequest.get('title'), await parsedRequest.get('asset').text(), parsed.get('answer'), upload instanceof File, upload.name, upload.type, await upload.text(), rewritten.redirected, rewritten.url, rewrittenText, gzipText, gzip.headers.get('content-encoding'), deflateText, deflate.headers.get('content-encoding'), brText, br.headers.get('content-encoding')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("global_fetch_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 6);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFetch = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseFetch").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(
            r#"[200,true,true,"http://127.0.0.1:{port}/final",["a=1","b=2"],true,201,"Created","text/plain",false,true,"payload","function","stop",true,"thaw","file-body","42",true,"reply.txt","text/plain","reply-body",true,"http://127.0.0.1:{port}/post-final","rewritten","compressed","gzip","compressed","deflate","compressed","br"]"#
        )
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn global_fetch_resolves_headers_before_delayed_body_chunks() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\n")
            .unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(250));
        stream.write_all(b"one").unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        stream.write_all(b"two").unwrap();
        drop(stream);
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n")
            .unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let _ = stream.write_all(b"late");
    });

    let dir = temp_registry("global_fetch_streaming");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function (port) { var base = 'http://127.0.0.1:' + port, started = Date.now(), response = await fetch(base + '/slow'), headersElapsed = Date.now() - started, reader = response.body.getReader(), first = await reader.read(), firstElapsed = Date.now() - started, second = await reader.read(), done = await reader.read(); var controller = new AbortController(), abortedResponse = await fetch(base + '/abort', { signal: controller.signal }), abortedReader = abortedResponse.body.getReader(), pending = abortedReader.read(), bodyReason; controller.abort('body-stop'); try { await pending; } catch (error) { bodyReason = error; } return [headersElapsed < 200, firstElapsed >= 200, new TextDecoder().decode(first.value), new TextDecoder().decode(second.value), done.done, response.bodyUsed, bodyReason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("global_fetch_streaming_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamingFetch = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseStreamingFetch").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,true,"one","two",true,true,"body-stop"]"#);
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_agents_and_header_validators_share_across_https() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_http_agents");
    fs::write(
            dir.join("index.js"),
            "var http = require('node:http'), https = require('node:https'); module.exports = function () { var agent = new http.Agent({ keepAlive: true, maxSockets: 7, maxTotalSockets: 9 }); var secureAgent = new https.Agent({ keepAliveMsecs: 250 }); var request = http.request({ host: 'example.com', port: 8080, agent: agent }); var secureRequest = https.request({ host: 'example.com', agent: false }); request.setHeader('X-Valid', ['one', 'two']); var errors = []; try { http.validateHeaderName('bad name'); } catch (error) { errors.push(error.code); } try { https.validateHeaderValue('X-Test', 'bad\\nvalue'); } catch (error) { errors.push(error.code); } try { request.setHeader('X-Missing', undefined); } catch (error) { errors.push(error.code); } http.setMaxIdleHTTPParsers(10); return [agent instanceof http.Agent, secureAgent instanceof https.Agent, secureAgent instanceof http.Agent, request.agent === agent, secureRequest.agent, request.getHeader('x-valid'), agent.getName({ host: 'example.com', port: 8080, family: 4 }), agent.keepAlive, agent.maxSockets, agent.maxTotalSockets, agent.totalSocketCount, secureAgent.protocol, secureAgent.defaultPort, secureAgent.keepAliveMsecs, http.globalAgent instanceof http.Agent, https.globalAgent instanceof https.Agent, errors]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http_agents_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 6);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpAgents = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpAgents").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,true,null,["one","two"],"example.com:8080::4",true,7,9,0,"https:",443,250,true,true,["ERR_INVALID_HTTP_TOKEN","ERR_INVALID_CHAR","ERR_HTTP_INVALID_HEADER_VALUE"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn child_process_sync_apis_execute_and_report_node_shaped_results() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_child_process_sync");
    fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = function (cwd) { var failed = cp.spawnSync('/bin/sh', ['-c', 'printf out; printf err >&2; exit 3'], { encoding: 'utf8', cwd: cwd }); var input = cp.spawnSync('/bin/sh', ['-c', 'cat'], { input: Buffer.from('stdin') }); var environment = cp.execFileSync('/bin/sh', ['-c', 'printf "$VALUE"'], { encoding: 'utf8', env: { VALUE: 'env-ok' } }); var shell = cp.execSync('printf shell-ok', { encoding: 'utf8' }); var missing = cp.spawnSync('/thaw/does-not-exist', []); var thrown; try { cp.execFileSync('/bin/sh', ['-c', 'printf bad >&2; exit 7'], { encoding: 'utf8' }); } catch (error) { thrown = [error.status, error.stderr, error.stdout, error.pid > 0]; } var overflow = cp.spawnSync('/bin/sh', ['-c', 'printf 12345'], { maxBuffer: 4 }); return [failed.status, failed.signal, failed.stdout, failed.stderr, failed.output[1], failed.pid > 0, Buffer.isBuffer(input.stdout), input.stdout.toString(), input.stderr.length, environment, shell, missing.status, missing.error.code, missing.error.path, thrown, overflow.status, overflow.error.code]; };"#,
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_child_process_sync_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessSync = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseChildProcessSync").unwrap();
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[3,null,"out","err","out",true,true,"stdin",0,"env-ok","shell-ok",null,"ENOENT","/thaw/does-not-exist",[7,"bad","",true],null,"ENOBUFS"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn child_process_async_apis_stream_and_emit_lifecycle_events() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_child_process_async");
    fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = async function () { var child = cp.spawn('/bin/sh', ['-c', 'read value; printf "out:$value"; printf "err:$value" >&2; exit 4']); var events = [], stdout = [], stderr = []; child.on('spawn', function() { events.push('spawn:' + (child.pid > 0)); }); child.stdout.on('data', function(value) { stdout.push(value.toString()); }); child.stderr.on('data', function(value) { stderr.push(value.toString()); }); var closed = new Promise(function(resolve, reject) { child.on('error', reject); child.on('exit', function(code, signal) { events.push('exit:' + code + ':' + signal); }); child.on('close', function(code, signal) { events.push('close:' + code + ':' + signal); resolve(); }); }); child.stdin.end('hello\n'); await closed; var executed = await new Promise(function(resolve) { cp.exec('printf callback', { encoding: 'utf8' }, function(error, out, err) { resolve([error, out, err]); }); }); var failed = await new Promise(function(resolve) { cp.execFile('/bin/sh', ['-c', 'printf failure >&2; exit 6'], { encoding: 'utf8' }, function(error, out, err) { resolve([error.code, error.stderr, out, err]); }); }); var killed = cp.spawn('/bin/sh', ['-c', 'sleep 10']); var killedResult = new Promise(function(resolve, reject) { killed.on('error', reject); killed.on('spawn', function() { killed.kill('SIGINT'); }); killed.on('close', function(code, signal) { resolve([code, signal, killed.killed]); }); }); var missing = cp.spawn('/thaw/missing-async', []), missingResult = new Promise(function(resolve) { var code; missing.on('error', function(error) { code = error.code; }); missing.on('close', function(exitCode, signal) { resolve([code, exitCode, signal]); }); }); return [events, stdout.join(''), stderr.join(''), child.exitCode, child.signalCode, child.stdin.writableEnded, child instanceof cp.ChildProcess, executed, failed, await killedResult, await missingResult]; };"#,
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_child_process_async_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessAsync = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseChildProcessAsync").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["spawn:true","exit:4:null","close:4:null"],"out:hello","err:hello",4,null,true,true,[null,"callback",""],[6,"failure","","failure"],[null,"SIGINT",true],["ENOENT",null,null]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn child_process_spawn_honors_stdio_timeout_abort_and_detached_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_child_process_options");
    fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = async function () { function close(child) { return new Promise(function(resolve) { child.on('error', function() {}); child.on('close', function(code, signal) { resolve([code, signal]); }); }); } var ignored = cp.spawn('/bin/sh', ['-c', 'printf hidden; printf secret >&2'], { stdio: 'ignore' }); var ignoredResult = await close(ignored); var inheritedOut = '', inheritedErr = '', oldOut = process.stdout, oldErr = process.stderr; process.stdout = { write: function(value) { inheritedOut += Buffer.from(value).toString(); return true; } }; process.stderr = { write: function(value) { inheritedErr += Buffer.from(value).toString(); return true; } }; var inherited = cp.spawn('/bin/sh', ['-c', 'printf visible; printf warning >&2'], { stdio: ['ignore', 'inherit', 'inherit'] }); var inheritedResult = await close(inherited); process.stdout = oldOut; process.stderr = oldErr; var timed = cp.spawn('/bin/sh', ['-c', 'sleep 10'], { timeout: 20 }); var timedResult = await close(timed); var controller = new AbortController(), aborted = cp.spawn('/bin/sh', ['-c', 'sleep 10'], { signal: controller.signal }), abortError; aborted.on('error', function(error) { abortError = [error.name, error.code]; }); var abortedResult = close(aborted); controller.abort('stop'); abortedResult = await abortedResult; var detached = cp.spawn('/bin/sh', ['-c', 'ps -o sid= -p $$'], { detached: true }), detachedOutput = ''; detached.stdout.on('data', function(value) { detachedOutput += value.toString(); }); var detachedResult = await close(detached); var invalid; try { cp.spawn('/bin/true', [], { stdio: 'invalid' }); } catch (error) { invalid = error.code; } return [[ignored.stdin, ignored.stdout, ignored.stderr, ignoredResult], [inherited.stdin, inherited.stdout, inherited.stderr, inheritedOut, inheritedErr, inheritedResult], [timed._timedOut, timedResult], [abortError, abortedResult], [detached.detached, Number(detachedOutput.trim()) === detached.pid, detachedResult], invalid]; };"#,
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_child_process_options_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessOptions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseChildProcessOptions").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[null,null,null,[0,null]],[null,null,null,"visible","warning",[0,null]],[true,[null,"SIGTERM"]],[["AbortError","ABORT_ERR"],[null,"SIGTERM"]],[true,true,[0,null]],"ERR_INVALID_ARG_VALUE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn child_process_fork_exchanges_ipc_messages_and_disconnects() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_child_process_fork");
    fs::write(
            dir.join("child.js"),
            "console.log('child-ready'); process.on('message', function(value) { process.send({ answer: value.base + 2, argument: process.argv[2] }, function() { process.disconnect(); }); });",
        )
        .unwrap();
    fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = async function (childPath) { var child = cp.fork(childPath, ['argument'], { silent: true, execArgv: ['--no-warnings'] }), events = [], output = '', callbackCode; child.stdout.on('data', function(value) { output += value.toString(); }); var completed = new Promise(function(resolve, reject) { child.on('error', reject); child.on('spawn', function() { events.push('spawn'); child.send({ base: 40 }, function(error) { callbackCode = error && error.code || null; }); }); child.on('message', function(value) { events.push('message:' + value.answer + ':' + value.argument); }); child.on('disconnect', function() { events.push('disconnect'); }); child.on('close', function(code, signal) { events.push('close:' + code + ':' + signal); resolve(); }); }); await completed; var closed; try { child.send({ late: true }); } catch (error) { closed = error.code; } return [events, output.trim(), callbackCode, child.connected, child.channel, closed, child instanceof cp.ChildProcess]; };"#,
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_child_process_fork_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessFork = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseChildProcessFork").unwrap();
    let child_path = dir.join("child.js").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[child_path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["spawn","message:42:argument","disconnect","close:0:null"],"child-ready",null,false,null,"ERR_IPC_CHANNEL_CLOSED",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_server_parses_and_replies_to_a_real_tcp_client() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::time::Duration;

    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream
                .write_all(b"POST /submit HTTP/1.1\r\nHost: localhost\r\nX-Client: rust\r\nContent-Length: 4\r\nConnection: close\r\n\r\nping")
                .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });

    let dir = temp_registry("builtin_http_server");
    fs::write(
            dir.join("index.js"),
            "var http = require('node:http'); module.exports = async function (port) { var observed = []; var server = http.createServer(function(request, response) { var body = []; observed.push(request.method, request.url, request.headers['x-client'], request.httpVersion); request.on('data', function(chunk) { body.push(chunk.toString()); }); request.on('end', function() { observed.push(body.join('')); response.statusCode = 201; response.statusMessage = 'Stored'; response.setHeader('X-Server', 'thaw'); response.setHeader('Set-Cookie', ['a=1', 'b=2']); response.write('po'); response.end('ng', function() { server.close(); }); }); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return [observed, server.listening, server instanceof http.Server]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http_server_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpServer = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpServer").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["POST","/submit","rust","1.1","ping"],false,true]"#
    );
    let response = client.join().unwrap();
    assert!(response.starts_with("HTTP/1.1 201 Stored\r\n"));
    assert!(response.contains("X-Server: thaw\r\n"));
    assert!(response.contains("Set-Cookie: a=1\r\nSet-Cookie: b=2\r\n"));
    assert!(response.ends_with("\r\n\r\npong"));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn https_client_verifies_a_custom_ca_and_parses_http() {
    use rustls::pki_types::ServerName;
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    use rustls::{
        ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
    };
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::Arc;

    let certificate_dir = temp_registry("builtin_https_certificate");
    let key_pem = certificate_dir.join("key.pem");
    let cert_pem = certificate_dir.join("cert.pem");
    let key_der = certificate_dir.join("key.der");
    let cert_der = certificate_dir.join("cert.der");
    assert!(Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature,keyEncipherment",
            "-addext",
            "extendedKeyUsage=serverAuth",
            "-keyout",
        ])
        .arg(&key_pem)
        .arg("-out")
        .arg(&cert_pem)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"])
        .arg(&cert_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&cert_der)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("openssl")
        .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
        .arg(&key_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&key_der)
        .status()
        .unwrap()
        .success());
    let certificate = CertificateDer::from(fs::read(&cert_der).unwrap());
    let private_key = PrivatePkcs8KeyDer::from(fs::read(&key_der).unwrap()).into();
    let config = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], private_key)
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let connection = ServerConnection::new(config).unwrap();
        let mut stream = StreamOwned::new(connection, socket);
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with("GET /secure HTTP/1.1\r\n"));
        stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nX-Secure: yes\r\nConnection: close\r\n\r\nsecret")
                .unwrap();
        stream.conn.send_close_notify();
        stream.flush().unwrap();
    });

    let dir = temp_registry("builtin_https_client");
    fs::write(
            dir.join("index.js"),
            "var https = require('node:https'); module.exports = async function (port, ca) { return await new Promise(function(resolve, reject) { var request = https.get({ hostname: '127.0.0.1', port: port, path: '/secure', ca: ca }, function(response) { var body = []; response.setEncoding('utf8'); response.on('data', function(chunk) { body.push(chunk); }); response.on('end', function() { resolve([response.statusCode, response.headers['x-secure'], body.join(''), request.protocol, request.socket.encrypted, request.socket.authorized, https.globalAgent.protocol]); }); }); request.on('error', reject); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_https_client_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 6);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpsClient = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpsClient").unwrap();
    let ca = fs::read_to_string(&cert_pem).unwrap();
    let arguments = CString::new(serde_json::to_string(&(port, ca)).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[200,"yes","secret","https:",true,true,"https:"]"#
    );
    server.join().unwrap();

    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(fs::read(&cert_der).unwrap()))
        .unwrap();
    let client_config = Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let tls_client = std::thread::spawn(move || {
        let socket = loop {
            match TcpStream::connect(("127.0.0.1", server_port)) {
                Ok(socket) => break socket,
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        };
        let connection =
            ClientConnection::new(client_config, ServerName::try_from("localhost").unwrap())
                .unwrap();
        let mut stream = StreamOwned::new(connection, socket);
        stream
            .write_all(b"GET /from-rust HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        stream.conn.send_close_notify();
        stream.flush().unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });
    let server_dir = temp_registry("builtin_https_server");
    fs::write(
            server_dir.join("index.js"),
            "var https = require('node:https'); module.exports = async function (port, cert, key) { var observed; var server = https.createServer({ cert: cert, key: key }, function(request, response) { observed = [request.method, request.url, request.socket.encrypted, request.socket.authorized]; response.statusCode = 202; response.setHeader('X-TLS', 'yes'); response.end('secure-server', function() { server.close(); }); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return [observed, server.listening, server instanceof https.Server]; };",
        )
        .unwrap();
    let server_node_modules = temp_registry("builtin_https_server_node_modules");
    let (server_bundle, _, server_file_count, _) =
        bundle_commonjs_package(&server_node_modules, "secure-pkg", &server_dir, "index.js")
            .unwrap();
    assert_eq!(server_file_count, 6);
    let server_script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; {server_bundle} globalThis.exerciseHttpsServer = module.exports;");
    let server_source = CString::new(server_script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(server_source.as_ptr()), 1);
    let server_function = CString::new("exerciseHttpsServer").unwrap();
    let server_arguments = CString::new(
        serde_json::to_string(&(
            server_port,
            fs::read_to_string(&cert_pem).unwrap(),
            fs::read_to_string(&key_pem).unwrap(),
        ))
        .unwrap(),
    )
    .unwrap();
    let server_result =
        thaw_quickjs::thaw_js_call(server_function.as_ptr(), server_arguments.as_ptr());
    let server_result = unsafe { CStr::from_ptr(server_result) }.to_string_lossy();
    assert_eq!(
        server_result,
        r#"[["GET","/from-rust",true,true],false,true]"#
    );
    let response = tls_client.join().unwrap();
    assert!(response.starts_with("HTTP/1.1 202 Accepted\r\n"));
    assert!(response.contains("X-TLS: yes\r\n"));
    assert!(response.ends_with("\r\n\r\nsecure-server"));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    let _ = fs::remove_dir_all(&server_dir);
    let _ = fs::remove_dir_all(&server_node_modules);
    let _ = fs::remove_dir_all(&certificate_dir);
}

#[test]
fn net_server_accepts_and_replies_to_a_real_tcp_client() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::time::Duration;

    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let client = std::thread::spawn(move || {
        (0..3)
            .map(|index| {
                let mut stream = loop {
                    match TcpStream::connect(("127.0.0.1", port)) {
                        Ok(stream) => break stream,
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                };
                stream.write_all(format!("ping{index}").as_bytes()).unwrap();
                stream.shutdown(Shutdown::Write).unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                response
            })
            .collect::<Vec<_>>()
    });

    let dir = temp_registry("builtin_net_server");
    fs::write(dir.join("index.js"), "var net = require('node:net'); module.exports = async function (port) { var events = [], handled = 0; var server; var closed = new Promise(function(resolve, reject) { server = net.createServer(function(socket) { events.push('connection:' + socket.remoteFamily); socket.on('error', reject); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); handled++; socket.end('pong' + handled, function() { if (handled === 3) server.close(); }); }); }); server.on('error', reject); server.on('listening', function() { var address = server.address(); events.push('listening:' + address.address + ':' + address.port); }); server.on('close', function() { events.push('close'); resolve(); }); server.listen(port, '127.0.0.1'); }); await closed; return [events, server.listening, server.address().port, server.connections, server.ref() === server, server.unref() === server]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_net_server_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetServer = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNetServer").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(
            r#"[["listening:127.0.0.1:{port}","connection:IPv4","data:ping0","connection:IPv4","data:ping1","connection:IPv4","data:ping2","close"],false,{port},0,true,true]"#
        )
    );
    assert_eq!(client.join().unwrap(), ["pong1", "pong2", "pong3"]);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn vm_builtin_runs_scripts_in_contexts_and_compiles_functions() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_vm");
    fs::write(dir.join("index.js"), "var vm = require('node:vm'); module.exports = async function () { var sandbox = { value: 3 }; var context = vm.createContext(sandbox); var script = new vm.Script('value *= 4; created = value + 1; value', { filename: 'sample.js' }); var contextual = script.runInContext(context); var fresh = vm.runInNewContext('input + 2', { input: 5 }); var current = vm.runInThisContext('6 * 7'); var add = vm.compileFunction('return left + right;', ['left', 'right'], { filename: 'add.js' }); var memory = await vm.measureMemory(); var cached = script.createCachedData(); return [contextual, sandbox.value, sandbox.created, fresh, current, add(8, 9), vm.isContext(context), vm.isContext({}), cached.toString().includes('value *= 4'), script.cachedDataRejected, memory.total.jsMemoryEstimate, vm.getDefaultContext() === globalThis, typeof vm.constants.DONT_CONTEXTIFY]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_vm_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseVm = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseVm").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[12,12,13,7,42,17,true,false,true,false,0,true,"symbol"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn punycode_builtin_converts_unicode_labels_and_code_points() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_punycode");
    fs::write(dir.join("index.js"), "var punycode = require('node:punycode'); module.exports = function () { var encoded = punycode.encode('mañana'); var snowman = punycode.encode('☃-⌘'); var points = punycode.ucs2.decode('A😀Z'); return [encoded, punycode.decode(encoded), snowman, punycode.decode(snowman), punycode.toASCII('mañana.com'), punycode.toUnicode('xn--bcher-kva.example'), points, punycode.ucs2.encode(points), punycode.toASCII('user@bücher.example'), punycode.version]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_punycode_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exercisePunycode = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exercisePunycode").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["maana-pta","mañana","--dqo34k","☃-⌘","xn--maana-pta.com","bücher.example",[65,128512,90],"A😀Z","user@xn--bcher-kva.example","2.1.0"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn dgram_socket_exchanges_real_udp_datagrams() {
    use std::ffi::{CStr, CString};
    use std::net::UdpSocket;
    use std::time::Duration;

    let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_millis(30)))
        .unwrap();
    let client = std::thread::spawn(move || {
        let mut response = [0u8; 16];
        loop {
            peer.send_to(b"ping", ("127.0.0.1", port)).unwrap();
            if let Ok((length, _)) = peer.recv_from(&mut response) {
                return response[..length].to_vec();
            }
        }
    });

    let dir = temp_registry("builtin_dgram");
    fs::write(dir.join("index.js"), "var dgram = require('node:dgram'); module.exports = async function (port) { var events = []; var socket = dgram.createSocket('udp4'); var closed = new Promise(function(resolve, reject) { socket.on('error', reject); socket.on('listening', function() { var address = socket.address(); events.push('listening:' + address.family + ':' + address.port); }); socket.on('message', function(message, remote) { events.push('message:' + message.toString() + ':' + remote.family + ':' + remote.size); socket.send('pong', remote.port, remote.address, function(error, written) { if (error) reject(error); else { events.push('sent:' + written); socket.close(); } }); }); socket.on('close', function() { events.push('close'); resolve(); }); }); socket.bind(port, '127.0.0.1'); await closed; return [events, socket.hasRef(), socket.ref() === socket, socket.unref() === socket]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_dgram_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDgram = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseDgram").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(
            r#"[["listening:IPv4:{port}","message:ping:IPv4:4","sent:4","close"],true,true,true]"#
        )
    );
    assert_eq!(client.join().unwrap(), b"pong");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_consumers_collect_streams_and_async_iterables() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_consumers");
    fs::write(dir.join("index.js"), "var stream = require('node:stream'); var consumers = require('node:stream/consumers'); function consume(method, value) { var input = new stream.PassThrough(); var result = consumers[method](input); input.end(value); return result; } module.exports = async function () { var text = await consume('text', 'hello'); var object = await consume('json', '{\"answer\":42}'); var bytes = await consume('buffer', 'abc'); var array = new Uint8Array(await consume('arrayBuffer', 'xy')); var blob = await consume('blob', 'blob'); var iterable = { async *[Symbol.asyncIterator]() { yield 'one'; yield Buffer.from('two'); } }; var joined = await consumers.text(iterable); var sliced = blob.slice(1, 3, 'text/plain'); return [text, object.answer, bytes.toString('hex'), Array.from(array), blob instanceof Blob, blob.size, await blob.text(), sliced.type, await sliced.text(), joined, new File(['x'], 'a.txt', { lastModified: 7 }).name]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_stream_consumers_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseConsumers = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseConsumers").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["hello",42,"616263",[120,121],true,4,"blob","text/plain","lo","onetwo","a.txt"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_web_pipes_transforms_and_exposes_readers_and_writers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_web");
    fs::write(dir.join("index.js"), "var web = require('node:stream/web'); var consumers = require('node:stream/consumers'); module.exports = async function () { var source = new web.ReadableStream({ start: function(controller) { controller.enqueue('one'); controller.enqueue('two'); controller.close(); } }); var upper = new web.TransformStream({ transform: function(chunk, controller) { controller.enqueue(chunk.toUpperCase()); } }); var transformed = await consumers.text(source.pipeThrough(upper)); var writes = []; var writable = new web.WritableStream({ write: function(chunk) { writes.push(chunk); }, close: function() { writes.push('closed'); } }); var writer = writable.getWriter(); await writer.write('value'); await writer.close(); writer.releaseLock(); var encodedSource = new web.ReadableStream({ start: function(controller) { controller.enqueue('hé'); controller.close(); } }); var decoded = await consumers.text(encodedSource.pipeThrough(new web.TextEncoderStream()).pipeThrough(new web.TextDecoderStream())); var readerSource = new web.ReadableStream({ pull: function(controller) { controller.enqueue(7); controller.close(); } }); var reader = readerSource.getReader(); var first = await reader.read(); var done = await reader.read(); reader.releaseLock(); var byteStrategy = new web.ByteLengthQueuingStrategy({ highWaterMark: 8 }); var countStrategy = new web.CountQueuingStrategy({ highWaterMark: 3 }); return [web.ReadableStream === globalThis.ReadableStream, transformed, writes, decoded, first, done.done, readerSource.locked, byteStrategy.highWaterMark, byteStrategy.size(new Uint8Array(4)), countStrategy.highWaterMark, countStrategy.size('x')]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_stream_web_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamWeb = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseStreamWeb").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"ONETWO",["value","closed"],"hé",{"value":7,"done":false},true,false,8,4,3,1]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn global_headers_normalize_duplicate_and_cookie_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_global_headers");
    fs::write(
            dir.join("index.js"),
            "module.exports = function () { function capture(action) { try { action(); } catch (error) { return [error.name, error.code]; } } var headers = new Headers([['X-B', '  one  '], ['x-a', 'a'], ['X-B', 'two'], ['set-cookie', 'a=1'], ['Set-Cookie', 'b=2']]), calls = []; headers.forEach(function(value, name, owner) { calls.push([name, value, owner === headers]); }); var clone = new Headers(headers), record = new Headers({ Z: 1, A: ['x', 'y'] }); headers.set('replace', 'first'); headers.set('replace', 'second'); headers.delete('replace'); return [[...headers], headers.get('X-B'), headers.getSetCookie(), [...headers.keys()], [...headers.values()], headers.has('X-A'), headers.get('missing'), calls, [...clone], [...record], Object.keys(headers), Object.keys(Headers.prototype), Object.prototype.toString.call(headers), capture(function() { headers.set('bad name', 'x'); }), capture(function() { headers.set('x', 'a\\nb'); }), capture(function() { new Headers([['a']]); })]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_global_headers_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHeaders = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseHeaders").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[["set-cookie","a=1"],["set-cookie","b=2"],["x-a","a"],["x-b","one, two"]],"one, two",["a=1","b=2"],["set-cookie","set-cookie","x-a","x-b"],["a=1","b=2","a","one, two"],true,null,[["set-cookie","a=1",true],["set-cookie","b=2",true],["x-a","a",true],["x-b","one, two",true]],[["set-cookie","a=1"],["set-cookie","b=2"],["x-a","a"],["x-b","one, two"]],[["a","x,y"],["z","1"]],[],["append","delete","get","has","set","getSetCookie","keys","values","entries","forEach"],"[object Headers]",["TypeError",null],["TypeError",null],["TypeError",null]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn global_response_consumes_clones_and_constructs_bodies() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_global_response");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var text = new Response('hello'), textBefore = [text.status, text.statusText, text.ok, text.type, text.url, text.redirected, text.headers.get('content-type'), text.body.constructor.name, text.bodyUsed], textValue = await text.text(), textAfter = text.bodyUsed, params = new Response(new URLSearchParams({ a: 'two words' })), paramsForm = await params.formData(), binary = new Response(new Uint8Array([1, 2, 3])), binaryBytes = Array.from(await binary.bytes()), original = new Response('clone'), clone = original.clone(), cloneValues = [await original.text(), await clone.text()], empty = new Response(null, { status: 204 }), emptyText = await empty.text(), json = Response.json({ a: 1 }), jsonValue = [json.headers.get('content-type'), await json.text()], redirect = Response.redirect('https://example.com/a', 307), failure = Response.error(), consumed = new Response('used'); await consumed.text(); var cloneError, statusError, bodyStatusError; try { consumed.clone(); } catch (error) { cloneError = error.name; } try { new Response(null, { status: 199 }); } catch (error) { statusError = error.name; } try { new Response('x', { status: 204 }); } catch (error) { bodyStatusError = error.name; } var blobResponse = new Response(new Blob(['blob'], { type: 'text/custom' })), blob = await blobResponse.blob(); return [textBefore, textValue, textAfter, params.headers.get('content-type'), paramsForm.get('a'), binaryBytes, cloneValues, emptyText, empty.bodyUsed, jsonValue, redirect.status, redirect.headers.get('location'), failure.status, failure.type, failure.ok, failure.body, cloneError, statusError, bodyStatusError, blob.type, await blob.text(), Object.prototype.toString.call(text)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_global_response_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseResponse = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseResponse").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[200,"",true,"default","",false,"text/plain;charset=UTF-8","ReadableStream",false],"hello",true,"application/x-www-form-urlencoded;charset=UTF-8","two words",[1,2,3],["clone","clone"],"",false,["application/json","{\"a\":1}"],307,"https://example.com/a",0,"error",false,null,"TypeError","RangeError","TypeError","text/custom","blob","[object Response]"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn global_request_normalizes_inherits_and_clones_bodies() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_global_request");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { function capture(action) { try { action(); } catch (error) { return error.name; } } var controller = new AbortController(), request = new Request('https://example.com/a?x=1', { method: 'post', headers: { X: ' y ' }, body: 'hello', signal: controller.signal }), before = [request.url, request.method, [...request.headers], request.body.constructor.name, request.bodyUsed, request.cache, request.credentials, request.destination, request.integrity, request.keepalive, request.mode, request.redirect, request.referrer, request.referrerPolicy, request.duplex, request.signal === controller.signal], clone = request.clone(), cloneText = await clone.text(), originalStillUnused = !request.bodyUsed, inherited = new Request(request, { method: 'PUT', headers: { Z: '1' } }), originalTransferred = request.bodyUsed && request.body.locked, inheritedText = await inherited.text(); controller.abort('stop'); await Promise.resolve(); var stream = new ReadableStream({ start: function(value) { value.enqueue(new TextEncoder().encode('stream')); value.close(); } }), streamRequest = new Request('http://example.com/', { method: 'POST', body: stream, duplex: 'half' }), streamText = await streamRequest.text(), params = new Request('http://example.com/', { method: 'POST', body: new URLSearchParams({ a: 'b' }) }), form = await params.formData(); return [before, cloneText, originalStillUnused, inherited.method, inherited.url, [...inherited.headers], inheritedText, originalTransferred, request.signal.aborted, request.signal.reason, inherited.signal.aborted, streamText, form.get('a'), Object.prototype.toString.call(request), capture(function() { new Request('/relative'); }), capture(function() { new Request('http://example.com/', { body: 'x' }); }), capture(function() { new Request('http://example.com/', { method: 'HEAD', body: 'x' }); }), capture(function() { new Request('http://example.com/', { method: 'bad method' }); }), capture(function() { new Request('http://example.com/', { method: 'CONNECT' }); }), capture(function() { new Request('http://example.com/', { method: 'POST', body: new ReadableStream() }); })]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_global_request_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseRequest = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseRequest").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["https://example.com/a?x=1","POST",[["content-type","text/plain;charset=UTF-8"],["x","y"]],"ReadableStream",false,"default","same-origin","","",false,"cors","follow","about:client","","half",false],"hello",true,"PUT","https://example.com/a?x=1",[["z","1"]],"hello",true,true,"stop",true,"stream","b","[object Request]","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn text_decoder_stream_decodes_incrementally_and_flushes_errors() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_text_decoder_streaming");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); module.exports = async function () { var decoder = new web.TextDecoderStream(), writer = decoder.writable.getWriter(), reader = decoder.readable.getReader(), settled = false, pending = reader.read().then(function(result) { settled = true; return result; }); await writer.write(Uint8Array.from([0xf0, 0x9f])); await Promise.resolve(); await Promise.resolve(); var incompletePending = !settled; await writer.write(Uint8Array.from([0x98, 0x80, 0x41])); var decoded = await pending; await writer.close(); var done = await reader.read(), fatal = new TextDecoderStream('utf-8', { fatal: true }), fatalWriter = fatal.writable.getWriter(), fatalReader = fatal.readable.getReader(), fatalRead = fatalReader.read().catch(function(error) { return error; }); await fatalWriter.write(Uint8Array.from([0xe2])); var closeError = await fatalWriter.close().catch(function(error) { return error; }), readError = await fatalRead, closedError = await fatalWriter.closed.catch(function(error) { return error; }); return [incompletePending, decoded.value, done.done, closeError instanceof TypeError, readError === closeError, closedError === closeError, decoder.encoding, decoder.fatal, decoder.ignoreBOM]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_text_decoder_streaming_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTextDecoderStreaming = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseTextDecoderStreaming")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"😀A",true,true,true,true,"utf-8",false,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn text_encoder_stream_preserves_split_surrogate_pairs() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_text_encoder_surrogates");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); async function encodeParts(parts) { var stream = new web.TextEncoderStream(), writer = stream.writable.getWriter(), reader = stream.readable.getReader(), output = [], consume = (async function() { while (true) { var result = await reader.read(); if (result.done) break; output.push(Array.from(result.value)); } })(); for (var part of parts) await writer.write(part); await writer.close(); await consume; return output; } module.exports = async function () { var encoder = new TextEncoder(), destination = new Uint8Array(4), into = encoder.encodeInto('\\ud83dA', destination); return [Array.from(encoder.encode('\\ud83d')), Array.from(encoder.encode('\\ude00')), into, Array.from(destination), await encodeParts(['\\ud83d', '\\ude00A']), await encodeParts(['X\\ud83d']), await encodeParts(['\\ud83d', 'B'])]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_text_encoder_surrogates_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTextEncoderSurrogates = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseTextEncoderSurrogates")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[239,191,189],[239,191,189],{"read":2,"written":4},[239,191,189,65],[[240,159,152,128,65]],[[88],[239,191,189]],[[239,191,189,66]]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_compression_streams_round_trip_supported_formats() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_compression");
    fs::write(dir.join("index.js"), "var web = require('node:stream/web'); var consumers = require('node:stream/consumers'); async function roundTrip(format) { var source = new web.ReadableStream({ start: function(controller) { controller.enqueue(new TextEncoder().encode('thaw ')); controller.enqueue(new TextEncoder().encode('compression')); controller.close(); } }); return consumers.text(source.pipeThrough(new web.CompressionStream(format)).pipeThrough(new web.DecompressionStream(format)).pipeThrough(new web.TextDecoderStream())); } module.exports = async function () { var gzipSource = new web.ReadableStream({ start: function(controller) { controller.enqueue(new TextEncoder().encode('header')); controller.close(); } }); var gzip = await consumers.buffer(gzipSource.pipeThrough(new CompressionStream('gzip'))); var unsupported = false; try { new CompressionStream('brotli'); } catch (error) { unsupported = error instanceof TypeError; } return [await roundTrip('gzip'), await roundTrip('deflate'), await roundTrip('deflate-raw'), await roundTrip('br'), gzip[0], gzip[1], web.CompressionStream === globalThis.CompressionStream, unsupported]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_web_compression_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseCompressionStreams = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseCompressionStreams").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["thaw compression","thaw compression","thaw compression","thaw compression",31,139,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_compression_streams_emit_and_decode_incrementally() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_compression_incremental");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var encoder = new TextEncoder(), compression = new CompressionStream('gzip'), compressionWriter = compression.writable.getWriter(), compressionReader = compression.readable.getReader(), firstCompressedRead = compressionReader.read(); await compressionWriter.write(encoder.encode('hello')); var firstCompressed = await firstCompressedRead, compressed = [firstCompressed.value], collectCompressed = (async function() { while (true) { var result = await compressionReader.read(); if (result.done) break; compressed.push(result.value); } })(); await compressionWriter.write(encoder.encode(' world')); await compressionWriter.close(); await collectCompressed; var decompression = new DecompressionStream('gzip'), decompressionWriter = decompression.writable.getWriter(), decompressionReader = decompression.readable.getReader(), decoded = [], collectDecoded = (async function() { while (true) { var result = await decompressionReader.read(); if (result.done) break; decoded.push(result.value); } })(); var decodedBeforeClose = false; for (var index = 0; index < compressed.length; index++) { await decompressionWriter.write(compressed[index]); if (index === 0) { await new Promise(function(resolve) { setTimeout(resolve, 0); }); decodedBeforeClose = decoded.length > 0; } } await decompressionWriter.close(); await collectDecoded; var total = decoded.reduce(function(sum, chunk) { return sum + chunk.byteLength; }, 0), bytes = new Uint8Array(total), offset = 0; decoded.forEach(function(chunk) { bytes.set(chunk, offset); offset += chunk.byteLength; }); var cancelReason = new Error('stop'), cancelledWith, transform = new TransformStream({ cancel: function(reason) { cancelledWith = reason; } }), transformWriter = transform.writable.getWriter(); await transform.readable.cancel(cancelReason); var writerError = await transformWriter.closed.catch(function(error) { return error; }); return [firstCompressed.value.byteLength > 0, compressed.length > 1, decodedBeforeClose, new TextDecoder().decode(bytes), cancelledWith === cancelReason, writerError === cancelReason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_compression_incremental_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseCompressionIncremental = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseCompressionIncremental")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,true,true,"hello world",true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn tls_client_verifies_custom_ca_and_exchanges_encrypted_bytes() {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    use rustls::server::WebPkiClientVerifier;
    use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::Command;
    use std::sync::Arc;

    let certificate_dir = temp_registry("builtin_tls_certificate");
    let key_pem = certificate_dir.join("key.pem");
    let cert_pem = certificate_dir.join("cert.pem");
    let key_der = certificate_dir.join("key.der");
    let cert_der = certificate_dir.join("cert.der");
    let client_key_pem = certificate_dir.join("client-key.pem");
    let client_cert_pem = certificate_dir.join("client-cert.pem");
    let client_cert_der = certificate_dir.join("client-cert.der");
    assert!(Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature,keyEncipherment",
            "-addext",
            "extendedKeyUsage=serverAuth",
            "-keyout"
        ])
        .arg(&key_pem)
        .arg("-out")
        .arg(&cert_pem)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=thaw-client",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature",
            "-addext",
            "extendedKeyUsage=clientAuth",
            "-keyout",
        ])
        .arg(&client_key_pem)
        .arg("-out")
        .arg(&client_cert_pem)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"])
        .arg(&cert_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&cert_der)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"])
        .arg(&client_cert_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&client_cert_der)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("openssl")
        .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
        .arg(&key_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&key_der)
        .status()
        .unwrap()
        .success());
    let certificate_bytes = fs::read(&cert_der).unwrap();
    let certificate = CertificateDer::from(certificate_bytes.clone());
    let private_key = PrivatePkcs8KeyDer::from(fs::read(&key_der).unwrap()).into();
    let mut client_roots = RootCertStore::empty();
    client_roots
        .add(CertificateDer::from(fs::read(&client_cert_der).unwrap()))
        .unwrap();
    let client_verifier = WebPkiClientVerifier::builder(Arc::new(client_roots))
        .build()
        .unwrap();
    let mut config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(vec![certificate], private_key)
        .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        for index in 0..2 {
            let (socket, _) = listener.accept().unwrap();
            let connection = ServerConnection::new(Arc::clone(&config)).unwrap();
            let mut stream = StreamOwned::new(connection, socket);
            let mut request = Vec::new();
            stream.read_to_end(&mut request).unwrap();
            assert_eq!(
                request,
                if index == 0 {
                    b"ping".as_slice()
                } else {
                    b"insecure".as_slice()
                }
            );
            stream
                .write_all(if index == 0 {
                    b"pong".as_slice()
                } else {
                    b"accepted".as_slice()
                })
                .unwrap();
            stream.conn.send_close_notify();
            stream.flush().unwrap();
        }
    });
    let ca_hex = fs::read(&cert_pem)
        .unwrap()
        .iter()
        .fold(String::new(), |mut output, byte| {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
            output
        });
    let client_cert_hex =
        fs::read(&client_cert_pem)
            .unwrap()
            .iter()
            .fold(String::new(), |mut output, byte| {
                use std::fmt::Write as _;
                let _ = write!(output, "{byte:02x}");
                output
            });
    let client_key_hex =
        fs::read(&client_key_pem)
            .unwrap()
            .iter()
            .fold(String::new(), |mut output, byte| {
                use std::fmt::Write as _;
                let _ = write!(output, "{byte:02x}");
                output
            });

    let dir = temp_registry("builtin_tls");
    fs::write(dir.join("index.js"), "var tls = require('node:tls'); module.exports = async function (port, caHex, certHex, keyHex) { var events = []; var socket = new tls.TLSSocket(); socket.on('connect', function() { events.push('connect'); }); socket.on('secureConnect', function() { events.push('secureConnect'); }); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); }); socket.on('end', function() { events.push('end'); }); var closed = new Promise(function(resolve, reject) { socket.on('error', reject); socket.on('close', function(hadError) { events.push('close:' + hadError); resolve(); }); }); socket.connect({ host: '127.0.0.1', port: port, servername: 'localhost', ca: Buffer.from(caHex, 'hex'), cert: Buffer.from(certHex, 'hex'), key: Buffer.from(keyHex, 'hex'), ALPNProtocols: ['h2', 'http/1.1'] }); socket.end('ping'); await closed; var peer = socket.getPeerCertificate(), local = socket.getCertificate(); var bypassData = '', bypass = new tls.TLSSocket(); var bypassClosed = new Promise(function(resolve, reject) { bypass.on('error', reject); bypass.on('data', function(chunk) { bypassData += chunk.toString(); }); bypass.on('close', resolve); }); bypass.connect({ host: '127.0.0.1', port: port, servername: 'not-localhost', cert: Buffer.from(certHex, 'hex'), key: Buffer.from(keyHex, 'hex'), rejectUnauthorized: false }); bypass.end('insecure'); await bypassClosed; return [events, socket.authorized, socket.authorizationError, socket.encrypted, socket.alpnProtocol, socket.getProtocol(), socket.getCipher().version, socket.bytesWritten, socket.bytesRead, tls.getCiphers().length, tls.DEFAULT_MIN_VERSION, peer.raw.length > 0, local.raw.length > 0, /^(?:[0-9A-F]{2}:){31}[0-9A-F]{2}$/.test(peer.fingerprint256), peer.subject.CN, peer.issuer.CN, local.subject.CN, peer.valid_from.length > 0, peer.valid_to.length > 0, peer.serialNumber.length > 0, bypassData, bypass.authorized, bypass.authorizationError]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_tls_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTls = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseTls").unwrap();
    let arguments = CString::new(format!(
        "[{port},\"{ca_hex}\",\"{client_cert_hex}\",\"{client_key_hex}\"]"
    ))
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["connect","secureConnect","data:pong","end","close:false"],true,null,true,"h2","TLSv1.3","TLSv1.3",4,4,3,"TLSv1.2",true,true,true,"localhost","localhost","thaw-client",true,true,true,"accepted",false,"UNABLE_TO_VERIFY_LEAF_SIGNATURE"]"#
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    let _ = fs::remove_dir_all(&certificate_dir);
}

#[test]
fn tls_server_accepts_verified_clients_and_exchanges_encrypted_bytes() {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName};
    use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    let certificate_dir = temp_registry("builtin_tls_server_certificate");
    let key_pem = certificate_dir.join("key.pem");
    let cert_pem = certificate_dir.join("cert.pem");
    let cert_der = certificate_dir.join("cert.der");
    let client_key_pem = certificate_dir.join("client-key.pem");
    let client_cert_pem = certificate_dir.join("client-cert.pem");
    let client_key_der = certificate_dir.join("client-key.der");
    let client_cert_der = certificate_dir.join("client-cert.der");
    assert!(Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature,keyEncipherment",
            "-addext",
            "extendedKeyUsage=serverAuth",
            "-keyout",
        ])
        .arg(&key_pem)
        .arg("-out")
        .arg(&cert_pem)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=thaw-client",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature",
            "-addext",
            "extendedKeyUsage=clientAuth",
            "-keyout",
        ])
        .arg(&client_key_pem)
        .arg("-out")
        .arg(&client_cert_pem)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"])
        .arg(&cert_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&cert_der)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"])
        .arg(&client_cert_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&client_cert_der)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("openssl")
        .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
        .arg(&client_key_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&client_key_der)
        .status()
        .unwrap()
        .success());
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(fs::read(&cert_der).unwrap()))
        .unwrap();
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(
            vec![CertificateDer::from(fs::read(&client_cert_der).unwrap())],
            PrivatePkcs8KeyDer::from(fs::read(&client_key_der).unwrap()).into(),
        )
        .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec()];
    let config = Arc::new(config);
    let client = std::thread::spawn(move || {
        (0..3)
            .map(|index| {
                let socket = (0..50)
                    .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                        Ok(socket) => Some(socket),
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(10));
                            None
                        }
                    })
                    .expect("TLS server did not start");
                let connection = ClientConnection::new(
                    Arc::clone(&config),
                    ServerName::try_from("localhost".to_string()).unwrap(),
                )
                .unwrap();
                let mut stream = StreamOwned::new(connection, socket);
                stream.write_all(format!("ping{index}").as_bytes()).unwrap();
                stream.conn.send_close_notify();
                stream.flush().unwrap();
                let mut response = Vec::new();
                stream.read_to_end(&mut response).unwrap();
                response
            })
            .collect::<Vec<_>>()
    });
    let to_hex = |value: &[u8]| {
        value.iter().fold(String::new(), |mut output, byte| {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
            output
        })
    };
    let cert_hex = to_hex(&fs::read(&cert_pem).unwrap());
    let key_hex = to_hex(&fs::read(&key_pem).unwrap());
    let client_ca_hex = to_hex(&fs::read(&client_cert_pem).unwrap());
    let dir = temp_registry("builtin_tls_server");
    fs::write(dir.join("index.js"), "var tls = require('node:tls'); module.exports = async function(port, certHex, keyHex, clientCaHex) { var events = [], handled = 0, server; var closed = new Promise(function(resolve, reject) { server = tls.createServer({ cert: Buffer.from(certHex, 'hex'), key: Buffer.from(keyHex, 'hex'), ca: Buffer.from(clientCaHex, 'hex'), requestCert: true, rejectUnauthorized: true, ALPNProtocols: ['h2', 'http/1.1'] }, function(socket) { var peer = socket.getPeerCertificate(), local = socket.getCertificate(); events.push('secureConnection:' + socket.alpnProtocol + ':' + (peer.raw.length > 0) + ':' + (local.raw.length > 0)); socket.on('error', reject); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); handled++; socket.end('pong', function() { if (handled === 3) server.close(); }); }); }); server.on('listening', function() { events.push('listening'); }); server.on('error', reject); server.on('tlsClientError', reject); server.on('close', function() { events.push('close'); resolve(); }); server.listen(port, '127.0.0.1'); }); await closed; return [events, server.listening, server.address().port, server.connections]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_tls_server_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTlsServer = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseTlsServer").unwrap();
    let arguments = CString::new(format!(
        "[{port},\"{cert_hex}\",\"{key_hex}\",\"{client_ca_hex}\"]"
    ))
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(
            r#"[["listening","secureConnection:h2:true:true","data:ping0","secureConnection:h2:true:true","data:ping1","secureConnection:h2:true:true","data:ping2","close"],false,{port},0]"#
        )
    );
    assert_eq!(client.join().unwrap(), vec![b"pong", b"pong", b"pong"]);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    let _ = fs::remove_dir_all(&certificate_dir);
}

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
