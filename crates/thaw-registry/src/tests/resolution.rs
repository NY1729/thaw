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

    let nested: serde_json::Value = serde_json::from_str(
        r#"{"exports":{"./package.json":"./package.json",".":{"require":{"types":"./index.d.cts","default":"./index.cjs"},"import":{"types":"./index.d.ts","default":"./index.js"}}}}"#,
    )
    .unwrap();
    assert_eq!(
        package_export_target(&nested, None, &["types"]),
        Some("./index.d.cts")
    );
    assert_eq!(
        package_export_target(&nested, Some("package.json"), &["types"]),
        None
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
    fs::create_dir_all(package.join("dist/internal")).unwrap();
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
        "export { double } from '../internal/double-api';",
    )
    .unwrap();
    fs::write(
        package.join("dist/internal/double-api.d.ts"),
        "export declare function double(value: number): number;",
    )
    .unwrap();
    fs::write(
        package.join("dist/features/double.js"),
        "module.exports = { double: function(value) { return value * 2; } };",
    )
    .unwrap();

    let added = add_installed(&registry, &scratch.join("node_modules"), "feature-kit").unwrap();
    assert_eq!(added.resolved_version, "1.2.3");
    let subpath = resolve(&registry, "feature-kit/features/double").unwrap();
    assert!(subpath
        .dts_source
        .contains("declare function double(value: number): number"));
    assert!(subpath.bundle_js.unwrap().contains("value * 2"));
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_named_function_reexports() {
    let scratch = temp_registry("installed-dts-reexport-scratch");
    let registry = temp_registry("installed-dts-reexport-registry");
    let package = scratch.join("node_modules/parser-kit");
    fs::create_dir_all(package.join("dist")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"parser-kit","version":"1.0.0","types":"./dist/index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("dist/index.d.ts"),
        "export { parse, stringify as encode } from './public-api';\nexport { loop } from './cycle-a';\nexport * from './all';",
    )
    .unwrap();
    fs::write(
        package.join("dist/public-api.d.ts"),
        "export { parse } from './parse';\nexport { stringify } from './stringify';",
    )
    .unwrap();
    fs::write(
        package.join("dist/parse.d.ts"),
        "export declare function parse(value: string): any;",
    )
    .unwrap();
    fs::write(
        package.join("dist/stringify.d.ts"),
        "export declare function stringify(value: any): string;",
    )
    .unwrap();
    fs::write(
        package.join("dist/cycle-a.d.ts"),
        "export { loop } from './cycle-b';",
    )
    .unwrap();
    fs::write(
        package.join("dist/cycle-b.d.ts"),
        "export { loop } from './cycle-a';",
    )
    .unwrap();
    fs::write(
        package.join("dist/all.d.ts"),
        "export declare function decode(value: string): any;\nexport * from './all-cycle';",
    )
    .unwrap();
    fs::write(
        package.join("dist/all-cycle.d.ts"),
        "export * from './all';",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { parse: JSON.parse, encode: JSON.stringify };",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "parser-kit").unwrap();
    let declarations = resolve(&registry, "parser-kit").unwrap().dts_source;
    assert!(declarations.contains("declare function parse(value: string): any"));
    assert!(declarations.contains("declare function encode(value: any): string"));
    assert!(declarations.contains("declare function decode(value: string): any"));
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
