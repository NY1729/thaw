fn installed_artifact_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, files);
            } else {
                files.insert(path.strip_prefix(root).unwrap().to_path_buf(), fs::read(&path).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    collect(root, root, &mut files);
    files
}

#[test]
fn failed_package_update_preserves_every_previous_artifact() {
    let scratch = temp_registry("atomic-add-scratch");
    let registry = temp_registry("atomic-add-registry");
    let package = scratch.join("node_modules/atomic-kit");
    fs::create_dir_all(package.join("feature")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"atomic-kit","version":"1.0.0","types":"index.d.ts","main":"index.js","exports":{".":{"types":"./index.d.ts","require":"./index.js"},"./feature":{"types":"./feature/index.d.ts","require":"./feature/index.js"}}}"#,
    ).unwrap();
    fs::write(package.join("index.d.ts"), "export declare const value: number;").unwrap();
    fs::write(package.join("index.js"), "exports.value = 1;").unwrap();
    fs::write(package.join("feature/index.d.ts"), "export declare const feature: number;").unwrap();
    fs::write(package.join("feature/index.js"), "exports.feature = 1;").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "atomic-kit").unwrap();
    let installed = registry.join("atomic-kit");
    fs::write(installed.join("native.node"), b"old addon").unwrap();
    fs::write(installed.join("native-addon.json"), b"old metadata").unwrap();
    fs::write(installed.join("platform-executable"), b"old executable").unwrap();
    fs::create_dir(installed.join("native-dependencies")).unwrap();
    fs::write(installed.join("native-dependencies/old.so"), b"old dependency").unwrap();
    fs::write(installed.join("lock.json"), b"old lock").unwrap();
    let before = installed_artifact_snapshot(&installed);

    fs::write(
        package.join("package.json"),
        r#"{"name":"atomic-kit","version":"2.0.0","types":"index.d.ts","main":"index.js","exports":{".":{"types":"./index.d.ts","require":"./index.js"},"./feature":{"types":"./feature/missing.d.ts","require":"./feature/index.js"}}}"#,
    ).unwrap();
    fs::write(package.join("index.d.ts"), "export declare const value: string;").unwrap();
    fs::write(package.join("index.js"), "exports.value = 'new';").unwrap();
    let error = add_installed(&registry, &scratch.join("node_modules"), "atomic-kit").unwrap_err();
    assert!(error.contains("missing.d.ts"), "{error}");
    assert_eq!(installed_artifact_snapshot(&installed), before);

    fs::write(package.join("feature/missing.d.ts"), "export declare const feature: string;").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "atomic-kit").unwrap();
    assert_eq!(fs::read_to_string(installed.join("version.txt")).unwrap(), "2.0.0");
    assert!(fs::read_to_string(installed.join("package.d.ts")).unwrap().contains("value: string"));
    for stale in ["native.node", "native-addon.json", "platform-executable", "native-dependencies", "lock.json"] {
        assert!(!installed.join(stale).exists(), "stale {stale}");
    }
    assert!(installed.join("subpaths/feature/bundle.js.gz").is_file());
    let feature_before = installed_artifact_snapshot(&installed.join("subpaths/feature"));
    fs::remove_file(package.join("feature/missing.d.ts")).unwrap();
    let error = add_installed_subpath(
        &registry,
        &scratch.join("node_modules"),
        "atomic-kit/feature",
    ).unwrap_err();
    assert!(error.contains("missing.d.ts"), "{error}");
    assert_eq!(
        installed_artifact_snapshot(&installed.join("subpaths/feature")),
        feature_before,
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn failed_first_lazy_subpath_leaves_root_directory_unchanged() {
    let scratch = temp_registry("atomic-lazy-scratch");
    let registry = temp_registry("atomic-lazy-registry");
    let package = scratch.join("node_modules/lazy-kit");
    fs::create_dir_all(package.join("feature")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"lazy-kit","version":"1.0.0","types":"index.d.ts","main":"index.js","exports":{".":{"types":"./index.d.ts","require":"./index.js"},"./feature":{"types":"./feature/missing.d.ts","require":"./feature/index.js"}}}"#,
    ).unwrap();
    fs::write(package.join("index.d.ts"), "export declare const root: number;").unwrap();
    fs::write(package.join("index.js"), "exports.root = 1;").unwrap();
    fs::write(package.join("feature/index.js"), "exports.feature = 1;").unwrap();
    add_installed_root(&registry, &scratch.join("node_modules"), "lazy-kit").unwrap();
    let installed = registry.join("lazy-kit");
    let before = installed_artifact_snapshot(&installed);
    let error = add_installed_subpath(&registry, &scratch.join("node_modules"), "lazy-kit/feature").unwrap_err();
    assert!(error.contains("missing.d.ts"), "{error}");
    assert_eq!(installed_artifact_snapshot(&installed), before);
    assert!(!installed.join("subpaths").exists());
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn lazy_parent_refresh_keeps_materialized_child_export() {
    let scratch = temp_registry("atomic-nested-scratch");
    let registry = temp_registry("atomic-nested-registry");
    let package = scratch.join("node_modules/nested-kit");
    fs::create_dir_all(package.join("foo/bar")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"nested-kit","version":"1.0.0","types":"index.d.ts","main":"index.js","exports":{".":{"types":"./index.d.ts","require":"./index.js"},"./foo":{"types":"./foo/index.d.ts","require":"./foo/index.js"},"./foo/bar":{"types":"./foo/bar/index.d.ts","require":"./foo/bar/index.js"}}}"#,
    ).unwrap();
    fs::write(package.join("index.d.ts"), "export declare const root: number;").unwrap();
    fs::write(package.join("index.js"), "exports.root = 1;").unwrap();
    fs::write(package.join("foo/index.d.ts"), "export declare const parent: number;").unwrap();
    fs::write(package.join("foo/index.js"), "exports.parent = 1;").unwrap();
    fs::write(package.join("foo/bar/index.d.ts"), "export declare const child: string;").unwrap();
    fs::write(package.join("foo/bar/index.js"), "exports.child = 'old';").unwrap();
    let modules = scratch.join("node_modules");
    add_installed_root(&registry, &modules, "nested-kit").unwrap();
    add_installed_subpath(&registry, &modules, "nested-kit/foo/bar").unwrap();
    let installed = registry.join("nested-kit/subpaths/foo");
    let child_before = installed_artifact_snapshot(&installed.join("bar"));
    add_installed_subpath(&registry, &modules, "nested-kit/foo").unwrap();
    assert_eq!(installed_artifact_snapshot(&installed.join("bar")), child_before);
    fs::write(package.join("foo/index.d.ts"), "export declare const parent: boolean;").unwrap();
    fs::write(package.join("foo/index.js"), "exports.parent = true;").unwrap();
    add_installed_subpath(&registry, &modules, "nested-kit/foo").unwrap();
    assert!(fs::read_to_string(installed.join("package.d.ts")).unwrap().contains("parent: boolean"));
    assert_eq!(installed_artifact_snapshot(&installed.join("bar")), child_before);
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn lazy_subpath_versions_reproject_nested_instances_and_drop_replaced_dependencies() {
    let scratch = temp_registry("lazy-versions-scratch");
    let registry = temp_registry("lazy-versions-registry");
    let modules = scratch.join("node_modules");
    let package = modules.join("version-kit");
    let top_shared = modules.join("shared");
    let nested_shared = package.join("feature/node_modules/shared");
    fs::create_dir_all(&nested_shared).unwrap();
    fs::create_dir_all(&top_shared).unwrap();
    fs::write(package.join("package.json"), r#"{"name":"version-kit","version":"1.0.0","types":"index.d.ts","main":"index.js","exports":{".":{"types":"./index.d.ts","require":"./index.js"},"./feature":{"types":"./feature/index.d.ts","require":"./feature/index.js"}}}"#).unwrap();
    fs::write(package.join("index.d.ts"), "export declare const root: number;").unwrap();
    fs::write(package.join("index.js"), "exports.root = require('shared').value;").unwrap();
    fs::write(package.join("feature/index.d.ts"), "export declare const feature: number;").unwrap();
    fs::write(package.join("feature/index.js"), "exports.feature = require('shared').value;").unwrap();
    fs::write(top_shared.join("package.json"), r#"{"name":"shared","version":"1.0.0","main":"index.js"}"#).unwrap();
    fs::write(top_shared.join("index.js"), "exports.value = 1;").unwrap();
    fs::write(nested_shared.join("package.json"), r#"{"name":"shared","version":"2.0.0","main":"index.js"}"#).unwrap();
    fs::write(nested_shared.join("index.js"), "exports.value = 2;").unwrap();

    add_installed_root(&registry, &modules, "version-kit").unwrap();
    assert_eq!(resolve(&registry, "version-kit").unwrap().dependency_versions.unwrap().get("shared").map(String::as_str), Some("1.0.0"));
    add_installed_subpath(&registry, &modules, "version-kit/feature").unwrap();
    let root = resolve(&registry, "version-kit").unwrap().dependency_versions.unwrap();
    assert_eq!(root.get("shared").map(String::as_str), Some("1.0.0"));
    assert_eq!(root.get("version-kit/feature/node_modules/shared").map(String::as_str), Some("2.0.0"));
    let subpath = resolve(&registry, "version-kit/feature").unwrap().dependency_versions.unwrap();
    assert_eq!(subpath.get("shared").map(String::as_str), Some("2.0.0"));

    fs::write(package.join("feature/index.js"), "exports.feature = 3;").unwrap();
    add_installed_subpath(&registry, &modules, "version-kit/feature").unwrap();
    let root = resolve(&registry, "version-kit").unwrap().dependency_versions.unwrap();
    assert_eq!(root.get("shared").map(String::as_str), Some("1.0.0"));
    assert!(!root.contains_key("version-kit/feature/node_modules/shared"));
    assert!(resolve(&registry, "version-kit/feature").unwrap().dependency_versions.is_none());
    let before_conflict = installed_artifact_snapshot(&registry.join("version-kit"));
    fs::remove_dir_all(&nested_shared).unwrap();
    fs::write(top_shared.join("package.json"), r#"{"name":"shared","version":"9.0.0","main":"index.js"}"#).unwrap();
    fs::write(package.join("feature/index.js"), "exports.feature = require('shared').value;").unwrap();
    let error = add_installed_subpath(&registry, &modules, "version-kit/feature").unwrap_err();
    assert!(error.contains("changed between registered bundles"), "{error}");
    assert_eq!(installed_artifact_snapshot(&registry.join("version-kit")), before_conflict);
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn lazy_subpath_refuses_legacy_index_and_existing_writer_without_mutation() {
    let scratch = temp_registry("lazy-index-scratch");
    let registry = temp_registry("lazy-index-registry");
    let modules = scratch.join("node_modules");
    let package = modules.join("index-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"), r#"{"name":"index-kit","version":"1.0.0","types":"index.d.ts","main":"index.js","exports":{".":{"types":"./index.d.ts","require":"./index.js"},"./feature":{"types":"./index.d.ts","require":"./index.js"}}}"#).unwrap();
    fs::write(package.join("index.d.ts"), "export declare const value: number;").unwrap();
    fs::write(package.join("index.js"), "exports.value = 1;").unwrap();
    add_installed_root(&registry, &modules, "index-kit").unwrap();
    let installed = registry.join("index-kit");
    let before = installed_artifact_snapshot(&installed);
    let guard = PackageWriterGuard::acquire(&installed).unwrap();
    let error = add_installed_subpath(&registry, &modules, "index-kit/feature").unwrap_err();
    assert!(error.contains("writer"), "{error}");
    assert_eq!(installed_artifact_snapshot(&installed), before);
    drop(guard);
    fs::remove_file(installed.join("instances.json")).unwrap();
    let legacy = installed_artifact_snapshot(&installed);
    let error = add_installed_subpath(&registry, &modules, "index-kit/feature").unwrap_err();
    assert!(error.contains("re-register"), "{error}");
    assert_eq!(installed_artifact_snapshot(&installed), legacy);
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn indexed_exports_named_instances_json_and_lock_json_work_in_both_orders() {
    for mode in ["eager", "parent-first", "children-first"] {
        let scratch = temp_registry(&format!("indexed-name-scratch-{mode}"));
        let registry = temp_registry(&format!("indexed-name-registry-{mode}"));
        let modules = scratch.join("node_modules");
        let package = modules.join("name-kit");
        let shared = modules.join("shared");
        fs::create_dir_all(&package).unwrap();
        fs::create_dir_all(&shared).unwrap();
        fs::write(package.join("package.json"), r#"{"name":"name-kit","version":"1.0.0","types":"index.d.ts","main":"index.js","exports":{".":{"types":"./index.d.ts","require":"./index.js"},"./foo":{"types":"./foo.d.ts","require":"./foo.js"},"./foo/instances.json":{"types":"./instances.d.ts","require":"./instances.js"},"./foo/lock.json":{"types":"./lock.d.ts","require":"./lock.js"}}}"#).unwrap();
        fs::write(package.join("index.d.ts"), "export declare const root: number;").unwrap();
        fs::write(package.join("index.js"), "exports.root = 1;").unwrap();
        fs::write(package.join("foo.d.ts"), "export declare const foo: number;").unwrap();
        fs::write(package.join("foo.js"), "exports.foo = 1;").unwrap();
        fs::write(package.join("instances.d.ts"), "export declare const instances: number;").unwrap();
        fs::write(package.join("instances.js"), "exports.instances = require('shared').value;").unwrap();
        fs::write(package.join("lock.d.ts"), "export declare const lock: number;").unwrap();
        fs::write(package.join("lock.js"), "exports.lock = require('shared').value;").unwrap();
        fs::write(shared.join("package.json"), r#"{"name":"shared","version":"2.0.0","main":"index.js"}"#).unwrap();
        fs::write(shared.join("index.js"), "exports.value = 2;").unwrap();

        if mode == "eager" {
            add_installed(&registry, &modules, "name-kit").unwrap();
        } else {
            add_installed_root(&registry, &modules, "name-kit").unwrap();
            let order = if mode == "parent-first" {
                ["name-kit/foo", "name-kit/foo/instances.json", "name-kit/foo/lock.json"]
            } else {
                ["name-kit/foo/lock.json", "name-kit/foo/instances.json", "name-kit/foo"]
            };
            for specifier in order {
                add_installed_subpath(&registry, &modules, specifier).unwrap();
            }
        }
        let installed = registry.join("name-kit");
        assert!(installed.join("instances.json").is_file());
        assert!(installed.join("subpaths/foo/instances.json").is_dir());
        assert!(installed.join("subpaths/foo/lock.json").is_dir());
        assert!(resolve(&registry, "name-kit/foo").unwrap().bundle_js.is_some());
        for specifier in ["name-kit/foo/instances.json", "name-kit/foo/lock.json"] {
            let resolved = resolve(&registry, specifier).unwrap();
            assert_eq!(resolved.dependency_versions.unwrap().get("shared").map(String::as_str), Some("2.0.0"));
        }
        assert_eq!(resolve(&registry, "name-kit").unwrap().dependency_versions.unwrap().get("shared").map(String::as_str), Some("2.0.0"));
        let _ = fs::remove_dir_all(scratch);
        let _ = fs::remove_dir_all(registry);
    }
}

#[test]
fn legacy_subpath_lock_remains_readable_without_private_index() {
    let registry = temp_registry("legacy-subpath-lock");
    let subpath = registry.join("legacy-kit/subpaths/feature");
    fs::create_dir_all(&subpath).unwrap();
    fs::write(subpath.join("package.d.ts"), "export declare const value: number;").unwrap();
    fs::write(subpath.join("bundle.js"), "exports.value = 1;").unwrap();
    fs::write(subpath.join("lock.json"), r#"{"legacy-kit":"1.0.0","shared":"2.0.0"}"#).unwrap();
    assert_eq!(resolve(&registry, "legacy-kit/feature").unwrap().dependency_versions.unwrap().get("shared").map(String::as_str), Some("2.0.0"));
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn publication_failure_restores_the_previous_directory() {
    let registry = temp_registry("atomic-add-rollback");
    let destination = registry.join("package");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("version.txt"), "1.0.0").unwrap();
    let staged = registry.join("missing-stage");
    let backup = registry.join("previous");
    let error = publish_staged_package(&staged, &destination, &backup).unwrap_err();
    assert!(error.contains("failed to install"), "{error}");
    assert_eq!(fs::read_to_string(destination.join("version.txt")).unwrap(), "1.0.0");
    assert!(!backup.exists());
    let _ = fs::remove_dir_all(registry);
}

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
        package_export_target(&manifest, None, &["require", "node", "default"]),
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

    let ordered: serde_json::Value = serde_json::from_str(
        r#"{"exports":{".":{"default":"./default.js","node":"./node.js"}}}"#,
    )
    .unwrap();
    assert_eq!(
        package_export_target(&ordered, None, &["require", "node", "default"]),
        Some("./default.js")
    );

    let import_only: serde_json::Value =
        serde_json::from_str(r#"{"exports":{".":{"import":"./index.mjs"}}}"#).unwrap();
    assert_eq!(
        package_export_target(&import_only, None, &["require", "node", "default"]),
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
fn null_export_exclusion_blocks_wildcard_enumeration_and_lazy_registration() {
    let modules = temp_registry("null-export-modules");
    let registry = temp_registry("null-export-registry");
    let package = modules.join("pkg");
    fs::create_dir_all(package.join("dist/features/private")).unwrap();
    for name in ["public", "private/secret", "private/open", "default-first", "conditional", "array-blocked", "array-fallback"] {
        fs::write(package.join(format!("dist/features/{name}.d.ts")), "export declare const value: number;").unwrap();
        fs::write(package.join(format!("dist/features/{name}.cjs")), "exports.value = 1;").unwrap();
    }
    let source = r#"{"exports":{"./features/*":{"types":"./dist/features/*.d.ts","require":"./dist/features/*.cjs"},"./features/private/*":null,"./features/private/open":{"types":"./dist/features/private/open.d.ts","require":"./dist/features/private/open.cjs"},"./features/conditional":{"require":null,"default":"./dist/features/conditional.cjs","types":"./dist/features/conditional.d.ts"},"./features/default-first":{"default":"./dist/features/default-first.cjs","require":null,"types":"./dist/features/default-first.d.ts"},"./features/array-blocked":{"require":[null],"default":"./dist/features/array-blocked.cjs","types":"./dist/features/array-blocked.d.ts"},"./features/array-fallback":{"require":[null,{"require":"./dist/features/array-fallback.cjs"}],"types":"./dist/features/array-fallback.d.ts"}}}"#;
    fs::write(package.join("package.json"), source).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(source).unwrap();
    let names = package_subpath_exports(&manifest, &package).unwrap()
        .into_iter().map(|export| export.subpath).collect::<Vec<_>>();
    assert_eq!(names, vec!["features/array-fallback".to_string(), "features/default-first".to_string(), "features/private/open".to_string(), "features/public".to_string()]);
    assert_eq!(
        runtime_export_specifiers("pkg", &package).unwrap(),
        vec!["pkg/features/array-fallback".to_string(), "pkg/features/default-first".to_string(), "pkg/features/private/open".to_string(), "pkg/features/public".to_string()],
    );
    assert_eq!(package_subpath_runtime_target(&manifest, "features/private/secret", &["require", "node", "default"]), None);
    assert_eq!(package_subpath_runtime_target(&manifest, "features/conditional", &["require", "node", "default"]), None);
    assert_eq!(package_subpath_runtime_target(&manifest, "features/array-blocked", &["require", "node", "default"]), None);
    assert_eq!(package_subpath_runtime_target(&manifest, "features/array-fallback", &["require", "node", "default"]), Some("./dist/features/array-fallback.cjs".to_string()));
    assert_eq!(package_subpath_runtime_target(&manifest, "features/default-first", &["require", "node", "default"]), Some("./dist/features/default-first.cjs".to_string()));
    assert!(add_installed_subpath(&registry, &modules, "pkg/features/private/secret")
        .unwrap_err().contains("no export named"));
    let _ = fs::remove_dir_all(modules);
    let _ = fs::remove_dir_all(registry);
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
fn installed_root_defers_subpaths_until_requested() {
    let scratch = temp_registry("lazy-subpath-scratch");
    let registry = temp_registry("lazy-subpath-registry");
    let package = scratch.join("node_modules/feature-kit");
    fs::create_dir_all(package.join("features")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"feature-kit","types":"./index.d.ts","main":"./index.js","exports":{".":{"types":"./index.d.ts","require":"./index.js"},"./double":{"types":"./features/double.d.ts","require":"./features/double.js"}}}"#,
    )
    .unwrap();
    fs::write(package.join("index.d.ts"), "export declare const root: number;").unwrap();
    fs::write(package.join("index.js"), "exports.root = 1;").unwrap();
    fs::write(
        package.join("features/double.d.ts"),
        "export declare function double(value: number): number;",
    )
    .unwrap();
    fs::write(
        package.join("features/double.js"),
        "exports.double = function(value) { return value * 2; };",
    )
    .unwrap();

    add_installed_root(&registry, &scratch.join("node_modules"), "feature-kit").unwrap();
    assert!(resolve(&registry, "feature-kit").is_ok());
    assert!(registry.join("feature-kit/bundle.js.gz").is_file());
    assert!(!registry.join("feature-kit/bundle.js").exists());
    assert!(resolve(&registry, "feature-kit/double").is_err());

    add_installed_subpath(
        &registry,
        &scratch.join("node_modules"),
        "feature-kit/double",
    )
    .unwrap();
    let subpath = resolve(&registry, "feature-kit/double").unwrap();
    assert!(subpath.bundle_js.unwrap().contains("value * 2"));
    assert!(registry
        .join("feature-kit/subpaths/double/bundle.js.gz")
        .is_file());
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_root_uses_an_installed_definitely_typed_package() {
    let scratch = temp_registry("installed-types-scratch");
    let registry = temp_registry("installed-types-registry");
    let package = scratch.join("node_modules/plain-kit");
    let types = scratch.join("node_modules/@types/plain-kit");
    fs::create_dir_all(&package).unwrap();
    fs::create_dir_all(&types).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"plain-kit","main":"index.js"}"#,
    )
    .unwrap();
    fs::write(package.join("index.js"), "exports.double = value => value * 2;").unwrap();
    fs::write(
        types.join("package.json"),
        r#"{"name":"@types/plain-kit","types":"index.d.ts"}"#,
    )
    .unwrap();
    fs::write(
        types.join("index.d.ts"),
        "export declare function double(value: number): number;",
    )
    .unwrap();

    add_installed_root(&registry, &scratch.join("node_modules"), "plain-kit").unwrap();
    let resolved = resolve(&registry, "plain-kit").unwrap();
    assert!(resolved.dts_source.contains("function double(value: number)"));
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

/// The class sibling of `installed_package_inlines_named_function_
/// reexports` above -- real trigger: `@types/semver`'s own layout
/// (`index.d.ts`: `import SemVer = require("./classes/semver"); export
/// { SemVer };`, one file per class/function; `classes/semver.d.ts`:
/// `declare class SemVer {...} export = SemVer;`). Before this fix,
/// `import { SemVer } from "semver"` failed outright ("`semver` has no
/// export named `SemVer`") since `export_assignment_function_
/// declarations` (the existing fix for this exact barrel shape, but
/// only for a *function*-shaped `export = X;` target -- semver's own
/// `functions/valid.d.ts`) returned empty for a class-shaped target,
/// with nothing tried next.
#[test]
fn installed_package_inlines_a_class_reexported_through_an_import_equals_barrel() {
    let scratch = temp_registry("installed-dts-class-export-assignment-scratch");
    let registry = temp_registry("installed-dts-class-export-assignment-registry");
    let package = scratch.join("node_modules/version-kit");
    fs::create_dir_all(package.join("dist/classes")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"version-kit","version":"1.0.0","types":"./dist/index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("dist/index.d.ts"),
        "import Version = require('./classes/version');\nexport { Version };\n",
    )
    .unwrap();
    fs::write(
        package.join("dist/classes/version.d.ts"),
        "declare class Version {\n\
         \x20\x20\x20\x20constructor(raw: string);\n\
         \x20\x20\x20\x20raw: string;\n\
         \x20\x20\x20\x20major(): number;\n\
         }\n\
         export = Version;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "function Version(raw) { this.raw = raw; }\n\
         Version.prototype.major = function() { return Number(this.raw.split('.')[0]); };\n\
         module.exports = { Version: Version };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "version-kit").unwrap();
    let declarations = resolve(&registry, "version-kit").unwrap().dts_source;
    assert!(declarations.contains("declare class Version"));
    assert!(declarations.contains("major(): number"));
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

/// A two-level-deep `export * from './decorators'` -> `decorators/
/// index.d.ts` -> `export * from './expose.decorator'` wildcard chain
/// -- real example: `@types/class-transformer`'s own layout (`Expose`/
/// `Exclude`/`Transform`/`Type` each live one file deeper than the
/// package's own barrel, at `decorators/<name>.decorator.d.ts`). Before
/// this fix, none of these ever got inlined: `declaration_reexport_
/// path`'s own candidate-building used `Path::with_extension("d.ts")`,
/// which *replaces* whatever Rust considers the current "extension"
/// (everything after the final `.` in a file name) rather than
/// appending after it -- so resolving `./expose.decorator` produced
/// `expose.d.ts` (silently wrong, and nonexistent) instead of `expose.
/// decorator.d.ts`, and the whole wildcard chain resolved to nothing.
/// General, not class-transformer-specific: any `.d.ts` file name with
/// an embedded dot before its own suffix (`.interface.d.ts`, `.enum.
/// d.ts`, `.type.d.ts`, ... all real, common `@types/*` conventions)
/// hit the same silent miss.
#[test]
fn installed_package_inlines_a_two_level_wildcard_reexport_through_a_dotted_file_name() {
    let scratch = temp_registry("installed-dts-dotted-wildcard-scratch");
    let registry = temp_registry("installed-dts-dotted-wildcard-registry");
    let package = scratch.join("node_modules/deco-kit");
    fs::create_dir_all(package.join("dist/decorators")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"deco-kit","version":"1.0.0","types":"./dist/index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("dist/index.d.ts"),
        "export * from './decorators';\n",
    )
    .unwrap();
    fs::write(
        package.join("dist/decorators/index.d.ts"),
        "export * from './expose.decorator';\n",
    )
    .unwrap();
    fs::write(
        package.join("dist/decorators/expose.decorator.d.ts"),
        "export declare function Expose(options?: any): PropertyDecorator & ClassDecorator;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { Expose: function() { return function() {}; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "deco-kit").unwrap();
    let declarations = resolve(&registry, "deco-kit").unwrap().dts_source;
    assert!(declarations.contains("declare function Expose"));
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

/// A `declare const NAME: import("./sibling.js").SomeType<Args>;` --
/// real example: `tar`'s own `create.d.ts`, `export declare const
/// create: import("./make-command.js").TarCommand<Pack, PackSync>;`.
/// `make-command.js`/`.d.ts` isn't a `package.json` exports-map entry,
/// so nothing else would ever fetch it -- confirms it's resolved the
/// same way an ordinary `export * from "./x.js"` re-export already is,
/// its declarations inlined, and the const's own annotation rewritten
/// to a plain, re-parseable `TypeName<Args>` (no `import(...)` prefix
/// left in the flattened output for thaw-bridge to trip over).
#[test]
fn installed_package_inlines_a_cross_file_import_type_on_a_callable_const() {
    let scratch = temp_registry("installed-dts-import-type-scratch");
    let registry = temp_registry("installed-dts-import-type-registry");
    let package = scratch.join("node_modules/tar-like");
    fs::create_dir_all(package.join("dist")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"tar-like","version":"1.0.0","types":"./dist/index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("dist/index.d.ts"),
        "export * from './create';",
    )
    .unwrap();
    fs::write(
        package.join("dist/create.d.ts"),
        "export declare const create: import(\"./make-command.js\").TarCommand<string>;",
    )
    .unwrap();
    fs::write(
        package.join("dist/make-command.d.ts"),
        "export type TarCommand<T> = {\n    (opt: T): void;\n};",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { create: function () {} };",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "tar-like").unwrap();
    let declarations = resolve(&registry, "tar-like").unwrap().dts_source;
    assert!(
        declarations.contains("export declare const create: TarCommand<string>;"),
        "{declarations}"
    );
    assert!(
        !declarations.contains("import("),
        "the flattened output should have no remaining `import(...)` type reference: {declarations}"
    );
    assert!(
        declarations.contains("type TarCommand<T> = {\n    (opt: T): void;\n};"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

/// `export * from "bare-specifier"` re-exports from a genuinely
/// *different* installed package, not another file within the same one
/// -- real example: `@types/ramda`'s entire `index.d.ts` is just
/// `export * from "types-ramda";`, where `types-ramda` is its own
/// separate npm package whose real type entry point (per its own
/// `package.json`'s `types` field) isn't at its package root at all
/// (`./es/index.d.ts`, not `index.d.ts`). The old resolution only ever
/// guessed `<name>.d.ts`/`<name>/index.d.ts`, silently finding neither
/// and producing zero declarations -- `R.add` (and every other real
/// ramda function) built with no error at all, just "call to
/// undeclared function". Fixed by reusing `find_own_dts` (the same
/// `types`/`typings`-field resolution already used for a `--use`d
/// package's own entry point) for a bare-specifier target too.
#[test]
fn installed_package_follows_a_bare_reexport_to_another_packages_own_types_entry() {
    let scratch = temp_registry("installed-dts-cross-package-reexport-scratch");
    let registry = temp_registry("installed-dts-cross-package-reexport-registry");
    let node_modules = scratch.join("node_modules");
    let types_wrapper = node_modules.join("types-ramda-like");
    fs::create_dir_all(&types_wrapper).unwrap();
    fs::write(
        types_wrapper.join("package.json"),
        r#"{"name":"types-ramda-like","version":"1.0.0","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        types_wrapper.join("index.d.ts"),
        "export * from \"real-types-package\";",
    )
    .unwrap();
    fs::write(types_wrapper.join("index.js"), "module.exports = {};").unwrap();
    let real_types = node_modules.join("real-types-package");
    fs::create_dir_all(real_types.join("es")).unwrap();
    fs::write(
        real_types.join("package.json"),
        r#"{"name":"real-types-package","version":"1.0.0","types":"./es/index.d.ts"}"#,
    )
    .unwrap();
    fs::write(
        real_types.join("es/index.d.ts"),
        "export declare function add(a: number, b: number): number;",
    )
    .unwrap();

    add_installed(&registry, &node_modules, "types-ramda-like").unwrap();
    let declarations = resolve(&registry, "types-ramda-like").unwrap().dts_source;
    assert!(
        declarations.contains("declare function add(a: number, b: number): number"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

/// A package whose entire real API lives inside a TypeScript `declare
/// namespace X { ... }` block, exported wholesale via `export = X;` --
/// real example: `winston`'s complete `.d.ts` is exactly this shape
/// (`declare namespace winston { class Logger {...} function
/// createLogger(...): Logger; let level: string; } export = winston;`).
/// None of `thaw_bridge`'s own `.d.ts` extractors ever looked inside a
/// namespace's own body -- they only scan a module's direct top-level
/// items -- so a package shaped like this silently produced zero
/// declarations at all: no error, just every real function/class
/// reported as "undeclared" the moment anything tried to use one.
/// Fixed by hoisting the namespace's own body out to look like ordinary
/// top-level declarations. A namespace member that's itself a *hoisted
/// alias from another package* (winston's own `export import format =
/// logform.format;`, `export import transports = Transports;`) is a
/// separate, deeper follow-up fixed later -- see
/// `installed_package_resolves_a_namespace_hoisted_export_import_alias`
/// below.
#[test]
fn installed_package_hoists_an_export_equals_namespaces_own_members() {
    let scratch = temp_registry("installed-dts-export-equals-namespace-scratch");
    let registry = temp_registry("installed-dts-export-equals-namespace-registry");
    let package = scratch.join("node_modules/logger-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"logger-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "declare namespace loggerKit {\n\
             class Logger {\n\
                 log(message: string): void;\n\
             }\n\
             function createLogger(options?: object): Logger;\n\
             let level: string;\n\
         }\n\
         export = loggerKit;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "function createLogger() { return { log: function (m) { console.log(m); } }; }\n\
         module.exports = { createLogger: createLogger, level: 'info' };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "logger-kit").unwrap();
    let declarations = resolve(&registry, "logger-kit").unwrap().dts_source;
    // The raw, *unmodified* source text (namespace wrapper included)
    // always ends up in `declarations` verbatim -- checking a member's
    // text merely appears somewhere would pass even without hoisting,
    // since it's already sitting there, still nested. A real check
    // needs the member's own snippet to appear a *second* time, hoisted
    // out to its own top-level-shaped copy alongside the untouched
    // original.
    assert_eq!(
        declarations.matches("function createLogger(options?: object): Logger").count(),
        2,
        "{declarations}"
    );
    assert_eq!(
        declarations.matches("class Logger").count(),
        2,
        "{declarations}"
    );
    assert_eq!(
        declarations.matches("let level: string").count(),
        2,
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

/// The deeper follow-up `installed_package_hoists_an_export_equals_
/// namespaces_own_members` deferred: a namespace member that's itself
/// `export import NAME = BASE[.MEMBER];`, where `BASE` was imported at
/// the *entry file's own* top level (`import * as BASE from "..."`) --
/// real-world example: winston's own `export import format =
/// logform.format;` (qualified, `BASE.MEMBER`) and `export import
/// transports = Transports;` (bare, aliasing a whole namespace-imported
/// module). Before this fix, `named_import_targets` (which resolves
/// `BASE`) never recorded a `ImportSpecifier::Namespace` entry at all,
/// and the hoisting step only ever copied such a member's raw source
/// text -- meaningless to every one of this crate's `.d.ts` extractors,
/// which never resolve a qualified reference into another *package's*
/// file. Fixed in two parts: `named_import_targets` now records a
/// namespace import too, and `hoisted_export_equals_namespace_members`
/// resolves a namespace member shaped this way into real declaration
/// text instead of a raw copy -- a genuine function for the qualified
/// form, or a synthetic `declare namespace NAME { export { ... }; }`
/// re-export (the same shape `thaw_bridge::nested_namespace_members`
/// already parses back out for zod's `z.coerce.number(...)`) for the
/// bare whole-namespace-alias form.
#[test]
fn installed_package_resolves_a_namespace_hoisted_export_import_alias() {
    let scratch = temp_registry("installed-dts-namespace-export-import-alias-scratch");
    let registry = temp_registry("installed-dts-namespace-export-import-alias-registry");
    let node_modules = scratch.join("node_modules");

    let format_package = node_modules.join("log-format");
    fs::create_dir_all(&format_package).unwrap();
    fs::write(
        format_package.join("package.json"),
        r#"{"name":"log-format","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        format_package.join("index.d.ts"),
        "export declare function format(pattern: string): string;\n",
    )
    .unwrap();
    fs::write(
        format_package.join("index.js"),
        "module.exports = { format: function (p) { return p; } };\n",
    )
    .unwrap();

    let package = node_modules.join("logger-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"logger-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("transports.d.ts"),
        "import * as Transport from './transport';\n\
         declare namespace transportsNs {\n\
             interface ConsoleTransportInstance extends Transport.Base {\n\
                 name: string;\n\
                 new (): ConsoleTransportInstance;\n\
             }\n\
             interface Transports {\n\
                 Console: ConsoleTransportInstance;\n\
             }\n\
         }\n\
         declare const transportsNs: transportsNs.Transports;\n\
         export = transportsNs;\n",
    )
    .unwrap();
    fs::write(
        package.join("transport.d.ts"),
        "export declare class Base {\n    format?: string;\n}\n",
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import * as logFormat from 'log-format';\n\
         import * as Transports from './transports';\n\
         declare namespace loggerKit {\n\
             export import format = logFormat.format;\n\
             export import transports = Transports;\n\
             function createLogger(options?: object): object;\n\
         }\n\
         export = loggerKit;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { createLogger: function () { return {}; } };\n",
    )
    .unwrap();

    add_installed(&registry, &node_modules, "logger-kit").unwrap();
    let declarations = resolve(&registry, "logger-kit").unwrap().dts_source;
    assert!(
        declarations.contains("export declare function format(pattern: string): string"),
        "{declarations}"
    );
    assert!(
        declarations.contains("interface ConsoleTransportInstance"),
        "{declarations}"
    );
    assert!(
        declarations.contains("declare namespace transports {")
            && declarations.contains("export const Console: __thaw_support_"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

/// A class referenced only through a bare *type* use -- not a `declare
/// const x: Name; export = x;` value binding (`inline_import_equals_
/// value_type`'s own narrower shape), and not a local `export { Name }`
/// re-export either -- of an `import Name = require("./path")` (a TS
/// import-equals declaration to a *relative* path) still needs that
/// sibling file's declarations inlined, or the class's own members
/// (real trigger: nodemailer's `Mail`, with `sendMail`) are unreachable.
/// Real shape: `import Mail = require("./lib/mailer"); export type
/// Transporter<T = any, D = TransportOptions> = Mail<T, D>;
/// export declare function createTransport(): Transporter;` -- `Mail`
/// is named only inside a *generic* type alias's own right-hand side,
/// never directly exported or bound to a `declare const`.
#[test]
fn installed_package_inlines_a_class_referenced_only_through_a_type_alias() {
    let scratch = temp_registry("installed-dts-import-equals-referenced-type-scratch");
    let registry = temp_registry("installed-dts-import-equals-referenced-type-registry");
    let package = scratch.join("node_modules/mail-kit");
    fs::create_dir_all(package.join("lib")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"mail-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import Mail = require(\"./lib/mailer\");\n\
         export type Transporter<T = any, D = object> = Mail<T, D>;\n\
         export declare function createTransport(): Transporter;\n",
    )
    .unwrap();
    fs::write(
        package.join("lib/mailer.d.ts"),
        "declare class Mail<T = any, D = object> {\n\
         \x20\x20\x20\x20sendMail(opts: { to: string }): T;\n\
         }\n\
         export = Mail;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "function Mail() {}\n\
         Mail.prototype.sendMail = function (opts) { return opts.to; };\n\
         module.exports = { createTransport: function () { return new Mail(); } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "mail-kit").unwrap();
    let declarations = resolve(&registry, "mail-kit").unwrap().dts_source;
    assert!(
        declarations.contains("declare class Mail<")
            && declarations.contains("sendMail(opts: { to: string }): T;"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_default_reexports() {
    // `export { default as v4 } from './v4'` -- the common shape for a
    // package that splits one function per file and re-exports each one
    // under a name from a barrel (e.g. real-world `uuid`). The re-exported
    // name is never literally declared as `default` anywhere, so following
    // it means resolving `export default v4;` back to the real identifier
    // `v4` first and matching *that* against the plain (non-exported)
    // `declare function v4(...)` sitting next to it.
    let scratch = temp_registry("installed-dts-default-reexport-scratch");
    let registry = temp_registry("installed-dts-default-reexport-registry");
    let package = scratch.join("node_modules/id-kit");
    fs::create_dir_all(package.join("dist")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"id-kit","version":"1.0.0","types":"./dist/index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("dist/index.d.ts"),
        "export { default as v4 } from './v4';\nexport { default as validate } from './validate';",
    )
    .unwrap();
    fs::write(
        package.join("dist/v4.d.ts"),
        "declare function v4(options?: unknown): string;\nexport default v4;",
    )
    .unwrap();
    fs::write(
        package.join("dist/validate.d.ts"),
        // The less common inline shape (`export default function ...`),
        // covered alongside the `export default <ident>;` shape above.
        "export default function validate(value: unknown): boolean;",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { v4: function() { return 'id'; }, validate: function(x) { return true; } };",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "id-kit").unwrap();
    let declarations = resolve(&registry, "id-kit").unwrap().dts_source;
    assert!(declarations.contains("declare function v4(options?: unknown): string"));
    assert!(declarations.contains("function validate(value: unknown): boolean"));
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_a_default_class_reexport() {
    let scratch = temp_registry("installed-default-class-reexport-scratch");
    let registry = temp_registry("installed-default-class-reexport-registry");
    let package = scratch.join("node_modules/client");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"client","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "export { default } from './Client';\nexport { default as Client } from './Client';\n",
    )
    .unwrap();
    fs::write(
        package.join("Client.d.ts"),
        "declare class Client { readonly status: string; }\nexport default Client;\n",
    )
    .unwrap();
    fs::write(package.join("index.js"), "module.exports = class Client {};\n").unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "client").unwrap();
    let declarations = resolve(&registry, "client").unwrap().dts_source;
    assert!(declarations.contains("class Client"), "{declarations}");

    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn anonymous_default_declarations_survive_named_reexports() {
    // Unrun regression: both declarations lack an identifier in their own
    // files, but the entry exposes each under an ordinary public name.
    let root = temp_registry("anonymous-default-reexports");
    let entry = root.join("index.d.ts");
    fs::write(&entry,
        "export { default as make } from './make';\n\
         export { default as Widget } from './widget';\n\
         export { default as named } from './named';\n\
         export { default as null } from './make';\n",
    ).unwrap();
    fs::write(root.join("make.d.ts"),
        "export /* default boundary */ default\nfunction(value: string): number;",
    ).unwrap();
    fs::write(root.join("widget.d.ts"),
        "declare class Base { inherited(): number; }\n\
         export default /* class boundary */ class<T> extends Base { method(value: T): T; }",
    ).unwrap();
    fs::write(root.join("named.d.ts"),
        "export /* named boundary */ default\nfunction original(value: string): string;",
    ).unwrap();

    let flattened = dts_source_with_reexported_functions(
        &entry, &fs::read_to_string(&entry).unwrap(),
    ).unwrap();
    assert!(flattened.contains("function make(value: string): number"), "{flattened}");
    assert!(flattened.contains("class __thaw_default_class_"), "{flattened}");
    assert!(flattened.contains("as Widget"), "{flattened}");
    assert!(flattened.contains("<T> extends Base"), "{flattened}");
    assert!(flattened.contains("method(value: T): T"), "{flattened}");
    assert!(flattened.contains("inherited(): number"), "{flattened}");
    assert!(flattened.contains("function named(value: string): string"), "{flattened}");
    assert!(flattened.contains("as null"), "{flattened}");
    assert!(thaw_parser::parse_declarations(&flattened).is_ok(), "{flattened}");
    let async_normalized = reexported_default_declaration(
        "async /* retained */ function(value: string): Promise<string>;",
        "function", &entry, None, true, false,
    ).unwrap();
    assert!(async_normalized.contains("/* retained */ function __thaw_default_function_"), "{async_normalized}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn installed_package_inlines_a_default_reexported_literal_constant() {
    // `export { default as NIL } from './nil.js'` -- the exact same
    // barrel shape as `installed_package_inlines_default_reexports`
    // above, but the target file's own default export is a bare data
    // *constant*, not a function (real-world uuid's own `dist/nil.d.ts`:
    // `declare const _default: "00000000-0000-0000-0000-000000000000";
    // export default _default;`). Previously fell through every case in
    // the `name == "default"` resolution block (which only ever looked
    // for a matching `Decl::Fn`), so `NIL`/`MAX` never made it into the
    // flattened `.d.ts` at all -- "`uuid` has no export named `NIL`" at
    // build time, even though every function export from the same
    // package worked fine.
    let scratch = temp_registry("installed-dts-default-const-reexport-scratch");
    let registry = temp_registry("installed-dts-default-const-reexport-registry");
    let package = scratch.join("node_modules/id-kit");
    fs::create_dir_all(package.join("dist")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"id-kit","version":"1.0.0","types":"./dist/index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("dist/index.d.ts"),
        "export { default as NIL } from './nil';\nexport { default as MAX } from './max';\nexport { default as v4 } from './v4';",
    )
    .unwrap();
    fs::write(
        package.join("dist/nil.d.ts"),
        "declare const _default: \"00000000-0000-0000-0000-000000000000\";\nexport default _default;",
    )
    .unwrap();
    fs::write(
        package.join("dist/max.d.ts"),
        "declare const _default: \"ffffffff-ffff-ffff-ffff-ffffffffffff\";\nexport default _default;",
    )
    .unwrap();
    fs::write(
        package.join("dist/v4.d.ts"),
        "declare function v4(options?: unknown): string;\nexport default v4;",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { NIL: '00000000-0000-0000-0000-000000000000', MAX: 'ffffffff-ffff-ffff-ffff-ffffffffffff', v4: function() { return 'id'; } };",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "id-kit").unwrap();
    let declarations = resolve(&registry, "id-kit").unwrap().dts_source;
    assert!(
        declarations.contains("declare const NIL: \"00000000-0000-0000-0000-000000000000\""),
        "{declarations}"
    );
    assert!(
        declarations.contains("declare const MAX: \"ffffffff-ffff-ffff-ffff-ffffffffffff\""),
        "{declarations}"
    );
    // The sibling function-shaped export from the same barrel is
    // unaffected.
    assert!(declarations.contains("declare function v4(options?: unknown): string"));
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_named_import_reexports() {
    // `import { Name } from './path'; export { Name };` (ordinary ES
    // import + a *local* re-export, no `from` clause on the export
    // itself) -- real-world example: hono's `index.d.ts`, which does
    // exactly this for its `Hono` class (`import { Hono } from './hono';
    // export { Hono };`). Unlike `export { Name } from './path'` (a
    // single statement, already handled), the import and export are two
    // separate statements here, and `Name` can be a class/interface, not
    // just a function.
    let scratch = temp_registry("installed-dts-named-import-scratch");
    let registry = temp_registry("installed-dts-named-import-registry");
    let package = scratch.join("node_modules/web-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"web-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import { App } from './app';\nimport { route as routeFn } from './route';\nexport { App, routeFn as route };\nexport { Context } from './context';\n",
    )
    .unwrap();
    fs::write(
        package.join("app.d.ts"),
        "import { Base } from './base';\nexport declare class App<T = unknown> extends Base {\n    constructor(base?: string);\n    get(path: string): T;\n}\n",
    )
    .unwrap();
    fs::write(
        package.join("base.d.ts"),
        "declare class InternalBase {\n    request(path: string): Promise<Response>;\n}\nexport { InternalBase as Base };\n",
    )
    .unwrap();
    fs::write(
        package.join("route.d.ts"),
        "export declare function route(path: string): string;\n",
    )
    .unwrap();
    fs::write(
        package.join("context.d.ts"),
        "export declare class Context {\n    text(value: string): Response;\n}\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { App: function() {}, route: function(p) { return p; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "web-kit").unwrap();
    let declarations = resolve(&registry, "web-kit").unwrap().dts_source;
    assert!(
        declarations.contains("class App"),
        "{declarations}"
    );
    assert!(
        declarations.contains("class Base")
            && declarations.contains("request(path: string): Promise<Response>"),
        "{declarations}"
    );
    assert!(
        declarations.contains("function route(path: string): string"),
        "{declarations}"
    );
    assert!(declarations.contains("class Context"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_follows_named_import_reexports_through_export_star() {
    let scratch = temp_registry("installed-dts-transitive-named-import-scratch");
    let registry = temp_registry("installed-dts-transitive-named-import-registry");
    let package = scratch.join("node_modules/http-client");
    fs::create_dir_all(package.join("types")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"http-client","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(package.join("index.d.ts"), "export * from './types/index';\n").unwrap();
    fs::write(
        package.join("types/index.d.ts"),
        "import { close, request } from './api';\nexport { close, request };\n",
    )
    .unwrap();
    fs::write(
        package.join("types/api.d.ts"),
        "declare function close(): Promise<void>;\ndeclare function request(url: string): Promise<string>;\nexport { close, request };\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { request: async function(url) { return url; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "http-client").unwrap();
    let declarations = resolve(&registry, "http-client").unwrap().dts_source;
    assert!(
        declarations.contains("function close(): Promise<void>")
            && declarations.contains("function request(url: string): Promise<string>"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_types_through_wildcard_barrels() {
    let scratch = temp_registry("installed-dts-wildcard-types-scratch");
    let registry = temp_registry("installed-dts-wildcard-types-registry");
    let package = scratch.join("node_modules/service-kit");
    fs::create_dir_all(package.join("types")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"service-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(package.join("index.d.ts"), "export * from './types';\n").unwrap();
    fs::write(
        package.join("types/index.d.ts"),
        "export type * from './server';\n",
    )
    .unwrap();
    fs::write(
        package.join("types/server.d.ts"),
        "export interface Result { status: number; }\n\
         export type Handler = (value: string) => Result;\n\
         export class Server { run(handler: Handler): Promise<Result>; }\n",
    )
    .unwrap();
    fs::write(package.join("index.js"), "module.exports = {};\n").unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "service-kit").unwrap();
    let declarations = resolve(&registry, "service-kit").unwrap().dts_source;
    assert!(declarations.contains("interface Result"), "{declarations}");
    assert!(declarations.contains("type Handler"), "{declarations}");
    assert!(declarations.contains("class Server"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_type_reexports_keep_class_shape_without_value_export() {
    // Unrun regression: a type-only wildcard and a named type alias must
    // retain the instance declaration, while only the explicit value alias
    // remains constructible at runtime.
    let scratch = temp_registry("installed-dts-type-class-scratch");
    let registry = temp_registry("installed-dts-type-class-registry");
    let package = scratch.join("node_modules/type-class");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"type-class","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"),
        "export type * from './client';\nexport type { Client as TypeClient } from './client';\nexport { Client as LiveClient } from './client';\n").unwrap();
    fs::write(package.join("client.d.ts"),
        "export declare class Client { constructor(name: string); getName(): string; static create(): Client; }\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = { LiveClient: class {} };\n").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "type-class").unwrap();
    let declarations = resolve(&registry, "type-class").unwrap().dts_source;
    assert!(declarations.contains("export type { Client }"), "{declarations}");
    assert!(declarations.contains("export type { Client as TypeClient }"), "{declarations}");
    assert!(declarations.contains("export { Client as LiveClient }"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_type_reexports_follow_named_type_imports_and_barrels() {
    // Unrun regression: import type and export type must resolve through
    // the same source graph without creating a runtime class value.
    let scratch = temp_registry("installed-dts-type-import-scratch");
    let registry = temp_registry("installed-dts-type-import-registry");
    let package = scratch.join("node_modules/type-import");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"type-import","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"), "export type * from './barrel';\n").unwrap();
    fs::write(package.join("barrel.d.ts"),
        "import type { Client as Local } from './client';\nexport type { Local as PublicClient };\n").unwrap();
    fs::write(package.join("client.d.ts"),
        "export declare class Client { request(): string; }\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = {};\n").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "type-import").unwrap();
    let declarations = resolve(&registry, "type-import").unwrap().dts_source;
    assert!(declarations.contains("request(): string"), "{declarations}");
    assert!(declarations.contains("export type { Client as PublicClient }"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn imported_type_reexport_without_type_modifier_stays_type_only() {
    // Unrun regression: the import's type-only provenance must survive a
    // plain `export { Local }`, while the parallel value import remains live.
    let scratch = temp_registry("installed-dts-import-type-provenance-scratch");
    let registry = temp_registry("installed-dts-import-type-provenance-registry");
    let package = scratch.join("node_modules/import-type-provenance");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"import-type-provenance","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"),
        "export * from './types';\nexport { TypeClient as Alias } from './types';\nexport * from './values';\n").unwrap();
    fs::write(package.join("types.d.ts"),
        "import type { Client as Local } from './client';\nexport { Local };\nexport { Local as TypeClient };\n").unwrap();
    fs::write(package.join("values.d.ts"),
        "import { Client as Local } from './client';\nexport { Local as LiveClient };\n").unwrap();
    fs::write(package.join("client.d.ts"),
        "export declare class Client { method(): string; }\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = { LiveClient: class {} };\n").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "import-type-provenance").unwrap();
    let declarations = resolve(&registry, "import-type-provenance").unwrap().dts_source;
    assert!(declarations.contains("export type { Client as Local }"), "{declarations}");
    assert!(declarations.contains("export type { Client as TypeClient }"), "{declarations}");
    assert!(declarations.contains("export type { Client as Alias }"), "{declarations}");
    assert!(declarations.contains("export { Client as LiveClient }"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_wildcard_follows_ordinary_named_class_barrel() {
    // Unrun regression for exact named edge inside a wildcard barrel.
    let scratch = temp_registry("installed-dts-named-class-barrel-scratch");
    let registry = temp_registry("installed-dts-named-class-barrel-registry");
    let package = scratch.join("node_modules/named-class-barrel");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"named-class-barrel","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"), "export * from './barrel';\n").unwrap();
    fs::write(package.join("barrel.d.ts"), "export { Client as PublicClient } from './client';\n").unwrap();
    fs::write(package.join("client.d.ts"),
        "export declare class Client { request(): string; }\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = { PublicClient: class {} };\n").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "named-class-barrel").unwrap();
    let declarations = resolve(&registry, "named-class-barrel").unwrap().dts_source;
    assert!(declarations.contains("export { Client as PublicClient }"), "{declarations}");
    assert!(declarations.contains("request(): string"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_wildcard_keeps_imported_class_aliases_at_each_barrel() {
    // Unrun regression: the declaration is named Client, but a plain
    // export of an imported Local must expose Local, then PublicClient.
    let scratch = temp_registry("installed-dts-imported-class-alias-scratch");
    let registry = temp_registry("installed-dts-imported-class-alias-registry");
    let package = scratch.join("node_modules/imported-class-alias");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"imported-class-alias","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"), "export * from './barrel';\n").unwrap();
    fs::write(package.join("barrel.d.ts"),
        "import { Client as Local } from './client';\nexport { Local };\nexport { Local as PublicClient };\n").unwrap();
    fs::write(package.join("client.d.ts"),
        "export declare class Client { request(): string; }\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = { Local: class {}, PublicClient: class {} };\n").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "imported-class-alias").unwrap();
    let declarations = resolve(&registry, "imported-class-alias").unwrap().dts_source;
    assert!(declarations.contains("export { Client as Local }"), "{declarations}");
    assert!(declarations.contains("export { Client as PublicClient }"), "{declarations}");
    assert!(declarations.contains("request(): string"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn named_class_alias_keeps_imported_superclass_name() {
    // Unrun regression: Base is supporting type information, not another
    // declaration of the public Derived alias.
    let scratch = temp_registry("installed-dts-derived-alias-scratch");
    let registry = temp_registry("installed-dts-derived-alias-registry");
    let package = scratch.join("node_modules/derived-alias");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"derived-alias","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"),
        "export { Derived as PublicDerived, Derived as OtherDerived } from './derived';\nexport type { Derived as TypeDerived } from './derived';\n").unwrap();
    fs::write(package.join("derived.d.ts"),
        "import { Base as Parent } from './base';\nexport declare class Derived extends Parent { self(): Derived; }\n").unwrap();
    fs::write(package.join("base.d.ts"),
        "import { Grand as Super } from './grand';\nexport declare class Base extends Super { base(): string; self(): Base; key(): Base.Key; generic<Base>(value: Base): Base; }\nexport declare namespace Base { type Key = string; type Own = Base; type Wrap<Base> = Base; type Mapped = { [Base in keyof Base]: Base }; type Inferred = Base extends infer Base ? Base : Base; }\n").unwrap();
    fs::write(package.join("grand.d.ts"),
        "export declare class Grand { grand(): number; self(): Grand; }\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = { PublicDerived: class {} };\n").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "derived-alias").unwrap();
    let declarations = resolve(&registry, "derived-alias").unwrap().dts_source;
    assert!(declarations.contains("class Parent"), "{declarations}");
    assert!(declarations.contains("export type { Parent };"), "{declarations}");
    assert!(!declarations.contains("export declare class Parent"), "{declarations}");
    assert!(declarations.contains("namespace Parent"), "{declarations}");
    assert!(declarations.contains("self(): Parent"), "{declarations}");
    assert!(declarations.contains("key(): Parent.Key"), "{declarations}");
    assert!(declarations.contains("generic<Base>(value: Base): Base"), "{declarations}");
    assert!(declarations.contains("type Own = Parent"), "{declarations}");
    assert!(declarations.contains("type Wrap<Base> = Base"), "{declarations}");
    assert!(declarations.contains("[Base in keyof Parent]: Base"), "{declarations}");
    assert!(declarations.contains("Parent extends infer Base ? Base : Parent"), "{declarations}");
    assert!(declarations.contains("extends Parent"), "{declarations}");
    assert!(declarations.contains("class Parent extends Super"), "{declarations}");
    assert!(declarations.contains("class Super"), "{declarations}");
    assert!(declarations.contains("self(): Super"), "{declarations}");
    assert!(declarations.contains("self(): Derived"), "{declarations}");
    assert!(declarations.contains("class Derived"), "{declarations}");
    assert!(declarations.contains("export { Derived as PublicDerived }"), "{declarations}");
    assert!(declarations.contains("export { Derived as OtherDerived }"), "{declarations}");
    assert!(declarations.contains("export type { Derived as TypeDerived }"), "{declarations}");
    assert_eq!(declarations.matches("class Parent").count(), 1, "{declarations}");
    assert_eq!(declarations.matches("class Derived").count(), 1, "{declarations}");
    assert!(!declarations.contains("class Base"), "{declarations}");
    assert!(!declarations.contains("class PublicDerived { base()"), "{declarations}");
    assert!(!declarations.contains("class TypeDerived { base()"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn materialized_base_import_retains_other_specifiers() {
    // Unrun regression: materializing Core removes its import binding,
    // while the unrelated type and value imports keep their syntax.
    let source = "import Core, { type Shape, Value as Other } from './core';\nexport class Child extends Core { method(x: Shape): Other; }";
    let locals = std::collections::BTreeSet::from(["Core".to_string()]);
    let flattened = without_materialized_class_imports(source, &locals).unwrap();
    assert!(flattened.contains("import { type Shape, Value as Other } from './core'"), "{flattened}");
    assert!(!flattened.contains("import Core"), "{flattened}");
    assert!(thaw_parser::parse_declarations(&flattened).is_ok(), "{flattened}");
    let source = "import type { Base as Parent, Shape } from './core';";
    let locals = std::collections::BTreeSet::from(["Parent".to_string()]);
    let retained = without_materialized_class_imports(source, &locals).unwrap();
    assert!(retained.contains("import type { Shape } from './core'"), "{retained}");
    assert!(thaw_parser::parse_declarations(&retained).is_ok(), "{retained}");
}

#[test]
fn generated_declaration_labels_are_custom_and_keep_parse_fallbacks() {
    use thaw_parser::common::{FileName, Spanned};

    let origin = Path::new("/logical/package/index.d.ts");
    let source_name = FileName::Custom(
        format!("{} (unwrapped declaration entry)", origin.display()),
    );
    let source = "import Core, { type Shape } from './core';\nexport class Child extends Core {}";
    let (module, source_map) =
        thaw_parser::parse_declarations_with_source_map_named(source, source_name.clone()).unwrap();
    assert_eq!(
        source_map.lookup_char_pos(module.body[0].span().lo).file.name.as_ref(),
        &source_name
    );
    let locals = std::collections::BTreeSet::from(["Core".to_string()]);
    let retained = without_materialized_class_imports_named(source, &locals, source_name).unwrap();
    assert!(retained.contains("import { type Shape } from './core'"), "{retained}");
    assert!(without_materialized_class_imports_named(
        "import {", &locals, FileName::Custom("generated malformed declaration".into()),
    ).is_err());

    let builtin = FileName::Custom("node:stream (generated builtin declarations)".into());
    let (module, source_map) = thaw_parser::parse_declarations_with_source_map_named(
        "export declare class Readable {}", builtin.clone(),
    ).unwrap();
    assert_eq!(
        source_map.lookup_char_pos(module.body[0].span().lo).file.name.as_ref(),
        &builtin
    );
}

#[test]
fn type_wildcard_converts_inlined_class_value_alias() {
    // Unrun regression: a later type-only wildcard must not retain the
    // generated value export of an earlier class alias.
    let scratch = temp_registry("installed-dts-type-over-value-alias-scratch");
    let registry = temp_registry("installed-dts-type-over-value-alias-registry");
    let package = scratch.join("node_modules/type-over-value-alias");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"type-over-value-alias","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"), "export type * from './barrel';\n").unwrap();
    fs::write(package.join("barrel.d.ts"), "export { Client as PublicClient } from './client';\n").unwrap();
    fs::write(package.join("client.d.ts"), "export declare class Client { self(): Client; }\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = {};\n").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "type-over-value-alias").unwrap();
    let declarations = resolve(&registry, "type-over-value-alias").unwrap().dts_source;
    assert!(declarations.contains("export type { Client as PublicClient }"), "{declarations}");
    assert!(!declarations.contains("export { Client as PublicClient }"), "{declarations}");
    assert!(declarations.contains("self(): Client"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_type_namespace_reexport_preserves_qualified_class_shape() {
    // Unrun regression for `export type * as NS` through a second barrel.
    let scratch = temp_registry("installed-dts-type-namespace-scratch");
    let registry = temp_registry("installed-dts-type-namespace-registry");
    let package = scratch.join("node_modules/type-namespace");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"type-namespace","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"), "export type * from './barrel';\n").unwrap();
    fs::write(package.join("barrel.d.ts"), "export type * as Types from './client';\n").unwrap();
    fs::write(package.join("client.d.ts"),
        "export declare class Client { request(): string; }\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = {};\n").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "type-namespace").unwrap();
    let declarations = resolve(&registry, "type-namespace").unwrap().dts_source;
    assert!(declarations.contains("declare namespace Types"), "{declarations}");
    assert!(declarations.contains("export type { Types }"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_a_local_bare_declaration_reexported_under_a_reserved_word_alias() {
    // `declare function _enum(...)` (no `export` prefix at all) followed
    // by a separate, *same-file* `export { _enum as enum };` -- real-
    // world example: zod v4's own `schemas.d.cts`, which declares this
    // exact shape (two overloads) because `enum` is a reserved word and
    // can't be the function's own declared name. Neither the existing
    // `import_equals_targets` nor `named_import_targets` lookup covers
    // this: `_enum` isn't bound via any import at all, just declared
    // directly in the same file. Also covers the reserved-word alias
    // itself (`null`) that must retain a valid internal declaration and
    // an export alias, since `declare function null(...)` is invalid.
    let scratch = temp_registry("installed-dts-local-bare-reexport-scratch");
    let registry = temp_registry("installed-dts-local-bare-reexport-registry");
    let package = scratch.join("node_modules/case-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"case-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "export * from './schemas';\n",
    )
    .unwrap();
    fs::write(
        package.join("schemas.d.ts"),
        "declare function _enum(values: readonly string[]): string;\n\
         declare function _enum(entries: Record<string, string>): string;\n\
         export { _enum as enum };\n\
         declare function _null(): string;\n\
         export { _null as null };\n\
         export declare function string(): string;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { enum: function() { return 'enum'; }, string: function() { return 'string'; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "case-kit").unwrap();
    let declarations = resolve(&registry, "case-kit").unwrap().dts_source;
    assert!(
        declarations.contains("declare function enum(values: readonly string[]): string"),
        "{declarations}"
    );
    assert!(
        declarations.contains("declare function enum(entries: Record<string, string>): string"),
        "{declarations}"
    );
    assert!(
        !declarations.contains("function null("),
        "reserved-word alias must not become an invalid declaration: {declarations}"
    );
    assert!(declarations.contains("export { __thaw_public_6e756c6c_"), "{declarations}");
    assert!(declarations.contains(" as null };"), "{declarations}");
    assert!(
        declarations.contains("function string(): string"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_same_file_function_aliases() {
    let scratch = temp_registry("installed-dts-local-function-alias-scratch");
    let registry = temp_registry("installed-dts-local-function-alias-registry");
    let package = scratch.join("node_modules/client-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"client-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "declare function lookup(url: string): string;\n\
         export { lookup as io, lookup as connect, lookup as default };\n",
    )
    .unwrap();
    fs::write(package.join("index.js"), "module.exports = function() {};\n").unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "client-kit").unwrap();
    let declarations = resolve(&registry, "client-kit").unwrap().dts_source;
    assert!(
        declarations.contains("declare function io(url: string): string"),
        "{declarations}"
    );
    assert!(
        declarations.contains("declare function connect(url: string): string"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn declaration_alias_uses_ast_identifier_after_comments_and_newlines() {
    // Unrun regression: keyword text in comments and whitespace before a
    // binding cannot select the rename range.
    let scratch = temp_registry("installed-dts-ast-alias-scratch");
    let registry = temp_registry("installed-dts-ast-alias-registry");
    let package = scratch.join("node_modules/ast-alias");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"ast-alias","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"),
        "export { original as renamed, tabbed as newTabbed } from './impl';\n").unwrap();
    fs::write(package.join("impl.d.ts"),
        "export declare function /* class Wrong */\noriginal(value: string): string;\n\
         export declare function\ttabbed(value: number): number;\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = {};\n").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "ast-alias").unwrap();
    let declarations = resolve(&registry, "ast-alias").unwrap().dts_source;
    assert!(declarations.contains("renamed(value: string): string"), "{declarations}");
    assert!(declarations.contains("newTabbed(value: number): number"), "{declarations}");
    assert!(!declarations.contains("Wrong(value:"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_a_callable_const_reexported_from_another_file() {
    let scratch = temp_registry("installed-dts-reexported-callable-const-scratch");
    let registry = temp_registry("installed-dts-reexported-callable-const-registry");
    let package = scratch.join("node_modules/server-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"server-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "export { serve } from './server.js';\n",
    )
    .unwrap();
    fs::write(
        package.join("server.d.ts"),
        "declare const serve: (options: { port?: number }) => unknown;\nexport { serve };\n",
    )
    .unwrap();
    fs::write(package.join("index.js"), "module.exports = { serve() {} };\n").unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "server-kit").unwrap();
    let declarations = resolve(&registry, "server-kit").unwrap().dts_source;
    assert!(declarations.contains("const serve:"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_flattens_a_nested_namespace_reexport() {
    // `export * as NAME from "SOURCE";` -- a namespace re-export, real-
    // world example: zod v4's own re-export barrel, `export * as coerce
    // from "./coerce.cjs";` (alongside `export * as core from
    // "../core/index.cjs";`, `export * as iso from "./iso.cjs";`, and
    // `export * as locales from "../locales/index.cjs";`), reached here
    // one file below the entry point's own (transitively, through a
    // plain `export * from "./external";`) -- the shape
    // `collect_namespace_reexports` recurses through. Each of `coerce`'s
    // own functions stays inside `coerce`, separate from the package's
    // top-level `string`/`number` functions.
    let scratch = temp_registry("installed-dts-nested-namespace-scratch");
    let registry = temp_registry("installed-dts-nested-namespace-registry");
    let package = scratch.join("node_modules/case-kit5");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"case-kit5","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "export * from './external';\n",
    )
    .unwrap();
    fs::write(
        package.join("external.d.ts"),
        "export declare function string(): string;\n\
         export declare function number(): number;\n\
         export * as coerce from './coerce';\n",
    )
    .unwrap();
    fs::write(
        package.join("coerce.d.ts"),
        "export declare function string(): string;\n\
         export declare function number(): number;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { string: function() { return 'top'; }, number: function() { return 0; }, coerce: { string: function() { return 'coerced'; }, number: function() { return 1; } } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "case-kit5").unwrap();
    let declarations = resolve(&registry, "case-kit5").unwrap().dts_source;
    assert!(
        declarations.contains("function string(): string"),
        "{declarations}"
    );
    assert!(
        declarations.contains("function number(): number"),
        "{declarations}"
    );
    assert!(
        declarations.contains("declare namespace coerce {"),
        "{declarations}"
    );
    let coerce = declarations.split("declare namespace coerce {").nth(1).unwrap();
    assert!(coerce.contains("export function string(): string"), "{declarations}");
    assert!(coerce.contains("export function number(): number"), "{declarations}");
    assert!(!declarations.contains("__thaw_ns_coerce_"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_value_namespace_reexports_keep_classes_types_and_local_function_types() {
    // Unrun regression: same bare member names in two value namespaces must
    // retain separate type and constructor identities. Type-only and class-
    // only namespaces must not disappear when the function set is empty.
    let scratch = temp_registry("installed-value-namespace-shapes-scratch");
    let registry = temp_registry("installed-value-namespace-shapes-registry");
    let package = scratch.join("node_modules/shape-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"shape-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"),
        "export * as left from './left';\nexport * as right from './right';\nexport * as types from './types';\nexport * as classes from './classes';\nexport * as aliased from './aliases';\n").unwrap();
    fs::write(package.join("left.d.ts"),
        "export interface Options { label: string; }\nexport declare class Client { constructor(options: Options); label(): string; }\nexport declare function make(options: Options): Client;\nexport interface Callable { (options: Options): Client; }\nexport declare const create: Callable;\n").unwrap();
    fs::write(package.join("right.d.ts"),
        "export interface Options { count: number; }\nexport declare class Client { constructor(options: Options); count(): number; }\nexport declare function make(options: Options): Client;\nexport interface Callable { (options: Options): Client; }\nexport declare const create: Callable;\n").unwrap();
    fs::write(package.join("types.d.ts"), "export interface Options { flag: boolean; }\n").unwrap();
    fs::write(package.join("classes.d.ts"), "export declare class Client { constructor(); }\n").unwrap();
    fs::write(package.join("aliases.d.ts"), "export { Hidden as PublicClient } from './aliased';\n").unwrap();
    fs::write(package.join("aliased.d.ts"), "export declare class Hidden { constructor(); }\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = {};\n").unwrap();
    add_installed(&registry, &scratch.join("node_modules"), "shape-kit").unwrap();
    let declarations = resolve(&registry, "shape-kit").unwrap().dts_source;
    for alias in ["left", "right", "types", "classes", "aliased"] {
        assert!(declarations.contains(&format!("declare namespace {alias} {{")), "{declarations}");
    }
    for alias in ["left", "right"] {
        let body = declarations.split(&format!("declare namespace {alias} {{")).nth(1)
            .unwrap().split("\n}\n").next().unwrap();
        assert!(body.contains("export interface Options"), "{declarations}");
        assert!(body.contains("export class Client"), "{declarations}");
        assert!(body.contains("export function make(options: Options): Client"), "{declarations}");
        assert!(body.contains("export const create: Callable"), "{declarations}");
        assert_eq!(body.matches("export interface Callable").count(), 1, "{declarations}");
    }
    assert!(!declarations.contains("__thaw_ns_"), "{declarations}");
    let aliased = declarations.split("declare namespace aliased {").nth(1).unwrap();
    assert!(aliased.contains("class Hidden"), "{declarations}");
    assert!(aliased.contains("export { Hidden as PublicClient }"), "{declarations}");
    assert!(!aliased.contains("export class Hidden"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_carries_a_self_referential_namespace_alias_from_a_transitively_reexported_file() {
    // `import * as z from "SOURCE"; export { z }; export default z;` --
    // `thaw_bridge::self_referential_namespace_aliases`'s own shape, but
    // declared one file *below* the package's own entry point (reached
    // only transitively through a plain `export * from "./external";`),
    // not in the entry point itself. Real example: zod v3's own
    // `lib/index.d.ts` (`index.d.ts`, the package's entry point, is just
    // `export * from "./lib";`) -- unlike zod v4, whose entry point
    // declares this alias directly and so needed no special handling.
    // Without carrying this snippet into the flattened output, `import {
    // z } from "zod"` failed outright ("no export named `z`") even though
    // `z`'s own methods (`object`, `string`, ...) were all individually
    // reachable by their own bare names.
    let scratch = temp_registry("installed-dts-self-referential-alias-scratch");
    let registry = temp_registry("installed-dts-self-referential-alias-registry");
    let package = scratch.join("node_modules/case-kit6");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"case-kit6","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(package.join("index.d.ts"), "export * from './external';\n").unwrap();
    fs::write(
        package.join("external.d.ts"),
        "import * as z from './z';\n\
         export declare function greet(): string;\n\
         export { z };\n\
         export default z;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { greet: function() { return 'hi'; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "case-kit6").unwrap();
    let declarations = resolve(&registry, "case-kit6").unwrap().dts_source;
    assert!(
        declarations.contains("import * as z from './z';") || declarations.contains("import * as z from \"./z\";"),
        "{declarations}"
    );
    assert!(declarations.contains("export { z };"), "{declarations}");
    assert!(declarations.contains("export default z;"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_a_namespace_import_reexported_via_export_assignment() {
    // `import * as X from "./y"; export = X;` -- the whole entry file's
    // declared shape *is* another file's namespace, wholesale, with
    // nothing declared locally at all. Real example: bcryptjs's own
    // `umd/index.d.ts`: `import * as bcrypt from "./types.js"; export =
    // bcrypt; export as namespace bcrypt;` -- every one of bcryptjs's
    // actual functions (`hashSync`, `compareSync`, ...) lives in the
    // sibling `types.d.ts`, never otherwise reachable, since
    // thaw-registry discards every individual `.d.ts` source file
    // except the one flattened `package.d.ts` it writes out. Without
    // this, the flattened output was just those three lines -- no
    // functions, nothing -- and every call against the package failed
    // to build ("call to unknown function").
    let scratch = temp_registry("installed-dts-export-assignment-namespace-scratch");
    let registry = temp_registry("installed-dts-export-assignment-namespace-registry");
    let package = scratch.join("node_modules/case-kit7");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"case-kit7","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import * as caseKit from './impl.js';\n\
         export = caseKit;\n\
         export as namespace caseKit;\n",
    )
    .unwrap();
    fs::write(
        package.join("impl.d.ts"),
        "export declare function greet(name: string): string;\n\
         export interface Options { loud?: boolean; }\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { greet: function(name) { return 'hi ' + name; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "case-kit7").unwrap();
    let declarations = resolve(&registry, "case-kit7").unwrap().dts_source;
    assert!(
        declarations.contains("export declare function greet(name: string): string;"),
        "{declarations}"
    );
    assert!(
        declarations.contains("export interface Options { loud?: boolean; }")
            || declarations.contains("export interface Options {\n  loud?: boolean;\n}"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_a_default_imported_class_used_as_an_extends_base() {
    // A locally declared class whose `extends` clause names a plain
    // default-imported base (`import CoreClass from "./core"; export
    // declare class Derived extends CoreClass { ... }`) -- the base
    // class's own declaration lives entirely in another file
    // thaw-registry otherwise discards. Real example: ajv's own entry
    // `.d.ts`: `import AjvCore from "./core"; export declare class Ajv
    // extends AjvCore { _addVocabularies(): void; ... }`, `./core.d.ts`
    // itself just `export default class Ajv { compile(...): ...;
    // validate(...): ...; ... }` -- essentially ajv's entire real API.
    // Without inlining it, only the 3 methods the entry file adds
    // directly were ever reachable; every call against the rest failed
    // outright ("call to undeclared function").
    let scratch = temp_registry("installed-dts-default-import-extends-scratch");
    let registry = temp_registry("installed-dts-default-import-extends-registry");
    let package = scratch.join("node_modules/case-kit8");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"case-kit8","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import CoreThing from './core.js';\n\
         export declare class Thing extends CoreThing {\n\
         \x20\x20extra(): string;\n\
         }\n\
         export declare class Sibling extends CoreThing { sibling(): boolean; }\n\
         export default Thing;\n",
    )
    .unwrap();
    fs::write(
        package.join("core.d.ts"),
         "export default class Thing {\n\
         \x20\x20base(): string;\n\
         \x20\x20self(): Thing;\n\
         }\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = function Thing() {\n\
         \x20\x20this.base = function() { return 'base'; };\n\
         \x20\x20this.extra = function() { return 'extra'; };\n\
         };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "case-kit8").unwrap();
    let declarations = resolve(&registry, "case-kit8").unwrap().dts_source;
    assert!(
        declarations.contains("class Thing extends CoreThing"),
        "{declarations}"
    );
    assert!(
        declarations.contains("class CoreThing") && declarations.contains("base(): string;"),
        "{declarations}"
    );
    assert!(declarations.contains("self(): CoreThing"), "{declarations}");
    assert!(declarations.contains("class Sibling extends CoreThing"), "{declarations}");
    assert_eq!(declarations.matches("class CoreThing").count(), 1, "{declarations}");
    assert!(declarations.contains("export type { CoreThing };"), "{declarations}");
    assert!(!declarations.contains("export declare class CoreThing"), "{declarations}");
    assert!(!declarations.contains("class Thing {"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_a_cross_file_class_and_namespace_merge() {
    // A class re-exported by name from a *different* file can be
    // "merged" with a same-named `declare namespace X { ... }` block in
    // that same file -- a common real pattern for attaching static
    // types alongside a class (real example: `minipass`'s own `export
    // declare class Minipass<...> { ... }` merged with `export declare
    // namespace Minipass { export interface Events<T> { ...}; export
    // type Options<T> = ...; ... }`). `@isaacs/fs-minipass`'s own entry
    // file does `import { Minipass } from 'minipass'; ...
    // Minipass.Events<...>` -- `reexported_class_or_interface_
    // declarations_inner` used to only look for a `Decl::Class`/
    // `Decl::TsInterface` matching the imported name, silently
    // dropping the sibling namespace half of the merge entirely.
    // `Minipass.Events`/`.Options`/etc. were then all unresolvable in
    // the dependent package's own flattened `.d.ts`, which (via a
    // generic `.on<Event extends keyof Events>(ev: Event, handler:
    // (...args: Events[Event]) => any)` method) widened every real
    // callback's inferred parameter list down to nothing, rejecting any
    // callback that actually took an argument.
    let scratch = temp_registry("installed-dts-namespace-merge-scratch");
    let registry = temp_registry("installed-dts-namespace-merge-registry");
    let package = scratch.join("node_modules/case-kit10");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"case-kit10","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import { Base } from './base.js';\n\
         export declare class Derived extends Base<string> {\n\
         \x20\x20extra(): string;\n\
         }\n",
    )
    .unwrap();
    fs::write(
        package.join("base.d.ts"),
        "export declare class Base<T> {\n\
         \x20\x20core(): string;\n\
         }\n\
         export declare namespace Base {\n\
         \x20\x20export interface Events<T> {\n\
         \x20\x20\x20\x20data: [chunk: T];\n\
         \x20\x20}\n\
         }\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = function Derived() {\n\
         \x20\x20this.core = function() { return 'core'; };\n\
         \x20\x20this.extra = function() { return 'extra'; };\n\
         };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "case-kit10").unwrap();
    let declarations = resolve(&registry, "case-kit10").unwrap().dts_source;
    assert!(
        declarations.contains("class Derived extends Base"),
        "{declarations}"
    );
    assert!(
        declarations.contains("class Base") && declarations.contains("core(): string;"),
        "{declarations}"
    );
    assert!(
        declarations.contains("namespace Base") && declarations.contains("interface Events"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_a_node_builtin_class_used_as_a_namespace_qualified_extends_base() {
    // The namespace-qualified counterpart to the default-import case
    // above (`installed_package_inlines_a_default_imported_class_used_
    // as_an_extends_base`): a locally declared class whose `extends`
    // clause names a *namespace-imported Node builtin's* class
    // (`import * as stream from "stream"; export declare class Parser
    // extends stream.Transform { ... }`) -- the base class's own
    // declaration doesn't live in a file thaw-registry ever fetches at
    // all; it's one of thaw's own synthetic ambient declarations for a
    // Node builtin module. Real example: csv-parse's own `Parser
    // extends stream.Transform`, whose consumer needs `.read()` --
    // declared only on `stream`'s own `Readable` (`Transform`'s own
    // ancestor), not `Transform` itself. Without inlining `Transform`
    // (and transitively `Duplex`/`Readable`, following the *builtin's
    // own* `extends` chain), every one of those inherited methods failed
    // to compile ("call to unknown function").
    let scratch = temp_registry("installed-dts-namespace-builtin-extends-scratch");
    let registry = temp_registry("installed-dts-namespace-builtin-extends-registry");
    let package = scratch.join("node_modules/stream-kit9");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"stream-kit9","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import * as stream from 'stream';\n\
         export declare class Parser extends stream.Transform {\n\
         \x20\x20constructor(options: any);\n\
         }\n\
         export declare function parse(options: any): Parser;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { parse: function(options) { return {}; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "stream-kit9").unwrap();
    let declarations = resolve(&registry, "stream-kit9").unwrap().dts_source;
    assert!(
        declarations.contains("class Transform extends Duplex"),
        "{declarations}"
    );
    assert!(
        declarations.contains("class Duplex extends Readable"),
        "{declarations}"
    );
    assert!(
        declarations.contains("class Readable") && declarations.contains("read(size?: number): Json"),
        "{declarations}"
    );
    // `Parser`'s own `extends stream.Transform` clause is untouched --
    // thaw-bridge's `parse_dts_classes` resolves the actual member
    // inheritance from here (covered directly, without needing a real
    // registry package, by `expands_members_inherited_through_a_
    // namespace_qualified_extends` in thaw-bridge's own test suite).
    assert!(
        declarations.contains("class Parser extends stream.Transform"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_import_equals_reexports() {
    // `import Name = require("./path")` (a TS import-equals declaration)
    // followed by a *local* `export { Name as exported };` (no `from`
    // clause -- `Name` is already a value bound earlier in the same
    // file) -- real-world example: semver's `index.d.ts`, which imports
    // one function per file this way and re-exports every one of them
    // together. `Name` resolves through its own file's `export = X;` to
    // the identifier actually declared there, one file per function, so
    // (unlike the `export { default as x } from './x'` shape) each
    // specifier in the same `export { ... }` can resolve to a different
    // file.
    let scratch = temp_registry("installed-dts-import-equals-scratch");
    let registry = temp_registry("installed-dts-import-equals-registry");
    let package = scratch.join("node_modules/ver-kit");
    fs::create_dir_all(package.join("functions")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"ver-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import verValid = require('./functions/valid');\n\
         import verMajor = require('./functions/major');\n\
         export { verValid as valid, verMajor as major };\n",
    )
    .unwrap();
    fs::write(
        package.join("functions/valid.d.ts"),
        "declare function valid(version: string): string | null;\nexport = valid;\n",
    )
    .unwrap();
    fs::write(
        package.join("functions/major.d.ts"),
        "declare function major(version: string): number;\nexport = major;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { valid: function(v) { return v; }, major: function(v) { return 1; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "ver-kit").unwrap();
    let declarations = resolve(&registry, "ver-kit").unwrap().dts_source;
    assert!(
        declarations.contains("declare function valid(version: string): string | null"),
        "{declarations}"
    );
    assert!(
        declarations.contains("function major(version: string): number"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_export_import_member_reexports() {
    // `export import NAME = BASE.MEMBER;` -- a TS import-equals
    // declaration whose module reference is a qualified *entity name* (a
    // property access into an already-imported value), not a
    // `require(...)` call (the shape the test above covers). Real
    // example: uuid@8.3.2's real `.d.ts` (via `@types/uuid`), `import
    // uuid from "./index.js"; export import v1 = uuid.v1; export import
    // validate = uuid.validate; ...`. `validate`'s own declared type
    // (`export const validate: validate;`) is itself a *local, unexported*
    // type alias (`type validate = (uuid: string) => boolean;`) in
    // `./index.js`'s own `.d.ts` -- not an interface, not a direct
    // function type on the const itself.
    let scratch = temp_registry("installed-dts-export-import-member-scratch");
    let registry = temp_registry("installed-dts-export-import-member-registry");
    let package = scratch.join("node_modules/uuid-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"uuid-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import uuid from './impl';\n\
         export import validate = uuid.validate;\n",
    )
    .unwrap();
    fs::write(
        package.join("impl.d.ts"),
        "export {};\n\
         type validate = (uuid: string) => boolean;\n\
         export const validate: validate;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { validate: function(id) { return id.length === 36; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "uuid-kit").unwrap();
    let declarations = resolve(&registry, "uuid-kit").unwrap().dts_source;
    assert!(
        declarations.contains("export const validate: validate;"),
        "{declarations}"
    );
    assert!(
        declarations.contains("type validate = (uuid: string) => boolean;"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_the_type_an_import_equals_value_is_declared_with() {
    // `import Name = require("./path")` used as a *type* reference
    // (`declare const x: Name;`), not the value re-export
    // `installed_package_inlines_import_equals_reexports` already
    // covers -- real-world example: mime's `import Mime =
    // require("./Mime"); declare const mime: Mime; export = mime;`,
    // `Mime` itself an ambient class declared in a wholly different
    // file. Without following that reference, `Mime`'s class body
    // (needed to make `mime.getType(...)` classifiable at all) never
    // reaches the flattened `.d.ts`.
    let scratch = temp_registry("installed-dts-import-equals-type-scratch");
    let registry = temp_registry("installed-dts-import-equals-type-registry");
    let package = scratch.join("node_modules/type-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"type-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import Thing = require('./Thing');\n\
         declare const thing: Thing;\n\
         export = thing;\n",
    )
    .unwrap();
    fs::write(
        package.join("Thing.d.ts"),
        "declare class Thing {\n    label(): string;\n}\n\nexport = Thing;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { label: function() { return 'thing'; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "type-kit").unwrap();
    let declarations = resolve(&registry, "type-kit").unwrap().dts_source;
    assert!(
        declarations.contains("declare class Thing"),
        "{declarations}"
    );
    assert!(declarations.contains("label(): string"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_triple_slash_referenced_declarations() {
    // `/// <reference path="..." />` -- the classic DefinitelyTyped-style
    // split for a package whose real API is spread across many files
    // (real-world example: `@types/lodash`'s `index.d.ts` referencing a
    // dozen files under `common/`). A referenced file's own module
    // augmentation targeting the entry file itself (`declare module
    // "../index" { interface LoDashStatic { ... } }`) should land inside
    // the *entry file's own* exported namespace (`declare namespace
    // <name> { ... }`, `<name>` from `export as namespace <name>;`) once
    // inlined, since that's the namespace `export = <const>` actually
    // exposes; an augmentation aimed at some other module is kept as its
    // own (unresolved but harmless) `declare module "..." { ... }`.
    let scratch = temp_registry("installed-dts-triple-slash-scratch");
    let registry = temp_registry("installed-dts-triple-slash-registry");
    let package = scratch.join("node_modules/stat-kit");
    fs::create_dir_all(package.join("common")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"stat-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "// Copyright holder\n\
         /* License notice\n\
            continues here. */\n\
         /// <reference path=\"./common/array.d.ts\" />\n\
         export = _;\n\
         export as namespace _;\n\
         declare const _: _.StatStatic;\n\
         declare namespace _ {\n\
             interface StatStatic { options(value: Options): Tag; }\n\
         }\n",
    )
    .unwrap();
    fs::write(
        package.join("common/array.d.ts"),
        "// Referenced file notice\n\
         /// <reference path=\"./deep.d.ts\" />\n\
         interface Options { enabled: boolean; }\n\
         type Tag = string;\n\
         declare namespace Extras { interface Nested { ready: boolean; } }\n\
         declare module \"../index\" {\n\
             interface StatStatic {\n\
                 sum(values: number[]): number;\n\
             }\n\
         }\n\
         declare module \"unrelated-package\" {\n\
             function untouched(): void;\n\
         }\n",
    )
    .unwrap();
    fs::write(
        package.join("common/deep.d.ts"),
        "interface DeepOption { depth: number; }\n\
         declare module '../index' {\n\
             interface StatStatic { deep(value: DeepOption): string; }\n\
         }\n",
    ).unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { sum: function(values) { return values.reduce(function(a, b) { return a + b; }, 0); } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "stat-kit").unwrap();
    let declarations = resolve(&registry, "stat-kit").unwrap().dts_source;
    assert!(
        declarations.contains("declare namespace _"),
        "{declarations}"
    );
    assert!(
        declarations.contains("sum(values: number[]): number"),
        "{declarations}"
    );
    for required in ["interface Options", "type Tag", "namespace Extras",
        "interface DeepOption", "deep(value: DeepOption): string"] {
        assert!(declarations.contains(required), "missing {required}: {declarations}");
    }
    assert!(!declarations.contains("declare module '../index'"), "{declarations}");
    assert!(
        declarations.contains("declare module \"unrelated-package\""),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_unwraps_a_self_targeting_ambient_module() {
    // A declaration file whose *entire* API is one `declare module
    // "<own name>" { ... }` block -- real-world example: highlight.js's
    // `types/index.d.ts`, which wraps its whole surface (including `export
    // default hljs`) that way. thaw-bridge only reads top-level exports, so
    // without unwrapping, every `hljs.someMethod(...)` failed with "call to
    // unknown function". A sibling ambient module for a subpath/private
    // surface must be left exactly as written.
    let scratch = temp_registry("installed-dts-ambient-self-scratch");
    let registry = temp_registry("installed-dts-ambient-self-registry");
    let package = scratch.join("node_modules/ambient-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"ambient-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "declare module 'ambient-kit/private' {\n\
             export type KeywordData = [string, number];\n\
         }\n\
         declare module 'ambient-kit' {\n\
             export interface Api {\n\
                 greet(name: string): string;\n\
             }\n\
             const api: Api;\n\
             export default api;\n\
         }\n\
         declare module 'ambient-kit/sub' {\n\
             export function extra(): void;\n\
         }\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { greet: function(name) { return 'hi ' + name; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "ambient-kit").unwrap();
    let declarations = resolve(&registry, "ambient-kit").unwrap().dts_source;
    assert!(
        declarations.contains("greet(name: string): string"),
        "{declarations}"
    );
    assert!(
        declarations.contains("export default api"),
        "{declarations}"
    );
    assert!(
        !declarations.contains("declare module 'ambient-kit'"),
        "the self-targeting ambient module should have been unwrapped:\n{declarations}"
    );
    assert!(
        declarations.contains("declare module 'ambient-kit/private'"),
        "{declarations}"
    );
    assert!(
        declarations.contains("declare module 'ambient-kit/sub'"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_follows_a_barrel_reexport_for_classes_and_consts() {
    // A package whose entry `.d.ts` has no declarations of its own, only
    // `export { X } from "./inner.js"` lines (real-world example:
    // graphql's `type/index.d.ts`, two hops from the root). Class/const
    // re-exports were never followed -- only functions/callable consts
    // were -- so `GraphQLObjectType`/`GraphQLSchema`/`GraphQLString` were
    // all silently missing from the flattened `package.d.ts`
    // ("`graphql` has no export named `GraphQLObjectType`").
    let scratch = temp_registry("installed-dts-barrel-scratch");
    let registry = temp_registry("installed-dts-barrel-registry");
    let package = scratch.join("node_modules/barrel-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"barrel-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "export { Widget, COLOR } from \"./inner.js\";\n",
    )
    .unwrap();
    fs::write(
        package.join("inner.d.ts"),
        "export declare class Widget { render(): string; }\n\
         export declare const COLOR: { name: string };\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { Widget: class { render() { return 'w'; } }, COLOR: { name: 'red' } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "barrel-kit").unwrap();
    let declarations = resolve(&registry, "barrel-kit").unwrap().dts_source;
    assert!(
        declarations.contains("class Widget"),
        "{declarations}"
    );
    assert!(
        declarations.contains("const COLOR"),
        "{declarations}"
    );
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

    let selected = select_prebuilt_addon_with(&package, &mut Vec::new(), |_| Ok(true)).unwrap().unwrap();
    assert_eq!(selected.path, target.join(filename));
    assert_eq!(selected.platform, platform);
    assert_eq!(selected.arch, arch);
    assert_eq!(selected.libc, libc);
    let _ = fs::remove_dir_all(package);
}

/// `prebuildify`'s subdirectory-per-target layout
/// (`prebuilds/<platform>-<arch>/*.node`) isn't the only real one: a
/// package can also vendor a flat `prebuilds/<platform>-<arch>.node`
/// file directly, no per-target subdirectory at all -- real trigger:
/// `better-sqlite3` 13 dropped its `prebuild-install` dependency for
/// exactly this simpler scheme, and the registry used to report
/// "available targets: none" for it (the old subdirectory-listing
/// diagnostic only looked at directories, never flat `.node` files).
#[test]
fn selects_a_flat_bundled_node_prebuild() {
    let package = temp_registry("select_flat_native_prebuild");
    let (platform, arch, libc) = target_prebuild_components();
    fs::create_dir_all(package.join("prebuilds")).unwrap();
    let filename = if platform == "linux" && libc == "musl" {
        format!("linuxmusl-{arch}.node")
    } else {
        format!("{platform}-{arch}.node")
    };
    let file = package.join("prebuilds").join(&filename);
    fs::write(&file, b"native bytes").unwrap();

    let selected = select_prebuilt_addon_with(&package, &mut Vec::new(), |_| Ok(true)).unwrap().unwrap();
    assert_eq!(selected.path, file);
    assert_eq!(selected.platform, platform);
    assert_eq!(selected.arch, arch);
    assert_eq!(selected.libc, libc);
    let _ = fs::remove_dir_all(package);
}

#[test]
fn prefers_node_over_electron_prebuilds() {
    let package = temp_registry("prefer_node_prebuild");
    let (platform, arch, _) = target_prebuild_components();
    let target = package.join("prebuilds").join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("electron.napi.node"), b"electron").unwrap();
    fs::write(target.join("node.napi.node"), b"node").unwrap();

    let selected = select_prebuilt_addon_with(&package, &mut Vec::new(), |_| Ok(true)).unwrap().unwrap();
    assert_eq!(selected.path, target.join("node.napi.node"));
    let _ = fs::remove_dir_all(package);
}

#[test]
fn skips_node_abi_prebuild_before_napi_prebuild() {
    let package = temp_registry("skip_node_abi_prebuild");
    let (platform, arch, _) = target_prebuild_components();
    let target = package.join("prebuilds").join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("node.abi108.node"), b"node ABI").unwrap();
    fs::write(target.join("node.napi.node"), b"Node-API").unwrap();
    assert!(!usable_napi_prebuild(&target.join("node.abi108.node")).unwrap());
    let mut diagnostics = Vec::new();
    let selected = select_prebuilt_addon_with(&package, &mut diagnostics, |path| {
        Ok(path.file_name().unwrap() != "node.abi108.node")
    }).unwrap().unwrap();
    assert_eq!(selected.path, target.join("node.napi.node"));
    assert!(diagnostics.iter().any(|message| message.contains("node.abi108.node")));
    let _ = fs::remove_dir_all(package);
}

#[test]
fn incompatible_flat_prebuild_does_not_hide_compatible_target_prebuild() {
    let package = temp_registry("flat_native_candidate_fallback");
    let (platform, arch, libc) = target_prebuild_components();
    let prebuilds = package.join("prebuilds");
    let flat = prebuilds.join(flat_prebuild_file_name(platform, arch, libc));
    let target = prebuilds.join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target).unwrap();
    fs::write(&flat, b"node ABI").unwrap();
    fs::write(target.join("node.napi.node"), b"Node-API").unwrap();
    let mut diagnostics = Vec::new();
    let selected = select_prebuilt_addon_with(&package, &mut diagnostics, |path| {
        Ok(path != flat.as_path())
    }).unwrap().unwrap();
    assert_eq!(selected.path, target.join("node.napi.node"));
    assert!(diagnostics.iter().any(|message| message.contains("not a supported N-API")));
    let _ = fs::remove_dir_all(package);
}

#[test]
fn native_symbol_gate_requires_napi_entry_without_node_internals() {
    assert!(napi_symbol_suitability("00000000 T napi_register_module_v1\n U napi_get_version"));
    let registered = if cfg!(target_os = "macos") {
        " U _napi_module_register\n U _napi_create_object"
    } else {
        " U napi_module_register\n U napi_create_object"
    };
    assert!(napi_symbol_suitability(registered));
    let node_internal = if cfg!(target_os = "macos") {
        "00000000 T _napi_register_module_v1\n U __ZN2v8Something"
    } else {
        "00000000 T napi_register_module_v1\n U _ZN2v8Something"
    };
    assert!(!napi_symbol_suitability(node_internal));
    assert!(!napi_symbol_suitability("00000000 T napi_register_module_v1\n U node_module_register"));
    if !cfg!(target_os = "macos") {
        assert!(!napi_symbol_suitability("00000000 T _napi_register_module_v1"));
    }
    assert!(!napi_symbol_suitability("00000000 V napi_register_module_v1"));
    assert!(!napi_symbol_suitability("00000000 T unrelated_entry"));
}

#[test]
fn incompatible_bundled_prebuild_falls_back_to_platform_optional_addon() {
    let node_modules = temp_registry("native_candidate_fallback");
    let package = node_modules.join("example");
    let (platform, arch, libc) = target_prebuild_components();
    let target = package.join("prebuilds").join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("node.abi108.node"), b"node ABI").unwrap();
    let dependency = if platform == "linux" {
        format!("@example/addon-{platform}-{arch}-{libc}")
    } else {
        format!("@example/addon-{platform}-{arch}")
    };
    let dependency_dir = node_modules.join(&dependency);
    fs::create_dir_all(&dependency_dir).unwrap();
    fs::write(dependency_dir.join("package.json"), r#"{"main":"binding.node"}"#).unwrap();
    fs::write(dependency_dir.join("binding.node"), b"Node-API").unwrap();
    let manifest = serde_json::json!({"optionalDependencies": {dependency.clone(): "1.0.0"}});
    let mut diagnostics = Vec::new();
    let selected = select_installed_addon_with(
        &package, &node_modules, &manifest, Some("module.exports = {};"), &mut diagnostics,
        |path| Ok(path.file_name().unwrap() == "binding.node"),
    ).unwrap().unwrap();
    assert_eq!(selected.path, dependency_dir.join("binding.node"));
    assert!(diagnostics.iter().any(|message| message.contains("node.abi108.node")));
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn incompatible_optional_main_uses_compatible_sibling() {
    let node_modules = temp_registry("optional_main_abi_sibling_napi");
    let (platform, arch, libc) = target_prebuild_components();
    let dependency = if platform == "linux" {
        format!("@example/addon-{platform}-{arch}-{libc}")
    } else {
        format!("@example/addon-{platform}-{arch}")
    };
    let root = node_modules.join(&dependency);
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::write(root.join("package.json"), r#"{"main":"node.abi108.node"}"#).unwrap();
    fs::write(root.join("node.abi108.node"), b"node ABI").unwrap();
    fs::write(root.join("lib/binding.napi.node"), b"Node-API").unwrap();
    let manifest = serde_json::json!({"optionalDependencies": {dependency.clone(): "1.0.0"}});
    let mut diagnostics = Vec::new();
    let selected = select_optional_dependency_addon_with(
        &node_modules, &manifest, &mut diagnostics,
        |path| Ok(path.file_name().unwrap() == "binding.napi.node"),
    ).unwrap().unwrap();
    assert_eq!(selected.path, root.join("lib/binding.napi.node"));
    assert!(diagnostics.iter().any(|message| message.contains("node.abi108.node")));
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn generated_napi_addon_is_found_before_prebuild_download_decision() {
    let node_modules = temp_registry("generated_before_download");
    let package = node_modules.join("example");
    let (platform, arch, _) = target_prebuild_components();
    let target = package.join("prebuilds").join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("node.abi108.node"), b"node ABI").unwrap();
    let generated = node_modules.join(".generated/client/addon.node");
    fs::create_dir_all(generated.parent().unwrap()).unwrap();
    fs::write(&generated, b"Node-API").unwrap();
    let manifest = serde_json::json!({"binary": {"napi_versions": [10]}});
    let mut diagnostics = Vec::new();
    let selected = select_installed_addon_with(
        &package, &node_modules, &manifest,
        Some("module.exports = require('.generated/client')"), &mut diagnostics,
        |path| Ok(path == generated.as_path()),
    ).unwrap().unwrap();
    assert_eq!(selected.path, generated);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn missing_native_main_can_be_supplied_after_predownload_probe() {
    let node_modules = temp_registry("downloaded_native_main");
    let package = node_modules.join("example");
    let (platform, arch, _) = target_prebuild_components();
    let main = format!("prebuilds/{platform}-{arch}/binding.node");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"), format!(r#"{{"name":"example","main":"{main}"}}"#)).unwrap();
    let early_bundle = bundle_commonjs_package_cached(
        &node_modules, "example", &package, &main, &mut SourceCache::default(),
    );
    assert!(early_bundle.is_err());
    let mut diagnostics = Vec::new();
    assert!(select_installed_addon_with(
        &package, &node_modules, &serde_json::json!({}), None, &mut diagnostics,
        |_| Ok(true),
    ).unwrap().is_none());
    let addon = package.join(&main);
    fs::create_dir_all(addon.parent().unwrap()).unwrap();
    fs::write(&addon, b"downloaded Node-API addon").unwrap();
    let (source, _, _, _) = bundle_commonjs_package_cached(
        &node_modules, "example", &package, &main, &mut SourceCache::default(),
    ).unwrap();
    assert!(select_installed_addon_with(
        &package, &node_modules, &serde_json::json!({}), Some(source.as_str()), &mut diagnostics,
        |_| Ok(true),
    ).unwrap().is_some());
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn native_candidate_inspection_failure_does_not_become_fallback() {
    let package = temp_registry("native_candidate_inspection_error");
    let (platform, arch, _) = target_prebuild_components();
    let target = package.join("prebuilds").join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("node.napi.node"), b"candidate").unwrap();
    let mut diagnostics = Vec::new();
    let error = select_prebuilt_addon_with(&package, &mut diagnostics, |_| {
        Err("permission denied during inspection".to_string())
    }).unwrap_err();
    assert!(error.contains("permission denied"));
    assert!(diagnostics.is_empty());
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
fn selects_highest_supported_napi_prebuild_and_rejects_future_only_versions() {
    // Unrun regression: installer and host must agree on the Node-API ceiling.
    let mut manifest = serde_json::json!({
        "name": "binding",
        "version": "1.2.3",
        "repository": "https://github.com/example/binding",
        "binary": { "napi_versions": [3, 8, 9, 10, 11, 99] }
    });
    let (_, asset, _, _) = prebuild_install_asset(&manifest).unwrap();
    assert!(asset.contains("-napi-v10-"));
    manifest["binary"]["napi_versions"] = serde_json::json!([9, 11, 99]);
    let (_, asset, _, _) = prebuild_install_asset(&manifest).unwrap();
    assert!(asset.contains("-napi-v9-"));
    manifest["binary"]["napi_versions"] = serde_json::json!([11, 99]);
    assert!(prebuild_install_asset(&manifest).is_none());
}

#[test]
fn reports_available_targets_when_no_prebuild_matches() {
    let package = temp_registry("mismatched_native_prebuild");
    let target = package.join("prebuilds/imaginary-other");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("binding.node"), b"native bytes").unwrap();
    let mut diagnostics = Vec::new();
    assert!(select_prebuilt_addon_with(&package, &mut diagnostics, |_| Ok(true))
        .unwrap().is_none());
    let diagnostic = diagnostics.join("; ");
    assert!(diagnostic.contains("no bundled native addon matches"));
    assert!(diagnostic.contains("imaginary-other"));
    let _ = fs::remove_dir_all(package);
}

#[test]
fn optional_dependency_target_matches_complete_architecture_and_libc_suffixes() {
    assert!(optional_dependency_matches_target("@esbuild/linux-arm", "linux-arm"));
    assert!(optional_dependency_matches_target("@example/addon-linux-arm", "linux-arm"));
    assert!(!optional_dependency_matches_target("@esbuild/linux-arm64", "linux-arm"));
    assert!(!optional_dependency_matches_target("@example/addon-linux-arm64", "linux-arm"));
    assert!(optional_dependency_matches_target("@example/addon-linux-x64", "linux-x64"));
    assert!(optional_dependency_matches_target("@example/addon-linux-x64-glibc", "linux-x64-glibc"));
    assert!(!optional_dependency_matches_target("@example/addon-linux-x64-musl", "linux-x64"));
    assert!(!optional_dependency_matches_target("@example/addon-linux-x64-musl", "linux-x64-glibc"));
}

#[test]
fn glibc_shared_library_collection_skips_musl_optional_dependency() {
    let (platform, arch, libc) = target_prebuild_components();
    if platform != "linux" || libc != "glibc" {
        return;
    }
    let node_modules = temp_registry("glibc-shared-libraries");
    let glibc_name = format!("@example/addon-linux-{arch}-glibc");
    let musl_name = format!("@example/addon-linux-{arch}-musl");
    let glibc_root = node_modules.join(&glibc_name);
    let musl_root = node_modules.join(&musl_name);
    fs::create_dir_all(&glibc_root).unwrap();
    fs::create_dir_all(&musl_root).unwrap();
    let glibc_library = glibc_root.join("libtarget.so");
    fs::write(&glibc_library, b"glibc").unwrap();
    fs::write(musl_root.join("libtarget.so"), b"musl").unwrap();
    let manifest = serde_json::json!({
        "optionalDependencies": {glibc_name.clone(): "1.0.0", musl_name.clone(): "1.0.0"}
    });
    assert_eq!(platform_shared_libraries(&node_modules, &manifest).unwrap(), vec![glibc_library]);
    let _ = fs::remove_dir_all(node_modules);
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
    let selected = select_optional_dependency_addon_with(&node_modules, &manifest, &mut Vec::new(), |_| Ok(true))
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
fn selects_a_gnu_named_linux_optional_dependency_node_addon() {
    let (platform, arch, libc) = target_prebuild_components();
    if platform != "linux" || libc != "glibc" {
        return;
    }
    let node_modules = temp_registry("optional_gnu_native_prebuild");
    let dependency = format!("@example/addon-linux-{arch}-gnu");
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
    let selected = select_optional_dependency_addon_with(&node_modules, &manifest, &mut Vec::new(), |_| Ok(true))
        .unwrap()
        .unwrap();
    assert_eq!(selected.path, dependency_dir.join("binding.node"));
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn selects_a_node_addon_behind_a_platform_dependency_js_entry() {
    let node_modules = temp_registry("optional_wrapped_native_prebuild");
    let (platform, arch, libc) = target_prebuild_components();
    let dependency = if platform == "linux" && libc == "musl" {
        format!("@example/addon-linuxmusl-{arch}")
    } else {
        format!("@example/addon-{platform}-{arch}")
    };
    let dependency_dir = node_modules.join(&dependency);
    fs::create_dir_all(dependency_dir.join("lib")).unwrap();
    fs::write(
        dependency_dir.join("package.json"),
        format!(r#"{{"name":"{dependency}","main":"index.js"}}"#),
    )
    .unwrap();
    fs::write(dependency_dir.join("index.js"), "module.exports = require('./lib/addon.node')").unwrap();
    fs::write(dependency_dir.join("lib/addon.node"), b"native bytes").unwrap();
    let manifest = serde_json::json!({
        "optionalDependencies": { dependency.clone(): "1.0.0" }
    });

    let selected = select_optional_dependency_addon_with(&node_modules, &manifest, &mut Vec::new(), |_| Ok(true))
        .unwrap()
        .unwrap();
    assert_eq!(selected.path, dependency_dir.join("lib/addon.node"));
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
fn same_named_file_precedes_directory_package_main() {
    let dir = temp_registry("file_before_directory_main");
    let directory = dir.join("foo");
    fs::create_dir(&directory).unwrap();
    fs::write(dir.join("foo.js"), "module.exports = 'file';").unwrap();
    fs::write(directory.join("package.json"), r#"{"main":"./inner.js"}"#).unwrap();
    fs::write(directory.join("inner.js"), "module.exports = 'directory';").unwrap();
    let (relative, absolute) = resolve_module_path(&dir, "./foo").unwrap();
    assert_eq!(relative, "./foo.js");
    assert_eq!(absolute, dir.join("foo.js"));
    for directory_request in ["./foo/", "./foo/."] {
        let (relative, absolute) = resolve_module_path(&dir, directory_request).unwrap();
        assert_eq!(relative, "foo/inner.js");
        assert_eq!(absolute, directory.join("inner.js"));
    }
    fs::remove_file(dir.join("foo.js")).unwrap();
    let (relative, absolute) = resolve_module_path(&dir, "./foo").unwrap();
    assert_eq!(relative, "foo/inner.js");
    assert_eq!(absolute, directory.join("inner.js"));
    fs::write(dir.join(".js"), "module.exports = 'sibling';").unwrap();
    fs::write(dir.join("package.json"), r#"{"main":"./entry.js"}"#).unwrap();
    fs::write(dir.join("entry.js"), "module.exports = 'root';").unwrap();
    let (relative, absolute) = resolve_module_path(&dir, ".").unwrap();
    assert_eq!(relative, "entry.js");
    assert_eq!(absolute, dir.join("entry.js"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn bundled_trailing_slash_require_keeps_directory_intent() {
    let package = temp_registry("bundle_directory_intent");
    fs::create_dir(package.join("foo")).unwrap();
    fs::write(package.join("index.js"), "module.exports = require('./foo/');").unwrap();
    fs::write(package.join("foo.js"), "module.exports = 'file-only-value';").unwrap();
    fs::write(package.join("foo/package.json"), r#"{"main":"./inner.js"}"#).unwrap();
    fs::write(package.join("foo/inner.js"), "module.exports = 'directory-only-value';").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&package, "pkg", &package, "index.js").unwrap();
    assert!(bundle.contains("directory-only-value"));
    assert!(!bundle.contains("file-only-value"));
    let _ = fs::remove_dir_all(package);
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
fn directory_main_self_cycle_uses_index_fallback() {
    let dir = temp_registry("module_main_self_cycle");
    fs::write(dir.join("package.json"), r#"{"main":"."}"#).unwrap();
    fs::write(dir.join("index.js"), "module.exports = 1;").unwrap();
    let (relative, absolute) = resolve_module_path(&dir, ".").unwrap();
    assert_eq!(relative, "index.js");
    assert_eq!(absolute.canonicalize().unwrap(), dir.join("index.js").canonicalize().unwrap());
    fs::write(dir.join("package.json"), r#"{"main":"./."}"#).unwrap();
    let (relative, absolute) = resolve_module_path(&dir, ".").unwrap();
    assert_eq!(relative, "index.js");
    assert_eq!(absolute.canonicalize().unwrap(), dir.join("index.js").canonicalize().unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn mutually_recursive_directory_entries_return_a_cycle_diagnostic() {
    let dir = temp_registry("module_main_mutual_cycle");
    for (name, main) in [("a", "../b"), ("b", "../a")] {
        let directory = dir.join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("package.json"), format!(r#"{{"main":"{main}"}}"#)).unwrap();
    }
    let error = resolve_module_path(&dir, "./a").unwrap_err();
    assert!(error.contains("circular directory module entry"), "{error}");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn symlinked_directory_entry_cycle_uses_index_fallback() {
    use std::os::unix::fs::symlink;
    let dir = temp_registry("module_main_symlink_cycle");
    fs::write(dir.join("package.json"), r#"{"main":"./alias"}"#).unwrap();
    fs::write(dir.join("index.js"), "module.exports = 1;").unwrap();
    symlink(".", dir.join("alias")).unwrap();
    let (relative, absolute) = resolve_module_path(&dir, ".").unwrap();
    assert_eq!(relative, "alias/index.js");
    assert_eq!(absolute.canonicalize().unwrap(), dir.join("index.js").canonicalize().unwrap());
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
                const externalPackage = import(`external-package`);
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
    assert!(analysis.has_nonliteral_module_load);
    assert!(analysis.specs.is_empty());
    assert!(analyze_module("require(name)").has_nonliteral_module_load);
}

#[test]
fn strips_node_shebangs_inside_bundle_factories() {
    let package = temp_registry("bundle_shebang");
    fs::write(package.join("index.js"), "#!/usr/bin/env node\nmodule.exports = 42;\n").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&package, "pkg", &package, "index.js").unwrap();
    assert!(!bundle.contains("#!/usr/bin/env node"));
    let _ = fs::remove_dir_all(package);
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

    let (name, relative, abs, dir) = resolve_bare_require(&node_modules, &node_modules, "es-errors/type").unwrap();
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

    let (name, relative, ..) = resolve_bare_require(&node_modules, &node_modules, "@scope/pkg/lib/util").unwrap();
    assert_eq!(name, "@scope/pkg");
    assert_eq!(relative, "lib/util.js");

    let _ = fs::remove_dir_all(&node_modules);
}

#[test]
fn resolves_generated_declarations_from_a_dot_named_package() {
    let root = temp_registry("generated_dot_package");
    let node_modules = root.join("node_modules");
    let entry = node_modules.join("@scope/client/default.d.ts");
    let generated = node_modules.join(".generated/client/default.d.ts");
    fs::create_dir_all(entry.parent().unwrap()).unwrap();
    fs::create_dir_all(generated.parent().unwrap()).unwrap();
    fs::write(&entry, "export * from '.generated/client/default';").unwrap();
    fs::write(&generated, "export declare class GeneratedClient {}").unwrap();

    assert_eq!(
        declaration_reexport_path(&entry, ".generated/client/default"),
        // `declaration_reexport_path` canonicalizes its result (for cycle
        // detection), so compare against the canonical spelling.
        Some(generated.canonicalize().unwrap())
    );

    let _ = fs::remove_dir_all(root);
}

/// `declaration_reexport_path` must canonicalize its result, so a re-export
/// cycle is seen as the *same* `PathBuf` on every hop. Without it, each
/// `./x.js` hop appended another `./` segment (`a/./b.d.ts` ->
/// `a/././b.d.ts` -> ...), so a path-keyed `visited` guard never matched
/// and `local_type_declaration_snippets` recursed forever -- real trigger:
/// drizzle-orm's `supabase/index.d.ts` re-export cycle, which made
/// `thaw registry add drizzle-orm` hang for tens of minutes.
#[test]
fn declaration_reexport_path_is_canonicalized_for_cycle_detection() {
    let root = temp_registry("canonical_reexport");
    let a = root.join("a.d.ts");
    let b = root.join("b.d.ts");
    fs::write(&a, "export * from \"./b.js\";\n").unwrap();
    fs::write(&b, "export * from \"./a.js\";\n").unwrap();

    let resolved_b = declaration_reexport_path(&a, "./b.js").unwrap();
    assert_eq!(resolved_b, b.canonicalize().unwrap());
    // The hop back to `a` must return the original `a`'s canonical path,
    // not a fresh `a/./a.d.ts`-style spelling that defeats `visited`.
    assert_eq!(
        declaration_reexport_path(&resolved_b, "./a.js").unwrap(),
        a.canonicalize().unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn relative_declaration_directory_prefers_manifest_types_over_index() {
    // Unrun regression: a relative directory can have both a fallback index
    // and an explicit package type entry. An adjacent file still wins first.
    let root = temp_registry("relative-manifest-types");
    let entry = root.join("entry.d.ts");
    let feature = root.join("feature");
    fs::create_dir_all(feature.join("types")).unwrap();
    fs::write(&entry, "export * from './feature';").unwrap();
    fs::write(feature.join("package.json"), r#"{"types":"./types/public.d.ts"}"#).unwrap();
    fs::write(feature.join("types/public.d.ts"), "export function chosen(): string;").unwrap();
    fs::write(feature.join("index.d.ts"), "export function fallback(): number;").unwrap();
    assert_eq!(
        declaration_reexport_path(&entry, "./feature"),
        Some(feature.join("types/public.d.ts").canonicalize().unwrap())
    );
    fs::write(root.join("feature.d.ts"), "export function adjacent(): boolean;").unwrap();
    assert_eq!(
        declaration_reexport_path(&entry, "./feature"),
        Some(root.join("feature.d.ts").canonicalize().unwrap())
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bare_type_reexports_use_nearest_package_subpath_exports() {
    // Unrun regression: the nested version wins over a hoisted version;
    // an exports map is authoritative even if a physical fallback exists.
    let root = temp_registry("bare-type-subpath-resolution");
    let app = root.join("node_modules/app");
    let nested = app.join("node_modules/dependency");
    let hoisted = root.join("node_modules/dependency");
    let scoped = root.join("node_modules/@scope/client");
    fs::create_dir_all(app.join("types")).unwrap();
    fs::create_dir_all(nested.join("types")).unwrap();
    fs::create_dir_all(hoisted.join("types")).unwrap();
    fs::create_dir_all(scoped.join("types")).unwrap();
    let entry = app.join("types/index.d.ts");
    fs::write(&entry, "export * from 'dependency/feature';").unwrap();
    fs::write(nested.join("package.json"),
        r#"{"exports":{"./feature":{"types":"./types/nested.d.ts"},"./wild/*":{"types":"./types/*.d.ts"}}}"#,
    ).unwrap();
    fs::write(nested.join("types/nested.d.ts"), "export function nested(): number;").unwrap();
    fs::write(nested.join("types/card.d.ts"), "export function card(): string;").unwrap();
    fs::write(hoisted.join("package.json"),
        r#"{"exports":{"./feature":{"types":"./types/hoisted.d.ts"}}}"#,
    ).unwrap();
    fs::write(hoisted.join("types/hoisted.d.ts"), "export function hoisted(): boolean;").unwrap();
    fs::write(scoped.join("package.json"),
        r#"{"exports":{"./feature":{"types":"./types/scoped.d.ts"}}}"#,
    ).unwrap();
    fs::write(scoped.join("types/scoped.d.ts"), "export function scoped(): string;").unwrap();

    assert_eq!(declaration_reexport_path(&entry, "dependency/feature"),
        Some(nested.join("types/nested.d.ts").canonicalize().unwrap()));
    assert_eq!(declaration_reexport_path(&entry, "dependency/wild/card"),
        Some(nested.join("types/card.d.ts").canonicalize().unwrap()));
    assert_eq!(declaration_reexport_path(&entry, "@scope/client/feature"),
        Some(scoped.join("types/scoped.d.ts").canonicalize().unwrap()));
    let flattened = dts_source_with_reexported_functions(
        &entry, &fs::read_to_string(&entry).unwrap(),
    ).unwrap();
    assert!(flattened.contains("function nested(): number"), "{flattened}");
    assert!(!flattened.contains("function hoisted()"), "{flattened}");

    fs::remove_dir_all(&nested).unwrap();
    assert_eq!(declaration_reexport_path(&entry, "dependency/feature"),
        Some(hoisted.join("types/hoisted.d.ts").canonicalize().unwrap()));
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("package.json"), r#"{"exports":{"./other":{"types":"./other.d.ts"}}}"#).unwrap();
    fs::write(nested.join("feature.d.ts"), "export function shadow(): void;").unwrap();
    assert_eq!(declaration_reexport_path(&entry, "dependency/feature"), None);
    fs::write(nested.join("package.json"),
        r#"{"exports":{"./feature":{"types":"./types/missing.d.ts"}}}"#,
    ).unwrap();
    assert_eq!(declaration_reexport_path(&entry, "dependency/feature"), None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bare_type_reexports_keep_nearest_manifestless_declarations() {
    // Unrun regression: declarations can be installed without a package.json.
    // A nearer type file wins over an ancestor package with a valid manifest.
    let root = temp_registry("bare-manifestless-types");
    let app = root.join("node_modules/app");
    let nested_modules = app.join("node_modules");
    let hoisted = root.join("node_modules/dependency");
    let hoisted_scoped = root.join("node_modules/@scope/client");
    fs::create_dir_all(app.join("types")).unwrap();
    fs::create_dir_all(nested_modules.join("dependency")).unwrap();
    fs::create_dir_all(nested_modules.join("@scope/client")).unwrap();
    fs::create_dir_all(&hoisted).unwrap();
    fs::create_dir_all(&hoisted_scoped).unwrap();
    let entry = app.join("types/index.d.ts");
    fs::write(&entry, "export * from 'dependency';").unwrap();
    fs::write(hoisted.join("package.json"), r#"{"types":"./index.d.ts","exports":{"./feature":{"types":"./feature.d.ts"}}}"#).unwrap();
    fs::write(hoisted.join("index.d.ts"), "export function hoisted(): void;").unwrap();
    fs::write(hoisted.join("feature.d.ts"), "export function hoistedFeature(): void;").unwrap();
    fs::write(hoisted_scoped.join("package.json"), r#"{"types":"./index.d.ts"}"#).unwrap();
    fs::write(hoisted_scoped.join("index.d.ts"), "export function hoistedScoped(): void;").unwrap();

    let nested = nested_modules.join("dependency");
    fs::write(nested.join("index.d.ts"), "export function local(): void;").unwrap();
    fs::write(nested.join("feature.d.ts"), "export function localFeature(): void;").unwrap();
    let scoped = nested_modules.join("@scope/client");
    fs::write(scoped.join("index.d.ts"), "export function localScoped(): void;").unwrap();
    fs::write(scoped.join("feature.d.ts"), "export function localScopedFeature(): void;").unwrap();
    assert_eq!(declaration_reexport_path(&entry, "dependency"),
        Some(nested.join("index.d.ts").canonicalize().unwrap()));
    assert_eq!(declaration_reexport_path(&entry, "dependency/feature"),
        Some(nested.join("feature.d.ts").canonicalize().unwrap()));
    assert_eq!(declaration_reexport_path(&entry, "@scope/client"),
        Some(scoped.join("index.d.ts").canonicalize().unwrap()));
    assert_eq!(declaration_reexport_path(&entry, "@scope/client/feature"),
        Some(scoped.join("feature.d.ts").canonicalize().unwrap()));
    fs::remove_file(nested.join("feature.d.ts")).unwrap();
    assert_eq!(declaration_reexport_path(&entry, "dependency/feature"), None);

    fs::remove_dir_all(&nested).unwrap();
    fs::write(nested_modules.join("dependency.d.ts"), "export function sibling(): void;").unwrap();
    assert_eq!(declaration_reexport_path(&entry, "dependency"),
        Some(nested_modules.join("dependency.d.ts").canonicalize().unwrap()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn named_declaration_hop_follows_wildcard_function_and_superclass() {
    // Unrun regression: the named source is a barrel rather than the final
    // declaration file. `Base` is supporting type information, not an alias
    // of the selected `Derived` declaration.
    let root = temp_registry("named-hop-wildcard");
    let entry = root.join("entry.d.ts");
    let barrel = root.join("barrel.d.ts");
    fs::write(&entry, "export { parse } from './barrel';\nexport { Derived as PublicDerived } from './barrel';\n").unwrap();
    fs::write(&barrel, "export * from './functions';\nexport * from './derived';\n").unwrap();
    fs::write(root.join("functions.d.ts"), "export declare function parse(value: string): number;").unwrap();
    fs::write(root.join("derived.d.ts"), "import { Base } from './base';\nexport declare class Derived extends Base { own(): string; }").unwrap();
    fs::write(root.join("base.d.ts"), "export declare class Base { inherited(): number; }").unwrap();
    let mut visited = std::collections::BTreeSet::new();
    let functions = reexported_function_declarations(&entry, "parse", &mut visited).unwrap();
    assert_eq!(functions.len(), 1, "{functions:?}");
    assert!(functions[0].contains("function parse(value: string): number"), "{functions:?}");
    let classes = reexported_class_or_interface_declarations(&entry, "PublicDerived").unwrap();
    assert!(classes.iter().any(|item| item.contains("class Derived extends Base")), "{classes:?}");
    assert!(classes.iter().any(|item| item.contains("class Base") && item.contains("inherited")), "{classes:?}");
    let aliased = reexported_declarations_as(classes, "PublicDerived", &entry, false).join("\n");
    assert!(aliased.contains("Derived as PublicDerived"), "{aliased}");
    assert!(aliased.contains("class Base") && !aliased.contains("Base as PublicDerived"), "{aliased}");
    let flattened = dts_source_with_reexported_functions(&entry, &fs::read_to_string(&entry).unwrap()).unwrap();
    assert!(flattened.contains("function parse(value: string): number"), "{flattened}");
    assert!(flattened.contains("Derived as PublicDerived"), "{flattened}");
    assert!(flattened.contains("class Base") && !flattened.contains("Base as PublicDerived"), "{flattened}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reexported_child_keeps_same_file_superclass_chain() {
    // Unrun regression: the selected class is re-exported, while its Base
    // and GrandBase are declared in the target file without any imports.
    let root = temp_registry("same-file-superclasses");
    let entry = root.join("entry.d.ts");
    fs::write(&entry, "export { Child as PublicChild } from './child';").unwrap();
    fs::write(
        root.join("child.d.ts"),
        "declare class GrandBase { grand(): boolean; }\n\
         declare class Base extends GrandBase { inherited(): number; }\n\
         export declare class Child extends Base { own(): string; }",
    ).unwrap();

    let classes = reexported_class_or_interface_declarations(&entry, "PublicChild").unwrap();
    assert_eq!(classes.len(), 3, "{classes:?}");
    assert!(classes[0].contains("class Child extends Base"), "{classes:?}");
    assert!(classes[1].contains("class Base extends GrandBase"), "{classes:?}");
    assert!(classes[2].contains("class GrandBase"), "{classes:?}");
    let flattened = dts_source_with_reexported_functions(
        &entry, &fs::read_to_string(&entry).unwrap(),
    ).unwrap();
    assert!(flattened.contains("Child as PublicChild"), "{flattened}");
    assert!(flattened.contains("inherited(): number"), "{flattened}");
    assert!(flattened.contains("grand(): boolean"), "{flattened}");
    assert!(!flattened.contains("Base as PublicChild"), "{flattened}");
    assert!(thaw_parser::parse_declarations(&flattened).is_ok(), "{flattened}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn selects_a_single_addon_from_a_hidden_generated_package() {
    let node_modules = temp_registry("generated_native_addon");
    let addon = node_modules.join(".generated/client/engine.so.node");
    fs::create_dir_all(addon.parent().unwrap()).unwrap();
    fs::create_dir_all(node_modules.join(".bin")).unwrap();
    fs::write(&addon, b"addon").unwrap();

    assert_eq!(
        select_generated_addon_with(&node_modules, "require('.generated/client')", &mut Vec::new(), |_| Ok(true))
            .unwrap()
            .unwrap()
            .path,
        addon
    );
    assert!(select_generated_addon_with(&node_modules, "module.exports = {}", &mut Vec::new(), |_| Ok(true))
        .unwrap()
        .is_none());

    let _ = fs::remove_dir_all(node_modules);
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

#[test]
fn builtin_declarations_cover_typed_buffer_crypto_and_fs_overloads() {
    let buffer = resolve_builtin("node:buffer").unwrap().dts_source;
    assert!(buffer.contains("static alloc(size: number"));
    assert!(buffer.contains("readInt32BE(offset?: number): number"));

    let crypto = resolve_builtin("node:crypto").unwrap().dts_source;
    assert!(crypto.contains("function pbkdf2Sync("));
    assert!(crypto.contains("function scryptSync("));
    assert!(crypto.contains("function scryptSync("));

    let fs = resolve_builtin("node:fs").unwrap().dts_source;
    assert!(fs.contains("readdirSync(path: string): string[]"));
    assert!(fs.contains("rmSync(path: string, options?: any)"));

    let fs_promises = resolve_builtin("node:fs/promises").unwrap().dts_source;
    assert!(fs_promises.contains("rm(path: string, options?: any): Promise<void>"));

    let events = resolve_builtin("node:events").unwrap().dts_source;
    assert!(events.contains("once(emitter: EventEmitter, event: any"));

    let stream = resolve_builtin("node:stream").unwrap().dts_source;
    assert!(stream.contains("static from(value: Json, options?: any): Readable"));

    let diagnostics = resolve_builtin("node:diagnostics_channel")
        .unwrap()
        .dts_source;
    assert!(diagnostics.contains("channel(name: string): JsValue"));

    let module = resolve_builtin("node:module").unwrap().dts_source;
    assert!(module.contains("createRequire(filename: string): JsValue"));

    let timers = resolve_builtin("node:timers").unwrap().dts_source;
    assert!(timers.contains("setInterval(callback: () => void, delay?: number): JsValue"));
    assert!(timers.contains("clearInterval(handle: JsValue): void"));

    let cluster = resolve_builtin("node:cluster").unwrap().dts_source;
    assert!(cluster.contains("const SCHED_NONE: number"));
    assert!(cluster.contains("let schedulingPolicy: number"));

    let console = resolve_builtin("node:console").unwrap().dts_source;
    assert!(console.contains("class Console"));

    let dns = resolve_builtin("node:dns").unwrap().dts_source;
    assert!(dns.contains("callback: (error: unknown, address: string, family: number) => void"));

    let readline = resolve_builtin("node:readline").unwrap().dts_source;
    assert!(readline.contains("class Interface"));
    assert!(readline.contains("on(event: string, listener: (line: string) => void): Interface"));

    let tls = resolve_builtin("node:tls").unwrap().dts_source;
    assert!(tls.contains("const DEFAULT_MIN_VERSION: string"));
    assert!(tls.contains("function getCiphers(): string[]"));

    let trace_events = resolve_builtin("node:trace_events").unwrap().dts_source;
    assert!(trace_events.contains("class Tracing"));

    let http2 = resolve_builtin("node:http2").unwrap().dts_source;
    assert!(http2.contains("interface Http2Settings"));
    assert!(http2.contains("getPackedSettings(settings: Http2Settings): JsValue"));

    let inspector = resolve_builtin("node:inspector").unwrap().dts_source;
    assert!(inspector.contains("class Session"));
    assert!(inspector.contains("error: Json | null"));

    let inspector_promises = resolve_builtin("node:inspector/promises")
        .unwrap()
        .dts_source;
    assert!(inspector_promises.contains("post(method: string, params?: any): Promise<JsValue>"));

    let domain = resolve_builtin("node:domain").unwrap().dts_source;
    assert!(domain.contains("class Domain"));
    assert!(domain.contains("listener: (error: JsValue) => void"));

    let readline_promises = resolve_builtin("node:readline/promises")
        .unwrap()
        .dts_source;
    assert!(readline_promises.contains("question(query: string, options?: any): Promise<string>"));

    let dgram = resolve_builtin("node:dgram").unwrap().dts_source;
    assert!(dgram.contains("class Socket"));
    assert!(dgram.contains("address(): SocketAddressInfo"));

    let wasi = resolve_builtin("node:wasi").unwrap().dts_source;
    assert!(wasi.contains("class WASI"));
    assert!(wasi.contains("getImportObject(): JsValue"));

    let test_reporters = resolve_builtin("node:test/reporters").unwrap().dts_source;
    assert!(test_reporters.contains("class Reporter"));
    assert!(test_reporters.contains("dot(source: Json): Reporter"));
}

/// `resolve_bare_require` used to resolve every bare specifier against
/// one flat `node_modules_dir`, regardless of which package was doing
/// the requiring -- correct for npm's common case (a dependency hoisted
/// to the top level because every consumer agrees on a compatible
/// version), but wrong the moment two packages need genuinely
/// incompatible versions of the same dependency: real npm then nests
/// the conflicting version under the dependent's own `node_modules`
/// (found via `cheerio` -> `htmlparser2` -> `entities`: `htmlparser2`
/// needs `entities@^7`, something else in the tree pins `entities@4`,
/// which wins the flat top-level slot, leaving `entities@7` nested
/// under `htmlparser2/node_modules`). The old flat-only lookup always
/// found the wrong (top-level) version's file layout in that case.
/// Fixed by walking up from the requiring package's own directory,
/// checking `<ancestor>/node_modules/<dep_name>` at each level (the
/// same algorithm real Node's `require` resolution uses), which finds
/// the nested version before ever reaching the top level.
#[test]
fn bare_requires_prefer_a_dependents_own_nested_node_modules_over_the_flat_top_level() {
    let root = temp_registry("bundle_nested_dependency_conflict");
    let node_modules = root.join("node_modules");
    let outer = node_modules.join("outer");
    let nested_shared = outer.join("node_modules/shared");
    let top_level_shared = node_modules.join("shared");
    fs::create_dir_all(&nested_shared).unwrap();
    fs::create_dir_all(&top_level_shared).unwrap();
    fs::write(
        outer.join("package.json"),
        r#"{"name":"outer","dependencies":{"shared":"^2.0.0"}}"#,
    )
    .unwrap();
    fs::write(
        outer.join("index.js"),
        "module.exports = require('shared');",
    )
    .unwrap();
    fs::write(
        nested_shared.join("package.json"),
        r#"{"name":"shared","version":"2.0.0","main":"index.js"}"#,
    )
    .unwrap();
    fs::write(
        nested_shared.join("index.js"),
        "module.exports = 'nested-v2';",
    )
    .unwrap();
    fs::write(
        top_level_shared.join("package.json"),
        r#"{"name":"shared","version":"1.0.0","main":"index.js"}"#,
    )
    .unwrap();
    fs::write(
        top_level_shared.join("index.js"),
        "module.exports = 'wrong-top-level-v1';",
    )
    .unwrap();

    let (bundle, _, file_count, versions) =
        bundle_commonjs_package(&node_modules, "outer", &outer, "index.js").unwrap();
    assert_eq!(file_count, 2, "outer/index.js + the nested shared/index.js");
    assert!(bundle.contains("nested-v2"), "{bundle}");
    assert!(!bundle.contains("wrong-top-level-v1"), "{bundle}");
    assert_eq!(versions.get("shared").map(String::as_str), Some("2.0.0"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn installed_package_unwraps_a_declare_global_augmentation() {
    // A package whose entire API lives inside `declare global { namespace
    // X { ... } }` -- real-world example: `@types/crypto-js`, whose
    // `export = CryptoJS;` is paired with one big `declare global {
    // namespace CryptoJS { function SHA256(...): ...; ... } }`. Nothing
    // unwrapped the `declare global` block, so the namespace had no
    // top-level declaration for `export = CryptoJS` to expose members
    // from, and every `CryptoJS.SHA256(...)` failed ("call to unknown
    // function").
    let scratch = temp_registry("installed-dts-declare-global-scratch");
    let registry = temp_registry("installed-dts-declare-global-registry");
    let package = scratch.join("node_modules/global-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"global-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "export = GlobalKit;\n\
         declare global {\n\
             namespace GlobalKit {\n\
                 function version(): string;\n\
             }\n\
         }\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { version: function() { return '1.0.0'; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "global-kit").unwrap();
    let declarations = resolve(&registry, "global-kit").unwrap().dts_source;
    assert!(
        !declarations.contains("declare global"),
        "the `declare global` wrapper should have been unwrapped:\n{declarations}"
    );
    assert!(
        declarations.contains("namespace GlobalKit"),
        "the hoisted namespace body should be present:\n{declarations}"
    );
    assert!(
        declarations.contains("function version"),
        "the hoisted namespace body should be present:\n{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_follows_a_dcts_esm_default_delegation() {
    // A CommonJS `.d.cts` entry that delegates its whole API to a sibling
    // ESM declaration file via `declare const X: typeof import("./x.mjs").
    // default; export = X;` -- real-world example: markdown-it's
    // `dist/markdown-it.d.cts` + `dist/markdown-it.d.mts`. The `.d.mts` holds
    // the real class, so without following the delegation `new MarkdownIt()`
    // had no constructor.
    let scratch = temp_registry("installed-dts-dcts-delegation-scratch");
    let registry = temp_registry("installed-dts-dcts-delegation-registry");
    let package = scratch.join("node_modules/dual-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"dual-kit","version":"1.0.0","types":"./index.d.cts","main":"./index.cjs"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.cts"),
        "declare namespace Kit {\n\
             type Options = import(\"./impl.mjs\").Options;\n\
         }\n\
         declare const Kit: typeof import(\"./impl.mjs\").default;\n\
         export = Kit;\n",
    )
    .unwrap();
    fs::write(
        package.join("impl.d.mts"),
        "export interface Options { value: number; }\n\
         declare class Kit {\n\
             constructor(options?: Options);\n\
             run(): string;\n\
         }\n\
         export { Kit as default };\n",
    )
    .unwrap();
    fs::write(package.join("index.cjs"), "module.exports = class Kit {};\n").unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "dual-kit").unwrap();
    let declarations = resolve(&registry, "dual-kit").unwrap().dts_source;
    assert!(
        declarations.contains("class Kit"),
        "the delegated ESM declaration should be flattened in:\n{declarations}"
    );
    assert!(
        declarations.contains("constructor(options?: Options)"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_declarations_resolve_multihop_function_aliases_and_explicit_module_extensions() {
    // Unrun regression: each re-export hop must rename the signature, and
    // .mjs/.cjs specifiers must find their .d.mts/.d.cts declarations.
    let scratch = temp_registry("installed-dts-multihop-alias-scratch");
    let registry = temp_registry("installed-dts-multihop-alias-registry");
    let package = scratch.join("node_modules/alias-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"alias-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"),
        "export { public as api } from './middle.mjs';\n\
         export { original as null } from './functions.cjs';\n").unwrap();
    fs::write(package.join("middle.d.mts"),
        "export { local as public } from './functions.cjs';\n").unwrap();
    fs::write(package.join("functions.d.cts"),
        "declare function local(value: string): string;\n\
         export { local };\n\
         export declare function original(value: number): number;\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = {};\n").unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "alias-kit").unwrap();
    let declarations = resolve(&registry, "alias-kit").unwrap().dts_source;
    assert!(declarations.contains("declare function api(value: string): string"), "{declarations}");
    assert!(declarations.contains("function original(value: number): number"), "{declarations}");
    assert!(declarations.contains("export { __thaw_public_6e756c6c_"), "{declarations}");
    assert!(declarations.contains(" as null };"), "{declarations}");
    assert!(!declarations.contains("function null("), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn reserved_aliases_from_distinct_modules_keep_distinct_internal_signatures() {
    // Unrun regression: both modules use internal `f`, but their public
    // reserved names and parameter types must not be merged.
    let scratch = temp_registry("installed-dts-reserved-collision-scratch");
    let registry = temp_registry("installed-dts-reserved-collision-registry");
    let package = scratch.join("node_modules/collision-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"collision-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"),
        "export * from './a.js';\nexport * from './b.js';\n").unwrap();
    fs::write(package.join("a.d.ts"),
        "export declare function f(value: string): string;\nexport { f as null };\n").unwrap();
    fs::write(package.join("b.d.ts"),
        "declare function f(value: number): number;\nexport { f as void };\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = {};\n").unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "collision-kit").unwrap();
    let declarations = resolve(&registry, "collision-kit").unwrap().dts_source;
    let public_binding = |alias: &str| {
        declarations.lines().find_map(|line| {
            line.trim().strip_prefix("export { ")
                .and_then(|line| line.strip_suffix(&format!(" as {alias} }};")))
        }).unwrap().to_string()
    };
    let null_binding = public_binding("null");
    let void_binding = public_binding("void");
    assert!(null_binding.starts_with("__thaw_public_6e756c6c_"), "{declarations}");
    assert!(void_binding.starts_with("__thaw_public_766f6964_"), "{declarations}");
    assert_ne!(null_binding, void_binding);
    assert!(declarations.contains(&format!("function {null_binding}(value: string): string")), "{declarations}");
    assert!(declarations.contains(&format!("function {void_binding}(value: number): number")), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn non_callable_value_alias_extracts_only_the_selected_declarator() {
    // Unrun regression: the public names must inherit the second value's
    // string type without renaming the unrelated first declarator.
    let scratch = temp_registry("installed-dts-value-multidecl-scratch");
    let registry = temp_registry("installed-dts-value-multidecl-registry");
    let package = scratch.join("node_modules/value-multidecl-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"value-multidecl-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"),
        "export { second as chosen, lb as selectedLet, vb as selectedVar } from './impl.js';\n").unwrap();
    fs::write(package.join("impl.d.ts"),
        "export declare const first: number, second: string;\n\
         export declare let la: number, lb: string;\n\
         export declare var va: number, vb: string;\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = {};\n").unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "value-multidecl-kit").unwrap();
    let declarations = resolve(&registry, "value-multidecl-kit").unwrap().dts_source;
    assert!(declarations.contains("const chosen: string;"), "{declarations}");
    assert!(declarations.contains("let selectedLet: string;"), "{declarations}");
    assert!(declarations.contains("var selectedVar: string;"), "{declarations}");
    assert!(!declarations.contains("const chosen: number"), "{declarations}");
    assert!(!declarations.contains("const first: number, chosen"), "{declarations}");
    assert!(thaw_parser::parse_declarations(&declarations).is_ok(), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn callable_const_alias_extracts_only_its_own_declarator() {
    // Unrun regression: selecting `b` must not rename the first `a` in the
    // same statement or borrow `a`'s number-returning signature.
    let scratch = temp_registry("installed-dts-multidecl-scratch");
    let registry = temp_registry("installed-dts-multidecl-registry");
    let package = scratch.join("node_modules/multidecl-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"),
        r#"{"name":"multidecl-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#).unwrap();
    fs::write(package.join("index.d.ts"),
        "export { b as c, ib as ic, lb as lc, lb as void, vb as vc, vb as null } from './impl.js';\n").unwrap();
    fs::write(package.join("impl.d.ts"),
        "export interface NumberCall { (): number; }\n\
         export interface StringCall { (): string; }\n\
         export declare const a: () => number, b: () => string;\n\
         export declare const ia: NumberCall, ib: StringCall;\n\
         export declare let la: () => number, lb: () => string;\n\
         export declare var va: NumberCall, vb: StringCall;\n").unwrap();
    fs::write(package.join("index.js"), "module.exports = {};\n").unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "multidecl-kit").unwrap();
    let declarations = resolve(&registry, "multidecl-kit").unwrap().dts_source;
    assert!(declarations.contains("const c: () => string;"), "{declarations}");
    assert!(declarations.contains("const ic: StringCall;"), "{declarations}");
    assert!(declarations.contains("let lc: () => string;"), "{declarations}");
    assert!(declarations.contains("var vc: StringCall;"), "{declarations}");
    assert!(declarations.contains("let __thaw_public_766f6964_"), "{declarations}");
    assert!(declarations.contains(" as void };"), "{declarations}");
    assert!(declarations.contains("var __thaw_public_6e756c6c_"), "{declarations}");
    assert!(declarations.contains(" as null };"), "{declarations}");
    assert!(!declarations.contains("const a: () => number, c:"), "{declarations}");
    assert!(!declarations.contains("const ia: NumberCall, ic:"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn reexported_type_records_preserve_terminal_origin_without_changing_output() {
    // Unrun precursor regression: wildcard and type namespace formatting
    // retain the source-owned children for the later private closure pass.
    let dir = temp_registry("owned-type-reexports");
    fs::write(dir.join("leaf.d.ts"),
        "export interface Model { id: Id; }\nexport type Id = string;\n").unwrap();
    fs::write(dir.join("entry.d.ts"),
        "export * from './leaf.js';\nexport type * as Types from './leaf.js';\n").unwrap();
    let entry = dir.join("entry.d.ts");
    let mut owned_visited = std::collections::BTreeSet::new();
    let owned = all_reexported_type_declarations_owned(&entry, &mut owned_visited).unwrap();
    assert!(thaw_parser::parse_declarations(&owned.iter().map(|declaration|
        declaration.snippet.as_str()).collect::<Vec<_>>().join("\n")).is_ok());
    let leaf = dir.join("leaf.d.ts").canonicalize().unwrap();
    assert!(owned.iter().any(|declaration| declaration.origin == leaf
        && declaration.local_name.as_deref() == Some("Model")));
    let namespace = owned.iter().find(|declaration| declaration.local_name.as_deref() == Some("Types")).unwrap();
    assert!(namespace.children.iter().any(|child| child.origin == leaf
        && child.local_name.as_deref() == Some("Model")));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn named_reexport_records_keep_original_owner_and_selected_alias_order() {
    // Unrun precursor regression: the selected class and imported Base
    // retain separate owners even though the returned text is unchanged.
    let dir = temp_registry("owned-named-reexports");
    fs::write(dir.join("base.d.ts"),
        "export declare class Base { inherited(): string; }\n").unwrap();
    fs::write(dir.join("leaf.d.ts"),
        "import { Base as LocalBase } from './base.js';\nexport declare class Model extends LocalBase {}\n").unwrap();
    let leaf = dir.join("leaf.d.ts");
    let owned = reexported_class_or_interface_declarations_owned(&leaf, "Model").unwrap();
    assert!(owned.first().unwrap().snippet.contains("class Model extends LocalBase"));
    assert_eq!(owned.first().and_then(|declaration| declaration.local_name.as_deref()), Some("Model"));
    assert_eq!(owned.first().unwrap().origin, leaf.canonicalize().unwrap());
    assert!(owned.iter().skip(1).any(|declaration| declaration.origin == dir.join("base.d.ts").canonicalize().unwrap()
        && declaration.local_name.as_deref() == Some("Base")));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn function_reexport_records_keep_leaf_owner_through_named_and_namespace_paths() {
    // Unrun precursor regression: public alias spelling does not replace
    // the leaf identity needed to resolve private signature types.
    let dir = temp_registry("owned-function-reexports");
    fs::write(dir.join("leaf.d.ts"),
        "export interface Options { value: string; }\n\
         export declare function make(options: Options): string;\n").unwrap();
    fs::write(dir.join("entry.d.ts"),
        "export { make as create } from './leaf.js';\n\
         export * as api from './leaf.js';\n").unwrap();
    let leaf = dir.join("leaf.d.ts").canonicalize().unwrap();
    let mut visited = std::collections::BTreeSet::new();
    let named = all_reexported_function_declarations_owned(&dir.join("entry.d.ts"), &mut visited).unwrap();
    let (public, selected) = named.iter().find(|(public, _)| public == "create").unwrap();
    assert_eq!(public, "create");
    assert_eq!(selected.origin, leaf);
    assert_eq!(selected.local_name.as_deref(), Some("make"));
    assert!(selected.snippet.contains("function create(options: Options): string"));
    let mut visited = std::collections::BTreeSet::new();
    let namespaced = all_reexported_function_declarations_owned(&dir.join("leaf.d.ts"), &mut visited).unwrap();
    assert!(namespaced.iter().any(|(name, declaration)| name == "make"
        && declaration.origin == leaf && declaration.local_name.as_deref() == Some("make")));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn namespace_renderer_keeps_function_and_type_member_sources() {
    // Unrun precursor regression: exercise the same owner-aware renderer
    // used by an ordinary `export * as api` entry, not just its collector.
    let dir = temp_registry("owned-namespace-renderer");
    fs::write(dir.join("leaf.d.ts"),
        "export interface Options { value: string; }\n\
         export declare function make(options: Options): string;\n").unwrap();
    fs::write(dir.join("entry.d.ts"), "export * as api from './leaf.js';\n").unwrap();
    let entry = dir.join("entry.d.ts");
    let leaf = dir.join("leaf.d.ts").canonicalize().unwrap();
    let flattened = dts_source_with_reexported_functions(&entry,
        &fs::read_to_string(&entry).unwrap()).unwrap();
    assert!(flattened.contains("declare namespace api {"), "{flattened}");
    assert!(flattened.contains("export interface Options"), "{flattened}");
    assert!(flattened.contains("export function make(options: Options): string"), "{flattened}");
    assert!(thaw_parser::parse_declarations(&flattened).is_ok(), "{flattened}");
    let mut visited = std::collections::BTreeSet::new();
    let functions = all_reexported_function_declarations_owned(&leaf, &mut visited).unwrap();
    let rendered = namespace_member_declarations_owned(functions.into_iter().next().unwrap().1).unwrap();
    assert!(rendered.iter().any(|member| member.origin == leaf
        && member.local_name.as_deref() == Some("make")
        && member.snippet.contains("function make(options: Options): string")));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn import_equals_export_assignment_records_keep_terminal_owner() {
    // Unrun precursor regression: `export =` function/class targets keep
    // the target file, while the barrel retains its public alias.
    let dir = temp_registry("owned-import-equals");
    fs::write(dir.join("function.d.ts"),
        "declare function make(value: string): string;\nexport = make;\n").unwrap();
    fs::write(dir.join("class.d.ts"),
        "declare class Client { read(): string; }\nexport = Client;\n").unwrap();
    fs::write(dir.join("entry.d.ts"),
        "import Maker = require('./function.js');\n\
         import Client = require('./class.js');\n\
         export { Maker as create, Client };\n").unwrap();
    let entry = dir.join("entry.d.ts");
    let flattened = dts_source_with_reexported_functions(&entry,
        &fs::read_to_string(&entry).unwrap()).unwrap();
    assert!(flattened.contains("function create(value: string): string"), "{flattened}");
    assert!(flattened.contains("class Client"), "{flattened}");
    let function = export_assignment_function_declarations_owned(&dir.join("function.d.ts")).unwrap();
    assert_eq!(function[0].origin, dir.join("function.d.ts").canonicalize().unwrap());
    assert_eq!(function[0].local_name.as_deref(), Some("make"));
    let class = export_assignment_class_or_interface_declarations_owned(&dir.join("class.d.ts")).unwrap();
    assert_eq!(class[0].origin, dir.join("class.d.ts").canonicalize().unwrap());
    assert_eq!(class[0].local_name.as_deref(), Some("Client"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn owned_public_alias_separates_export_name_from_structural_binding() {
    // Unrun closure regression: an alias marker selects the source Model,
    // but Renamed is the exported spelling and Model is the local type name.
    let origin = Path::new("/tmp/owned-alias-source.d.ts");
    let structural = OwnedDeclaration::new(origin, "declare class Model { id: string; }".into());
    let marker = structural.clone().with_snippet("export { Model as Renamed };".into());
    assert_eq!(namespace_member_public_names(&marker), vec!["Renamed"]);
    let record = EmittedOwnedDeclaration {
        declaration: marker,
        scope: Some("api".into()),
        public_names: vec!["Renamed".into()],
        start: 0,
        end: 0,
        metadata_only: false,
    };
    let occurrences = public_type_occurrences(&[record]);
    let selected = &occurrences[&(origin.to_path_buf(), Vec::new(), "Model".into())][0];
    assert_eq!(selected.scope.as_deref(), Some("api"));
    assert_eq!(selected.structural_name, "Model");
    assert_eq!(selected.public_name, "Renamed");
    assert!(selected.value_export);
    assert_eq!(selected.type_reference_from(Some("api")), "Model");
    assert_eq!(selected.type_reference_from(Some("other")), "api.Renamed");
    assert_eq!(selected.type_reference_from(None), "api.Renamed");
}

#[test]
fn owned_source_namespace_keys_keep_same_named_private_members_distinct() {
    // Unrun metadata regression: same-file namespace-local names must not
    // collapse into one support origin key after flattening.
    let origin = Path::new("/tmp/owned-scopes-source.d.ts");
    let outer = OwnedDeclaration::new(origin,
        "export declare namespace Outer { export interface Id { outer: string; } export { Id as PublicId }; export namespace Inner { export interface Id { inner: number; } } }".into());
    let first = outer.children.iter().find(|child| child.local_name.as_deref() == Some("Id")).unwrap();
    let nested = outer.children.iter().find(|child| child.local_name.as_deref() == Some("Inner")).unwrap();
    let second = nested.children.iter().find(|child| child.local_name.as_deref() == Some("Id")).unwrap();
    assert_eq!(first.source_scope, vec!["Outer"]);
    assert_eq!(second.source_scope, vec!["Outer", "Inner"]);
    let alias = outer.children.iter().find(|child| child.snippet.contains("Id as PublicId")).unwrap();
    assert_eq!(alias.source_scope, vec!["Outer"]);
    assert_eq!(alias.local_name.as_deref(), Some("Id"));
    assert_eq!(namespace_member_public_names(alias), vec!["PublicId"]);
    assert_ne!(
        (first.origin.clone(), first.source_scope.clone(), first.local_name.clone()),
        (second.origin.clone(), second.source_scope.clone(), second.local_name.clone()),
    );
}

#[test]
fn source_type_lookup_prefers_nearest_lexical_namespace() {
    // Unrun resolver regression: Inner.Id must not silently bind Outer.Id.
    let dir = temp_registry("owned-private-lexical-lookup");
    let path = dir.join("types.d.ts");
    fs::write(&path,
        "declare namespace Outer { interface Id { outer: string; } namespace Inner { interface Id { inner: number; } interface Model { id: Id; } } }",
    ).unwrap();
    let table = source_type_bindings(&path).unwrap();
    let outer = resolve_lexical_source_type(&table, &path, &["Outer".into()], "Id").unwrap();
    let inner = resolve_lexical_source_type(&table, &path, &["Outer".into(), "Inner".into()], "Id").unwrap();
    assert_eq!(outer[0].source_scope, vec!["Outer"]);
    assert_eq!(inner[0].source_scope, vec!["Outer", "Inner"]);
    assert_ne!(outer[0].snippet, inner[0].snippet);
    assert!(resolve_lexical_source_type(&table, &path, &["Other".into()], "Id").is_none());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn source_type_lookup_keeps_merged_interface_fragments() {
    // Unrun resolver regression: a lexical key can have multiple declarations.
    let dir = temp_registry("owned-private-merged-interface");
    let path = dir.join("types.d.ts");
    fs::write(&path,
        "declare namespace Outer { interface Id { first: string; } interface Id { second: number; } }",
    ).unwrap();
    let table = source_type_bindings(&path).unwrap();
    let fragments = resolve_lexical_source_type(&table, &path, &["Outer".into()], "Id").unwrap();
    assert_eq!(fragments.len(), 2);
    assert!(fragments[0].snippet.contains("first"));
    assert!(fragments[1].snippet.contains("second"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn imported_private_type_lookup_keeps_terminal_owner_separate_from_superclass() {
    // Unrun graph regression: the dependency is followed as its own node.
    let dir = temp_registry("owned-private-relative-import");
    let helper = dir.join("helper.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&helper,
        "declare class Base { inherited(): string; }\nexport declare class Key extends Base { value: number; }\n",
    ).unwrap();
    fs::write(&entry,
        "import type { Key as Id } from './helper';\nexport interface Model { id: Id; }\n",
    ).unwrap();
    let requested = resolve_owned_source_type_reference(&entry, &[], "Id").unwrap();
    assert!(!requested.is_empty());
    assert!(requested.iter().all(|declaration| declaration.local_name.as_deref() == Some("Key")));
    assert!(requested.iter().all(|declaration| declaration.origin == helper.canonicalize().unwrap()));
    assert!(!requested.iter().any(|declaration| declaration.snippet.contains("class Base")));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn owned_type_reference_ranges_ignore_identifiers_and_type_parameter_shadows() {
    // Unrun span regression for the shared imported-alias and closure visitor.
    let origin = std::path::Path::new("types.d.ts");
    let snippet = "export interface Model extends Base { id: Id; Id: string; method<Id>(value: Id): Base; }";
    let base = type_reference_ranges(snippet, "Base", origin).unwrap();
    let id = type_reference_ranges(snippet, "Id", origin).unwrap();
    assert_eq!(base.len(), 2);
    assert_eq!(id.len(), 1);
    assert_eq!(&snippet[id[0].0..id[0].1], "Id");
    let shadowed = "export interface Box<Id> { id: Id; }";
    assert!(type_reference_ranges(shadowed, "Id", origin).unwrap().is_empty());
}

#[test]
fn type_query_value_shadows_are_distinct_from_generic_type_shadows() {
    // Unrun visitor regression: a generic type parameter Foo hides the type
    // use, while `typeof Foo` still names the outer value declaration.
    let origin = std::path::Path::new("types.d.ts");
    let generic = "declare const Foo: number; export interface Holder<Foo> { ctor: typeof Foo; item: Foo; }";
    let sites = type_reference_sites(generic, "Foo", origin).unwrap();
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].kind, TypeReferenceKind::ValueQuery);
    assert_eq!(&generic[sites[0].full.0..sites[0].full.1], "Foo");
    let parameter = "export declare function use(config: number): typeof config;";
    assert!(type_reference_sites(parameter, "config", origin).unwrap().is_empty());
}

#[test]
fn explicit_external_value_export_does_not_fall_back_to_local_binding() {
    // Unrun owner regression: a source-qualified export has no local
    // fallback even when the file declares an identically named helper.
    let dir = temp_registry("owned-external-value-export");
    let path = dir.join("index.d.ts");
    fs::write(&path,
        "declare function make(): string;\nexport { make } from 'external-package';\n",
    ).unwrap();
    let found = exported_owned_value_declarations(
        &path, "make", &mut std::collections::BTreeSet::new(),
    ).unwrap();
    assert!(found.is_empty());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn owned_type_reference_edges_keep_import_alias_and_terminal_origin() {
    // Unrun closure-graph regression: local spelling is not the owner key.
    let dir = temp_registry("owned-private-reference-edges");
    let helper = dir.join("helper.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&helper, "export interface Key { id: string; }\n").unwrap();
    let model = "export interface Model { id: Id; }";
    fs::write(&entry, format!("import type {{ Key as Id }} from './helper';\n{model}\n")).unwrap();
    let owned = OwnedDeclaration::new(&entry, model.to_string());
    let edges = owned_type_reference_edges(&owned).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].local_spelling, "Id");
    assert_eq!(&model[edges[0].range.0..edges[0].range.1], "Id");
    assert_eq!(edges[0].terminal.0, helper.canonicalize().unwrap());
    assert_eq!(edges[0].terminal.2, "Key");
    assert_eq!(edges[0].fragments.len(), 1);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn owned_type_reference_edges_resolve_qualified_namespace_member_only() {
    // Unrun graph regression: `Types.Id` is one member, not the whole namespace.
    let dir = temp_registry("owned-private-qualified-edges");
    let helper = dir.join("helper.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&helper,
        "export declare namespace Types { export interface Id { value: string; } export interface Unused { other: number; } }\n",
    ).unwrap();
    let model = "export interface Model { id: Remote.Types.Id; }";
    fs::write(&entry, format!("import * as Remote from './helper';\n{model}\n")).unwrap();
    let owned = OwnedDeclaration::new(&entry, model.to_string());
    let edges = owned_type_reference_edges(&owned).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].local_spelling, "Remote");
    assert_eq!(&model[edges[0].full_range.0..edges[0].full_range.1], "Remote.Types.Id");
    assert_eq!(edges[0].terminal.0, helper.canonicalize().unwrap());
    assert_eq!(edges[0].terminal.1, vec!["Types"]);
    assert_eq!(edges[0].terminal.2, "Id");
    assert_eq!(edges[0].fragments.len(), 1);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn qualified_type_lookup_does_not_escape_a_shadowing_inner_namespace() {
    // Unrun lexical regression: Inner.A exists, so A.Id cannot bind outer A.Id.
    let dir = temp_registry("owned-private-qualified-shadow");
    let path = dir.join("types.d.ts");
    fs::write(&path,
        "declare namespace A { interface Id { outer: string; } namespace Inner { namespace A { interface Other { inner: number; } } interface Model { id: A.Id; } } }",
    ).unwrap();
    let unresolved = resolve_owned_qualified_source_type_reference(
        &path, &["A".into(), "Inner".into()], &["A".into(), "Id".into()],
    ).unwrap();
    assert!(unresolved.is_empty());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn qualified_type_lookup_follows_type_namespace_barrel_to_terminal_owner() {
    // Unrun barrel regression: import * then export type * as Types retains
    // the leaf identity, rather than treating a barrel as its owner.
    let dir = temp_registry("owned-qualified-type-barrel");
    let leaf = dir.join("leaf.d.ts");
    let barrel = dir.join("barrel.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&leaf, "export interface Id { leaf: string; }\n").unwrap();
    fs::write(&barrel, "export type * as Types from './leaf';\n").unwrap();
    fs::write(&entry,
        "import type * as Remote from './barrel';\nexport interface Model { id: Remote.Types.Id; }\n",
    ).unwrap();
    let found = resolve_owned_qualified_source_type_reference(
        &entry, &[], &["Remote".into(), "Types".into(), "Id".into()],
    ).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].origin, leaf.canonicalize().unwrap());
    assert_eq!(found[0].local_name.as_deref(), Some("Id"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn qualified_value_query_follows_namespace_barrel_to_terminal_owner() {
    // Unrun value-kind barrel regression for typeof Remote.Api.make.
    let dir = temp_registry("owned-qualified-value-barrel");
    let leaf = dir.join("leaf.d.ts");
    let barrel = dir.join("barrel.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&leaf, "export declare function make(): string;\n").unwrap();
    fs::write(&barrel, "export * as Api from './leaf';\n").unwrap();
    fs::write(&entry,
        "import * as Remote from './barrel';\nexport interface Model { maker: typeof Remote.Api.make; }\n",
    ).unwrap();
    let found = resolve_owned_qualified_source_value_reference(
        &entry, &[], &["Remote".into(), "Api".into(), "make".into()],
    ).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].origin, leaf.canonicalize().unwrap());
    assert_eq!(found[0].local_name.as_deref(), Some("make"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn qualified_import_uses_local_namespace_behind_public_alias() {
    // Unrun source regression: public Api is stored under source scope Hidden.
    let dir = temp_registry("owned-qualified-local-namespace-alias");
    let leaf = dir.join("leaf.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&leaf, "declare namespace Hidden { export interface Id { key: string; } export declare function make(): Id; interface Private {} }\nexport { Hidden as Api };\n").unwrap();
    fs::write(&entry, "import * as Remote from './leaf';\nexport interface Model { id: Remote.Api.Id; make: typeof Remote.Api.make; }\n").unwrap();
    let id = resolve_owned_qualified_source_type_reference(
        &entry, &[], &["Remote".into(), "Api".into(), "Id".into()],
    ).unwrap();
    assert_eq!(id.len(), 1);
    assert_eq!(id[0].origin, leaf.canonicalize().unwrap());
    assert_eq!(id[0].source_scope, vec!["Hidden"]);
    let make = resolve_owned_qualified_source_value_reference(
        &entry, &[], &["Remote".into(), "Api".into(), "make".into()],
    ).unwrap();
    assert_eq!(make.len(), 1);
    assert_eq!(make[0].source_scope, vec!["Hidden"]);
    fs::write(&entry, "import { Api as Remote } from './leaf';\nexport interface Model { id: Remote.Id; make: typeof Remote.make; }\n").unwrap();
    let named_id = resolve_owned_qualified_source_type_reference(
        &entry, &[], &["Remote".into(), "Id".into()],
    ).unwrap();
    assert_eq!(named_id.len(), 1);
    assert_eq!(named_id[0].source_scope, vec!["Hidden"]);
    let named_make = resolve_owned_qualified_source_value_reference(
        &entry, &[], &["Remote".into(), "make".into()],
    ).unwrap();
    assert_eq!(named_make.len(), 1);
    assert_eq!(named_make[0].source_scope, vec!["Hidden"]);
    assert!(resolve_owned_qualified_source_type_reference(
        &entry, &[], &["Remote".into(), "Private".into()],
    ).unwrap().is_empty());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn qualified_import_does_not_treat_an_exported_function_as_namespace() {
    // Unrun regression: a same-name exported function does not expose a
    // private namespace member through a coincidental lookup key.
    let dir = temp_registry("owned-qualified-function-root");
    let leaf = dir.join("leaf.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&leaf, "declare function make(): void;\nexport { make as Api };\ninterface Id {}\n").unwrap();
    fs::write(&entry, "import * as Remote from './leaf';\nexport interface Model { id: Remote.Api.Id; }\n").unwrap();
    assert!(resolve_owned_qualified_source_type_reference(
        &entry, &[], &["Remote".into(), "Api".into(), "Id".into()],
    ).unwrap().is_empty());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn type_only_namespace_alias_does_not_resolve_a_value_query() {
    // Unrun regression: `export type` permits Remote.Api.Id but cannot
    // expose `typeof Remote.Api.make` as a runtime value binding.
    let dir = temp_registry("owned-qualified-type-only-namespace");
    let leaf = dir.join("leaf.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&leaf, "declare namespace Hidden { export interface Id {} export declare function make(): void; }\nexport type { Hidden as Api };\n").unwrap();
    fs::write(&entry, "import type * as Remote from './leaf';\nexport interface Model { id: Remote.Api.Id; }\n").unwrap();
    assert_eq!(resolve_owned_qualified_source_type_reference(
        &entry, &[], &["Remote".into(), "Api".into(), "Id".into()],
    ).unwrap().len(), 1);
    assert!(resolve_owned_qualified_source_value_reference(
        &entry, &[], &["Remote".into(), "Api".into(), "make".into()],
    ).unwrap().is_empty());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn named_namespace_barrel_follows_renamed_terminal_scope() {
    // Unrun regression: source-qualified `Hidden as Api` keeps Hidden as
    // the terminal lexical scope across another named import alias.
    let dir = temp_registry("owned-named-namespace-barrel-alias");
    let leaf = dir.join("leaf.d.ts");
    let barrel = dir.join("barrel.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&leaf, "export declare namespace Hidden { export interface Id {} export declare function make(): Id; }\n").unwrap();
    fs::write(&barrel, "export { Hidden as Api } from './leaf';\n").unwrap();
    fs::write(&entry, "import { Api as Remote } from './barrel';\nexport interface Model { id: Remote.Id; make: typeof Remote.make; }\n").unwrap();
    let id = resolve_owned_qualified_source_type_reference(
        &entry, &[], &["Remote".into(), "Id".into()],
    ).unwrap();
    assert_eq!(id.len(), 1);
    assert_eq!(id[0].source_scope, vec!["Hidden"]);
    let make = resolve_owned_qualified_source_value_reference(
        &entry, &[], &["Remote".into(), "make".into()],
    ).unwrap();
    assert_eq!(make.len(), 1);
    assert_eq!(make[0].source_scope, vec!["Hidden"]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn private_type_closure_keeps_only_referenced_source_owned_fragments() {
    // Unrun closure regression: colliding Id names stay keyed by origin.
    let dir = temp_registry("owned-private-reference-closure");
    let mut emitted = Vec::new();
    for (file, model, payload) in [("a.d.ts", "ModelA", "string"), ("b.d.ts", "ModelB", "number")] {
        let path = dir.join(file);
        let selected = format!("export interface {model} {{ id: Id; }}");
        fs::write(&path, format!(
            "interface Id {{ value: {payload}; }}\ninterface Unused {{ ignored: boolean; }}\n{selected}\n",
        )).unwrap();
        emitted.push(EmittedOwnedDeclaration {
            declaration: OwnedDeclaration::new(&path, selected.clone()), scope: None,
            public_names: vec![model.to_string()], start: 0, end: selected.len(),
            metadata_only: false,
        });
    }
    let public = public_type_occurrences(&emitted);
    let support = referenced_private_type_closure(&emitted, &public).unwrap();
    assert_eq!(support.types.len(), 2);
    assert!(support.values.is_empty());
    assert!(support.types.keys().all(|(_, _, name)| name == "Id"));
    assert_ne!(support.types.keys().next().unwrap().0, support.types.keys().next_back().unwrap().0);
    assert!(support.types.values().all(|fragments| fragments.len() == 1));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn flattened_wildcard_types_rewrite_colliding_private_references_into_support_namespaces() {
    // Unrun output regression: two private Id bindings remain distinct.
    let dir = temp_registry("owned-private-flat-output");
    let entry = dir.join("index.d.ts");
    let source = "export * from './a';\nexport * from './b';\n";
    fs::write(&entry, source).unwrap();
    fs::write(dir.join("a.d.ts"),
        "interface Id { value: string; }\nexport interface ModelA { id: Id; }\n",
    ).unwrap();
    fs::write(dir.join("b.d.ts"),
        "interface Id { value: number; }\nexport interface ModelB { id: Id; }\n",
    ).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert_eq!(output.matches("type __thaw_private_support_marker__ = never").count(), 2, "{output}");
    assert!(output.contains("export interface ModelA { id: __thaw_support_"), "{output}");
    assert!(output.contains("export interface ModelB { id: __thaw_support_"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn flattened_public_import_alias_rewrites_without_private_support() {
    // Unrun output regression: a selected public target needs remapping even
    // when the reference closure contains no private declaration.
    let dir = temp_registry("owned-public-alias-only");
    let entry = dir.join("index.d.ts");
    let source = "export { Model as Renamed } from './model';\nexport { Wrapper } from './wrapper';\n";
    fs::write(&entry, source).unwrap();
    fs::write(dir.join("model.d.ts"), "export interface Model { value: string; }\n").unwrap();
    fs::write(dir.join("wrapper.d.ts"),
        "import type { Model as Local } from './model';\nexport interface Wrapper { item: Local; }\n",
    ).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("item: Model"), "{output}");
    assert!(!output.contains("item: Local"), "{output}");
    assert!(!output.contains("__thaw_private_support_marker__"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn retained_entry_and_appended_types_rewrite_after_import_removal() {
    // Unrun interval regression: a materialized class import shortens the
    // retained entry prefix while appended declarations move with it.
    let dir = temp_registry("owned-entry-intervals");
    let entry = dir.join("index.d.ts");
    let source = "import Base from './base';\ninterface EntryId { value: string; }\nexport class EntryModel extends Base { id: EntryId; }\nexport { LeafModel } from './leaf';\n";
    fs::write(&entry, source).unwrap();
    fs::write(dir.join("base.d.ts"), "export default class Base { ready(): boolean; }\n").unwrap();
    fs::write(dir.join("leaf.d.ts"),
        "interface LeafId { value: number; }\nexport interface LeafModel { id: LeafId; }\n",
    ).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("class EntryModel extends __thaw_support_"), "{output}");
    assert!(output.contains(".Base { id: __thaw_support_"), "{output}");
    assert!(output.contains("interface LeafModel { id: __thaw_support_"), "{output}");
    assert_eq!(output.matches("type __thaw_private_support_marker__ = never").count(), 3, "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn value_namespace_members_rewrite_distinct_private_type_owners() {
    // Unrun namespace-renderer regression: child intervals must be rewritten
    // inside each wrapper while the wrappers remain public value namespaces.
    let dir = temp_registry("owned-private-namespace-output");
    let entry = dir.join("index.d.ts");
    let source = "export * as left from './left';\nexport * as right from './right';\n";
    fs::write(&entry, source).unwrap();
    fs::write(dir.join("left.d.ts"),
        "interface Id { left: string; }\nexport interface Model { id: Id; }\n",
    ).unwrap();
    fs::write(dir.join("right.d.ts"),
        "interface Id { right: number; }\nexport interface Model { id: Id; }\n",
    ).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    for name in ["left", "right"] {
        let body = output.split(&format!("declare namespace {name} {{")).nth(1)
            .unwrap().split("\n}").next().unwrap();
        assert!(body.contains("export interface Model { id: __thaw_support_"), "{output}");
    }
    assert_eq!(output.matches("type __thaw_private_support_marker__ = never").count(), 2, "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn public_namespace_class_alias_reuses_selected_type_spelling() {
    // Unrun alias-marker regression: a bare source class followed by an
    // export marker is still a selected public type occurrence.
    let dir = temp_registry("owned-public-namespace-class-alias");
    let entry = dir.join("index.d.ts");
    let source = "export * as api from './barrel';\nexport { Model } from './model';\n";
    fs::write(&entry, source).unwrap();
    fs::write(dir.join("barrel.d.ts"),
        "export { Hidden as PublicClient } from './client';\n",
    ).unwrap();
    fs::write(dir.join("client.d.ts"), "export class Hidden { ping(): string; }\n").unwrap();
    fs::write(dir.join("model.d.ts"),
        "import { Hidden as Local } from './client';\nexport interface Model { client: Local; }\n",
    ).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("client: api.PublicClient"), "{output}");
    assert!(!output.contains("client: Local"), "{output}");
    assert!(!output.contains("__thaw_private_support_marker__"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn private_type_query_value_and_signature_type_share_owned_support_namespace() {
    // Unrun value-kind closure regression: typeof names a value, whose
    // signature in turn pulls only its referenced private type.
    let dir = temp_registry("owned-private-type-query");
    let entry = dir.join("index.d.ts");
    let source = "export { Model } from './leaf';\n";
    fs::write(&entry, source).unwrap();
    fs::write(dir.join("leaf.d.ts"),
        "interface Id { value: string; }\ndeclare function make(): Id;\nexport interface Model { maker: typeof make; }\n",
    ).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("maker: typeof __thaw_support_"), "{output}");
    assert!(output.contains("export function make(): __thaw_support_"), "{output}");
    assert!(output.contains("export interface Id { value: string; }"), "{output}");
    assert_eq!(output.matches("type __thaw_private_support_marker__ = never").count(), 1, "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn private_type_query_copies_only_selected_variable_declarator() {
    // Unrun multi-var regression: an unrelated declarator is never pulled
    // into the private support surface by `typeof config`.
    let dir = temp_registry("owned-private-type-query-var");
    let entry = dir.join("index.d.ts");
    let source = "export { Model } from './leaf';\n";
    fs::write(&entry, source).unwrap();
    fs::write(dir.join("leaf.d.ts"),
        "interface Id { value: string; }\ndeclare const config: Id, unused: boolean;\nexport interface Model { setting: typeof config; }\n",
    ).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("setting: typeof __thaw_support_"), "{output}");
    assert!(output.contains("export const config: __thaw_support_"), "{output}");
    assert!(!output.contains("unused: boolean"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn imported_equals_inlined_type_keeps_its_private_helper_owner() {
    // Unrun generated-output regression: both the retained entry and the
    // directly appended sibling body must be rewritten from known spans.
    let dir = temp_registry("owned-import-equals-inline-helper");
    let entry = dir.join("entry.d.ts");
    let sibling = dir.join("mail.d.ts");
    fs::write(&entry,
        "import Mail = require('./mail');\nexport interface Model { mail: Mail; }\n",
    ).unwrap();
    fs::write(&sibling,
        "interface Id { value: string; }\ndeclare class Mail { id: Id; }\nexport = Mail;\n",
    ).unwrap();
    let flattened = dts_source_with_reexported_functions(
        &entry, &fs::read_to_string(&entry).unwrap(),
    ).unwrap();
    assert!(flattened.contains("export interface Model { mail: __thaw_support_"), "{flattened}");
    assert!(flattened.contains("export class Mail { id: __thaw_support_"), "{flattened}");
    assert!(flattened.contains("export interface Id { value: string; }"), "{flattened}");
    assert!(thaw_parser::parse_declarations(&flattened).is_ok(), "{flattened}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn builtin_transform_materialization_carries_referenced_option_types() {
    // Unrun synthetic-source regression: requested builtin classes have no
    // file owner, but their same-source constructor types must survive.
    let source = resolve_builtin("stream").unwrap().dts_source;
    let snippets = builtin_class_and_ancestor_declarations(
        &source, "stream", "Transform", &mut std::collections::BTreeSet::new(),
    ).unwrap();
    assert!(snippets.iter().any(|snippet| snippet.contains("class Transform extends Duplex")));
    assert!(snippets.iter().any(|snippet| snippet.contains("interface TransformOptions")));
    assert!(snippets.iter().any(|snippet| snippet.contains("interface TransformFlushOptions")));
    assert!(snippets.iter().any(|snippet| snippet.contains("interface DuplexOptions")));
    let flattened = snippets.join("\n");
    assert!(thaw_parser::parse_declarations(&flattened).is_ok(), "{flattened}");
}

#[test]
fn builtin_sibling_types_are_materialized_once_per_builtin_origin() {
    // Unrun same-source dedup regression: two derived classes share
    // Transform, Duplex, and their option types.
    let dir = temp_registry("owned-builtin-sibling-dedup");
    let entry = dir.join("index.d.ts");
    let source = "import * as stream from 'node:stream';\nexport declare class First extends stream.Transform {}\nexport declare class Second extends stream.PassThrough {}\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert_eq!(output.matches("interface TransformOptions").count(), 1, "{output}");
    assert_eq!(output.matches("interface DuplexOptions").count(), 1, "{output}");
    assert_eq!(output.matches("class Transform extends Duplex").count(), 1, "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn builtin_signature_siblings_include_generic_and_property_types() {
    // Unrun non-stream regressions: a generic result and a sibling class
    // property are both ordinary same-source type references.
    let reporters = resolve_builtin("test/reporters").unwrap().dts_source;
    let reporter = builtin_class_and_ancestor_declarations(
        &reporters, "test/reporters", "Reporter", &mut std::collections::BTreeSet::new(),
    ).unwrap();
    assert!(reporter.iter().any(|snippet| snippet.contains("interface ReporterResult")));
    let url = resolve_builtin("url").unwrap().dts_source;
    let url_class = builtin_class_and_ancestor_declarations(
        &url, "url", "URL", &mut std::collections::BTreeSet::new(),
    ).unwrap();
    assert!(url_class.iter().any(|snippet| snippet.contains("class URLSearchParams")));
}

#[test]
fn retained_multivariable_second_binding_keeps_public_value_identity() {
    // Unrun same-statement metadata regression: b is a public value even
    // though declaration_identity of the full var statement returns a.
    let dir = temp_registry("owned-retained-multivar-public");
    let entry = dir.join("index.d.ts");
    let source = "interface Id { value: string; }\nexport declare const a: number, b: Id;\nexport interface Model { selected: typeof b; }\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("selected: typeof b"), "{output}");
    assert!(output.contains("b: __thaw_support_"), "{output}");
    assert!(!output.contains("selected: typeof __thaw_support_"), "{output}");
    assert_eq!(output.matches("type __thaw_private_support_marker__ = never").count(), 1, "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn default_class_owner_matches_the_materialized_import_equals_spelling() {
    // Unrun provenance regression: `inline_import_equals_value_type`
    // changes export-default syntax, but the owner stays the source class.
    let dir = temp_registry("owned-default-class-materialization");
    let leaf = dir.join("mime.d.ts");
    fs::write(&leaf, "interface Id {}\nexport default class Mime { id: Id; }\n").unwrap();
    let owner = leaf.canonicalize().unwrap();
    let table = source_type_bindings(&leaf).unwrap();
    let mime = table.get(&(owner.clone(), Vec::new(), "Mime".to_string())).unwrap();
    assert_eq!(mime.len(), 1);
    assert!(mime[0].snippet.contains("declare class Mime"));
    assert_eq!(mime[0].origin, owner);
    let appended = "export default class Mime { id: Id; }";
    let mut records = Vec::new();
    record_owned_entry_declarations(&mut records, &leaf, appended, 11).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].declaration.local_name.as_deref(), Some("Mime"));
    assert_eq!(records[0].public_names, vec!["default"]);
    assert_eq!((records[0].start, records[0].end), (11, 11 + appended.len()));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn triple_slash_direct_append_retains_referenced_file_type_owner() {
    // Unrun appended-origin regression: the referenced source owns both
    // its public model and private Id, even after entry-prefix replacement.
    let dir = temp_registry("owned-triple-slash-direct-append");
    let entry = dir.join("index.d.ts");
    fs::write(&entry, "/// <reference path=\"./leaf.d.ts\" />\n").unwrap();
    fs::write(dir.join("leaf.d.ts"),
        "interface Id { value: string; }\nexport interface Model { id: Id; }\n",
    ).unwrap();
    let output = dts_source_with_reexported_functions(
        &entry, &fs::read_to_string(&entry).unwrap(),
    ).unwrap();
    assert!(output.contains("export interface Model { id: __thaw_support_"), "{output}");
    assert!(output.contains("export interface Id { value: string; }"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn hoisted_export_equals_member_keeps_original_namespace_owner() {
    // Unrun hoist regression: the flat public member came from Api, so
    // its private Id still resolves in that source namespace.
    let dir = temp_registry("owned-hoisted-export-equals-helper");
    let entry = dir.join("index.d.ts");
    let source = "declare namespace Api { interface Id { value: string; } export interface Model { id: Id; } }\nexport = Api;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("export interface Model { id: __thaw_support_"), "{output}");
    assert!(output.contains("export namespace Api {"), "{output}");
    assert!(output.contains("export interface Id { value: string; }"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn resolved_hoisted_import_equals_keeps_terminal_function_owner() {
    // Unrun resolved-hoist regression: a renamed import-equals member
    // carries the leaf function's signature and private helper owner.
    let dir = temp_registry("owned-hoisted-import-equals-helper");
    let entry = dir.join("index.d.ts");
    fs::write(dir.join("leaf.d.ts"),
        "interface Id { value: string; }\nexport declare function make(id: Id): Id;\n",
    ).unwrap();
    let source = "import { make as Local } from './leaf';\ndeclare namespace Api { export import create = Local; }\nexport = Api;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("function create(id: __thaw_support_"), "{output}");
    assert!(output.contains("export interface Id { value: string; }"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn unwrapped_self_ambient_module_uses_transformed_source_owner() {
    // Unrun ambient-view regression: the disk file stores a string-module
    // wrapper, while the emitted public declaration is top level.
    let dir = temp_registry("owned-self-ambient-view");
    let entry = dir.join("index.d.ts");
    let source = "declare module './index' { interface Hidden { self: string; } export interface Public { value: Hidden; } }\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("export interface Public { value: __thaw_support_"), "{output}");
    assert!(output.contains("export interface Hidden { self: string; }"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn self_targeted_triple_slash_augmentation_keeps_target_scope_private_type() {
    // Unrun exact-projection regression: a top-level Hidden in the target
    // cannot replace the augmentation's same-named Hidden.
    let dir = temp_registry("owned-augmentation-projected-view");
    let entry = dir.join("index.d.ts");
    fs::write(&entry,
        "/// <reference path=\"./leaf.d.ts\" />\nexport as namespace Api;\n",
    ).unwrap();
    fs::write(dir.join("leaf.d.ts"),
        "interface Hidden { wrong: number; }\ndeclare module './index' { interface Hidden { right: string; } export interface Public { value: Hidden; } }\n",
    ).unwrap();
    let output = dts_source_with_reexported_functions(
        &entry, &fs::read_to_string(&entry).unwrap(),
    ).unwrap();
    assert!(output.contains("export interface Public { value: __thaw_support_"), "{output}");
    assert!(output.contains("right: string"), "{output}");
    assert!(!output.contains("export interface Hidden { wrong: number; }"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn unwrapped_global_namespace_retains_private_lexical_member() {
    // Unrun ambient-global regression: the source view includes the
    // unwrapped namespace before its members are hoisted for export =.
    let dir = temp_registry("owned-global-ambient-view");
    let entry = dir.join("index.d.ts");
    let source = "declare global { namespace Api { interface Hidden { fromGlobal: string; } export interface Public { value: Hidden; } } }\nexport = Api;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("export interface Public { value: __thaw_support_"), "{output}");
    assert!(output.contains("export namespace Api {"), "{output}");
    assert!(output.contains("fromGlobal: string"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn owned_export_followers_prefer_explicit_and_reject_ambiguous_stars() {
    // Unrun export-order regression: an explicit binding wins even if its
    // star appears first; distinct star owners conflict, a diamond does not.
    let dir = temp_registry("owned-export-star-precedence");
    let left = dir.join("left.d.ts");
    let right = dir.join("right.d.ts");
    let chosen = dir.join("chosen.d.ts");
    let ambiguous = dir.join("ambiguous.d.ts");
    let selected_over_malformed = dir.join("selected-over-malformed.d.ts");
    let unresolved_explicit = dir.join("unresolved-explicit.d.ts");
    let a = dir.join("a.d.ts");
    let b = dir.join("b.d.ts");
    let diamond = dir.join("diamond.d.ts");
    fs::write(&left, "export interface Id { left: string; }\nexport declare function make(): string;\n").unwrap();
    fs::write(&right, "export interface Id { right: number; }\nexport declare function make(): number;\n").unwrap();
    fs::write(&chosen, "export * from './left';\nexport { Id, make } from './right';\n").unwrap();
    fs::write(&ambiguous, "export * from './left';\nexport * from './right';\n").unwrap();
    fs::write(dir.join("bad.d.ts"), "export interface Incomplete {\n").unwrap();
    fs::write(&selected_over_malformed,
        "export * from './bad';\nexport { Id, make } from './right';\n",
    ).unwrap();
    fs::write(&unresolved_explicit,
        "export * from './left';\nexport { Id, make } from 'external-missing-package';\n",
    ).unwrap();
    fs::write(&a, "export * from './left';\n").unwrap();
    fs::write(&b, "export * from './left';\n").unwrap();
    fs::write(&diamond, "export * from './a';\nexport * from './b';\n").unwrap();
    let views = OwnedSourceViews::default();
    assert_eq!(exported_owned_type_declarations(&chosen, "Id", &views).unwrap()[0].origin,
        right.canonicalize().unwrap());
    assert_eq!(exported_owned_value_declarations(&chosen, "make",
        &mut std::collections::BTreeSet::new()).unwrap()[0].origin,
        right.canonicalize().unwrap());
    assert_eq!(exported_owned_type_declarations(&selected_over_malformed, "Id", &views)
        .unwrap()[0].origin, right.canonicalize().unwrap());
    assert_eq!(exported_owned_value_declarations(&selected_over_malformed, "make",
        &mut std::collections::BTreeSet::new()).unwrap()[0].origin,
        right.canonicalize().unwrap());
    assert!(exported_owned_type_declarations(&ambiguous, "Id", &views).unwrap().is_empty());
    assert!(exported_owned_value_declarations(&ambiguous, "make",
        &mut std::collections::BTreeSet::new()).unwrap().is_empty());
    assert!(exported_owned_type_declarations(&unresolved_explicit, "Id", &views)
        .unwrap().is_empty());
    assert!(exported_owned_value_declarations(&unresolved_explicit, "make",
        &mut std::collections::BTreeSet::new()).unwrap().is_empty());
    assert_eq!(exported_owned_type_declarations(&diamond, "Id", &views).unwrap().len(), 1);
    assert_eq!(exported_owned_value_declarations(&diamond, "make",
        &mut std::collections::BTreeSet::new()).unwrap().len(), 1);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn owned_imports_do_not_revive_uncached_ambiguous_or_unresolved_barrel_exports() {
    // Unrun: a fresh source view must preserve the selected follower's empty
    // result. The old class follower returned the first star instead.
    let dir = temp_registry("owned-uncached-barrel-selection");
    fs::write(dir.join("left.d.ts"),
        "export interface Id { left: string; }\nexport declare function make(): string;\n",
    ).unwrap();
    fs::write(dir.join("right.d.ts"),
        "export interface Id { right: number; }\nexport declare function make(): number;\n",
    ).unwrap();
    fs::write(dir.join("ambiguous.d.ts"),
        "export * from './left';\nexport * from './right';\n",
    ).unwrap();
    fs::write(dir.join("unresolved.d.ts"),
        "export * from './left';\nexport { Id, make } from 'missing-external';\n",
    ).unwrap();
    for barrel in ["ambiguous", "unresolved"] {
        let entry = dir.join(format!("{barrel}-entry.d.ts"));
        fs::write(&entry, format!(
            "import {{ Id, make }} from './{barrel}';\nexport interface Public {{ id: Id; create: typeof make; }}\n",
        )).unwrap();
        let views = OwnedSourceViews::default();
        assert!(resolve_owned_source_type_reference_with_views(&entry, &[], "Id", &views)
            .unwrap().is_empty());
        assert!(resolve_owned_source_value_reference_with_views(&entry, &[], "make", &views)
            .unwrap().is_empty());
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn owned_import_equals_follows_export_assignment_terminal_in_type_and_value_roles() {
    // Unrun: Alias is only the import spelling. The owner is Actual, and its
    // private Hidden field must remain reachable from type and typeof sites.
    let dir = temp_registry("owned-import-equals-terminal");
    let target = dir.join("target.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&target,
        "interface Hidden { secret: string; }\ndeclare class Actual { value: Hidden; static ready(): boolean; }\nexport = Actual;\n",
    ).unwrap();
    let source = "import Alias = require('./target');\nexport interface Model { instance: Alias; factory: typeof Alias; }\n";
    fs::write(&entry, source).unwrap();
    let views = OwnedSourceViews::default();
    for resolved in [
        resolve_owned_source_type_reference_with_views(&entry, &[], "Alias", &views).unwrap(),
        resolve_owned_source_value_reference_with_views(&entry, &[], "Alias", &views).unwrap(),
    ] {
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].origin, target.canonicalize().unwrap());
        assert_eq!(resolved[0].local_name.as_deref(), Some("Actual"));
    }
    let flattened = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(flattened.contains("instance: __thaw_support_"), "{flattened}");
    assert!(flattened.contains("factory: typeof __thaw_support_"), "{flattened}");
    assert!(flattened.contains("secret: string"), "{flattened}");
    assert!(thaw_parser::parse_declarations(&flattened).is_ok(), "{flattened}");
    let barrel = dir.join("barrel.d.ts");
    fs::write(&barrel,
        "import Alias = require('./target');\nexport { Alias as Public };\n",
    ).unwrap();
    let selected_type = exported_owned_type_declarations(&barrel, "Public", &views).unwrap();
    let selected_value = exported_owned_value_declarations(&barrel, "Public",
        &mut std::collections::BTreeSet::new()).unwrap();
    assert_eq!(selected_type[0].local_name.as_deref(), Some("Actual"));
    assert_eq!(selected_value[0].local_name.as_deref(), Some("Actual"));
    fs::write(dir.join("function.d.ts"),
        "declare function ActualFunction(): string;\nexport = ActualFunction;\n",
    ).unwrap();
    let function_entry = dir.join("function-entry.d.ts");
    fs::write(&function_entry, "import Alias = require('./function');\nexport interface Holder { fn: typeof Alias; }\n").unwrap();
    assert!(resolve_owned_source_type_reference_with_views(
        &function_entry, &[], "Alias", &views,
    ).unwrap().is_empty());
    let function_value = resolve_owned_source_value_reference_with_views(
        &function_entry, &[], "Alias", &views,
    ).unwrap();
    assert_eq!(function_value[0].local_name.as_deref(), Some("ActualFunction"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn owned_export_assignment_barrel_cycles_stop_in_type_and_value_roles() {
    // Unrun: crossing export = must retain the active named-export visit.
    // Otherwise A.Public -> B export = Actual -> A.Public recurses forever.
    let dir = temp_registry("owned-export-assignment-cycle");
    let a = dir.join("a.d.ts");
    let b = dir.join("b.d.ts");
    fs::write(&a,
        "import Alias = require('./b');\nexport { Alias as Public };\n",
    ).unwrap();
    fs::write(&b,
        "import { Public as Actual } from './a';\nexport = Actual;\n",
    ).unwrap();
    let views = OwnedSourceViews::default();
    assert!(exported_owned_type_declarations(&a, "Public", &views)
        .unwrap().is_empty());
    assert!(exported_owned_value_declarations(&a, "Public",
        &mut std::collections::BTreeSet::new()).unwrap().is_empty());

    // A noncyclic named import through the same export-assignment boundary
    // still reaches the original type/value owner.
    let c = dir.join("c.d.ts");
    fs::write(&c,
        "export declare class Actual { id: number; }\n",
    ).unwrap();
    fs::write(&b,
        "import { Actual as Chosen } from './c';\nexport = Chosen;\n",
    ).unwrap();
    for selected in [
        exported_owned_type_declarations(&a, "Public", &views).unwrap(),
        exported_owned_value_declarations(&a, "Public",
            &mut std::collections::BTreeSet::new()).unwrap(),
    ] {
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].origin, c.canonicalize().unwrap());
        assert_eq!(selected[0].local_name.as_deref(), Some("Actual"));
    }
    // Sibling edges in one export walk may select the same export-assignment
    // origin. Its active marker must be cleared after each completed edge.
    for kind in [TypeReferenceKind::Type, TypeReferenceKind::ValueQuery] {
        let mut visited = std::collections::BTreeSet::new();
        for _ in 0..2 {
            let selected = selected_export_assignment_declarations_with_visited(
                &b, kind, &views, &mut visited,
            ).unwrap();
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].origin, c.canonicalize().unwrap());
            assert!(visited.is_empty(), "{visited:?}");
        }
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn owned_import_equals_qualified_members_use_the_export_assignment_namespace() {
    // Unrun: Alias.Id and typeof Alias.config refer to Actual's children,
    // not direct exports named Id/config from the target module.
    let dir = temp_registry("owned-import-equals-qualified-terminal");
    let target = dir.join("target.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&target,
        "declare namespace Actual { export interface Id { value: string; } export const config: Id; }\nexport = Actual;\n",
    ).unwrap();
    fs::write(&entry,
        "import Alias = require('./target');\nexport interface Model { id: Alias.Id; setting: typeof Alias.config; }\n",
    ).unwrap();
    let views = OwnedSourceViews::default();
    let id = resolve_owned_qualified_source_type_reference_with_views(
        &entry, &[], &["Alias".into(), "Id".into()], &views,
    ).unwrap();
    let config = resolve_owned_qualified_source_value_reference_with_views(
        &entry, &[], &["Alias".into(), "config".into()], &views,
    ).unwrap();
    assert_eq!(id[0].origin, target.canonicalize().unwrap());
    assert_eq!(id[0].source_scope, vec!["Actual"]);
    assert_eq!(id[0].local_name.as_deref(), Some("Id"));
    assert_eq!(config[0].source_scope, vec!["Actual"]);
    assert_eq!(config[0].local_name.as_deref(), Some("config"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn owned_import_equals_object_property_query_keeps_its_original_type_owner() {
    // Unrun: `export = Actual` can be an object-valued const instead of a
    // namespace. Its property is selected from the source annotation.
    let dir = temp_registry("owned-import-equals-object-property");
    let target = dir.join("target.d.ts");
    let entry = dir.join("entry.d.ts");
    fs::write(&target,
        "interface Hidden { secret: string; }\ninterface Members { config: Hidden; }\ndeclare const Actual: Members;\nexport = Actual;\n",
    ).unwrap();
    fs::write(&entry,
        "import Alias = require('./target');\nexport interface Model { setting: typeof Alias.config; }\n",
    ).unwrap();
    let selected = resolve_owned_qualified_source_value_reference_with_views(
        &entry, &[], &["Alias".into(), "config".into()], &OwnedSourceViews::default(),
    ).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].origin, target.canonicalize().unwrap());
    assert_eq!(selected[0].local_name.as_deref(), Some("config"));
    assert!(selected[0].snippet.contains("config: Hidden"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn wildcard_export_does_not_forward_default_type_or_value() {
    // Unrun negative rule: export * excludes default; a named explicit
    // default alias reaches the same source class in both namespaces.
    let dir = temp_registry("owned-default-wildcard-exclusion");
    let leaf = dir.join("leaf.d.ts");
    let star = dir.join("star.d.ts");
    let named = dir.join("named.d.ts");
    fs::write(&leaf, "export default class Hidden { value: string; }\n").unwrap();
    fs::write(&star, "export * from './leaf';\n").unwrap();
    fs::write(&named, "export { default as Public } from './leaf';\n").unwrap();
    let views = OwnedSourceViews::default();
    assert!(exported_owned_type_declarations(&star, "default", &views).unwrap().is_empty());
    assert!(exported_owned_value_declarations(&star, "default",
        &mut std::collections::BTreeSet::new()).unwrap().is_empty());
    assert_eq!(exported_owned_type_declarations(&named, "Public", &views).unwrap().len(), 1);
    assert_eq!(exported_owned_value_declarations(&named, "Public",
        &mut std::collections::BTreeSet::new()).unwrap().len(), 1);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn default_identifier_expression_preserves_class_type_owner() {
    // Unrun selected-default regression: the default expression names a
    // local class rather than an inline ExportDefaultDecl.
    let dir = temp_registry("owned-default-identifier-type");
    let leaf = dir.join("leaf.d.ts");
    fs::write(&leaf, "declare class C { value: string; }\nexport default C;\n").unwrap();
    let views = OwnedSourceViews::default();
    let selected = exported_owned_type_declarations(&leaf, "default", &views).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].local_name.as_deref(), Some("C"));
    assert_eq!(selected[0].origin, leaf.canonicalize().unwrap());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn composite_export_import_preserves_terminal_owner_and_public_alias() {
    // Unrun: `types.Console` is generated from a target export-assignment
    // property. Its declaration and private dependency still belong to the
    // target, even when the entry has a same-named local declaration.
    let dir = temp_registry("owned-composite-export-import");
    let entry = dir.join("index.d.ts");
    let target = dir.join("transports.d.ts");
    fs::write(&target,
        "declare namespace TransportNs {\n  interface Secret { token: string; }\n  interface ConsoleTransportInstance { secret: Secret; }\n  interface ConsoleTransportInstance { extra: string; }\n  interface Transports { Console: ConsoleTransportInstance; }\n}\ndeclare const TransportNs: TransportNs.Transports;\nexport = TransportNs;\n",
    ).unwrap();
    let source = "import * as Types from './transports';\ndeclare namespace Kit {\n  export import types = Types;\n  export interface Public { selected: types.Console; }\n}\ninterface ConsoleTransportInstance { wrong: number; }\nexport = Kit;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("export const Console: __thaw_support_"), "{output}");
    assert!(output.contains("selected: types.Console"), "{output}");
    assert!(!output.contains("export const Console: ConsoleTransportInstance"), "{output}");
    assert_eq!(output.matches("type __thaw_private_support_marker__ = never").count(), 1, "{output}");
    assert!(output.contains("secret: __thaw_support_"), "{output}");
    assert!(output.contains("extra: string"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn composite_export_assignment_preserves_generic_property_instantiation() {
    // Unrun: a merged generic interface member must project through the
    // instantiated object type. Copying its bare `T` would be invalid.
    let dir = temp_registry("owned-composite-generic-property");
    let entry = dir.join("index.d.ts");
    let target = dir.join("members.d.ts");
    fs::write(&target,
        "declare namespace Source {\n  export class C { constructor(name: string); }\n  export interface Constructor<U> { new (value: U): C; }\n  export interface Members<T> { item: T; instance: C; }\n  export interface Members<T> { ctor: typeof C; factory: Constructor<T>; }\n}\ndeclare const X: Source.Members<string>;\nexport = X;\n",
    ).unwrap();
    let source = "import * as Parts from './members';\ndeclare namespace Api { export import parts = Parts; }\nexport = Api;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("Members<string>[\"item\"]"), "{output}");
    assert!(output.contains("Members<string>[\"instance\"]"), "{output}");
    assert!(output.contains("Members<string>[\"ctor\"]"), "{output}");
    assert!(output.contains("Members<string>[\"factory\"]"), "{output}");
    assert!(!output.contains("const item: T"), "{output}");
    assert_eq!(output.matches("export const ctor:").count(), 1, "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn composite_export_assignment_follows_alias_chain_and_intersection() {
    // Unrun: property names come from the referenced alias graph, while
    // indexed access on the instantiated root preserves each generic type.
    let dir = temp_registry("owned-composite-alias-intersection");
    let entry = dir.join("index.d.ts");
    fs::write(dir.join("members.d.ts"),
        "interface Base<T> { item: T; }\n\
         type Combined<U> = Base<U> & { extra: U; };\n\
         type Outer<V> = Combined<V>;\n\
         declare const X: Outer<string>;\n\
         export = X;\n",
    ).unwrap();
    let source = "import * as Parts from './members';\ndeclare namespace Api { export import parts = Parts; }\nexport = Api;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("Outer<string>[\"item\"]"), "{output}");
    assert!(output.contains("Outer<string>[\"extra\"]"), "{output}");
    assert!(!output.contains("const item: T"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn composite_export_assignment_follows_inherited_and_quoted_object_properties() {
    // Unrun: inherited generic properties must be projected through the
    // instantiated root. A quoted static identifier remains an export;
    // a non-identifier key cannot be spliced into `export const NAME`.
    let dir = temp_registry("owned-composite-inherited-quoted");
    let entry = dir.join("index.d.ts");
    fs::write(dir.join("members.d.ts"),
        "interface Base<T> { \"inherited\": T; \"not-valid-name\": T; }\n\
         interface Members<U> extends Base<U> { own: U; }\n\
         declare const X: Members<string>;\n\
         export = X;\n",
    ).unwrap();
    let source = "import * as Parts from './members';\ndeclare namespace Api { export import parts = Parts; }\nexport = Api;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("Members<string>[\"inherited\"]"), "{output}");
    assert!(output.contains("Members<string>[\"own\"]"), "{output}");
    assert!(output.contains("export const inherited:"), "{output}");
    assert!(!output.contains("export const not-valid-name"), "{output}");
    assert!(!output.contains("const inherited: T"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn composite_export_assignment_inherited_same_name_uses_one_root_projection() {
    // Unrun: a direct property and its inherited declaration can have the
    // same type, or the direct property can narrow the inherited type.
    // Neither should conflict with the root indexed-access projection.
    let dir = temp_registry("owned-composite-inherited-override");
    let entry = dir.join("index.d.ts");
    let source = "import * as Parts from './members';\ndeclare namespace Api { export import parts = Parts; }\nexport = Api;\n";
    fs::write(&entry, source).unwrap();
    for inherited in ["number", "number | string"] {
        fs::write(dir.join("members.d.ts"), format!(
            "interface Base {{ id: {inherited}; }}\ninterface Members extends Base {{ id: number; own: string; }}\ndeclare const X: Members;\nexport = X;\n",
        )).unwrap();
        let output = dts_source_with_reexported_functions(&entry, source).unwrap();
        assert!(output.contains("Members[\"id\"]"), "{output}");
        assert!(output.contains("Members[\"own\"]"), "{output}");
        assert_eq!(output.matches("export const id:").count(), 1, "{output}");
        assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn bare_namespace_alias_keeps_named_export_owners_and_type_roles() {
    // Unrun HOLD regression for the remaining namespace-shaped target:
    // two different source files use the same structural class spelling.
    // The public paths must retain distinct instance shapes, while an
    // interface exported with `export type` must not gain a runtime value.
    let dir = temp_registry("owned-bare-namespace-selection");
    let entry = dir.join("index.d.ts");
    fs::write(dir.join("first.d.ts"),
        "interface Hidden { first: string; }\nexport declare class Client { value: Hidden; next(): Client; }\n",
    ).unwrap();
    fs::write(dir.join("second.d.ts"),
        "interface Hidden { second: number; }\nexport declare class Client { value: Hidden; }\nexport interface Model<T extends Hidden = Hidden> { id: number; data: T; }\n",
    ).unwrap();
    fs::write(dir.join("parts.d.ts"),
        "export { Client as First } from './first';\nexport { Client as Second } from './second';\nexport type { Model as Shape } from './second';\n",
    ).unwrap();
    let source = "import * as Parts from './parts';\ndeclare namespace Api { export import parts = Parts; export interface Uses { first: parts.First; second: parts.Second; shape: parts.Shape; } }\nexport = Api;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("parts.First"), "{output}");
    assert!(output.contains("parts.Second"), "{output}");
    assert!(output.contains("parts.Shape"), "{output}");
    assert!(output.contains("first: string"), "{output}");
    assert!(output.contains("second: number"), "{output}");
    assert_eq!(output.matches("class Client").count(), 2, "{output}");
    assert!(output.contains("export type Shape<T extends __thaw_support_"), "{output}");
    assert!(output.contains("= __thaw_owned_"), "{output}");
    assert!(!output.contains("export const Shape:"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn bare_namespace_alias_respects_explicit_over_wildcard_selection() {
    // Unrun: flattened star records are candidates, not authoritative
    // exports. A later explicit Client shadows the earlier star Client.
    let dir = temp_registry("owned-bare-namespace-export-order");
    let entry = dir.join("index.d.ts");
    fs::write(dir.join("star.d.ts"),
        "export declare class Client { wrong: number; }\n").unwrap();
    fs::write(dir.join("selected.d.ts"),
        "export declare class Client { selected: string; }\n").unwrap();
    fs::write(dir.join("parts.d.ts"),
        "export * from './star';\nexport { Client } from './selected';\n").unwrap();
    let source = "import * as Parts from './parts';\ndeclare namespace Api { export import parts = Parts; }\nexport = Api;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("selected: string"), "{output}");
    assert!(!output.contains("wrong: number"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    fs::write(dir.join("parts.d.ts"),
        "export * from './star';\nexport { Client } from './missing';\n").unwrap();
    let unresolved = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(!unresolved.contains("wrong: number"), "{unresolved}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn bare_namespace_alias_preserves_type_only_namespace_children() {
    // Unrun HOLD control: a type-only namespace export is a container
    // of declarations, not a type alias to a namespace value.
    let dir = temp_registry("owned-bare-type-namespace");
    let entry = dir.join("index.d.ts");
    fs::write(dir.join("models.d.ts"),
        "interface Hidden { token: string; }\nexport interface Model { hidden: Hidden; }\nexport declare class ShapeClass { hidden: Hidden; }\n",
    ).unwrap();
    fs::write(dir.join("parts.d.ts"),
        "export type * as Shapes from './models';\n",
    ).unwrap();
    let source = "import * as Parts from './parts';\ndeclare namespace Api { export import parts = Parts; export interface Use { model: parts.Shapes.Model; } }\nexport = Api;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("namespace Shapes"), "{output}");
    assert!(output.contains("Model"), "{output}");
    assert!(output.contains("ShapeClass"), "{output}");
    assert!(!output.contains("export type Shapes ="), "{output}");
    assert!(output.contains("export namespace Shapes")
        && output.contains("type __thaw_type_only_namespace_marker_"), "{output}");
    assert!(!output.contains("export type { Shapes };"), "{output}");
    assert!(output.contains("__thaw_support_"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn bare_namespace_alias_preserves_nested_value_namespace_reexport() {
    // Unrun: `export * as tools` is represented by scoped child records in
    // the target flattening, without a top-level tools record.
    let dir = temp_registry("owned-bare-nested-namespace");
    let entry = dir.join("index.d.ts");
    fs::write(dir.join("tools.d.ts"),
        "export declare function make(value: string): string;\nexport declare class Shape { id: string; }\n",
    ).unwrap();
    fs::write(dir.join("parts.d.ts"),
        "export * as tools from './tools';\n").unwrap();
    let source = "import * as Parts from './parts';\ndeclare namespace Api { export import parts = Parts; }\nexport = Api;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("namespace tools")
        && output.contains("make(value: string): string")
        && output.contains("class Shape"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    fs::write(dir.join("left.d.ts"), "export * as tools from './tools';\n").unwrap();
    fs::write(dir.join("right.d.ts"), "export * as tools from './tools';\n").unwrap();
    fs::write(dir.join("parts.d.ts"),
        "export * from './left';\nexport * from './right';\n").unwrap();
    let diamond = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert_eq!(diamond.matches("export import tools =").count(), 1, "{diamond}");
    assert!(thaw_parser::parse_declarations(&diamond).is_ok(), "{diamond}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn qualified_hoisted_export_import_uses_ambient_source_view() {
    // Unrun: the selected function lives in a self ambient wrapper. The
    // qualified hoist must resolve the same transformed view as closure.
    let dir = temp_registry("owned-qualified-hoist-ambient");
    let entry = dir.join("index.d.ts");
    let target = dir.join("target.d.ts");
    fs::write(&target,
        "declare module './target' { export function create(): string; }\n",
    ).unwrap();
    let source = "import * as Imported from './target';\ndeclare namespace Kit { export import make = Imported.create; }\nexport = Kit;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("function make(): string"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recursive_composite_export_import_keeps_leaf_owner_and_nested_path() {
    // Unrun: each emitted owner remains distinct across two bare namespace
    // alias levels; the public path is an actual nested declaration.
    let dir = temp_registry("owned-recursive-composite");
    let entry = dir.join("index.d.ts");
    fs::write(dir.join("leaf.d.ts"),
        "declare namespace Leaf { interface Secret { code: string; } interface Item { hidden: Secret; } interface Members { Item: Item; } }\ndeclare const Leaf: Leaf.Members;\nexport = Leaf;\n",
    ).unwrap();
    fs::write(dir.join("mid.d.ts"),
        "import * as LeafModule from './leaf';\ndeclare namespace Mid { export import items = LeafModule; }\nexport = Mid;\n",
    ).unwrap();
    let source = "import * as MidModule from './mid';\ndeclare namespace Top { export import bundle = MidModule; export interface Public { item: bundle.items.Item; } }\nexport = Top;\n";
    fs::write(&entry, source).unwrap();
    let output = dts_source_with_reexported_functions(&entry, source).unwrap();
    assert!(output.contains("declare namespace bundle {"), "{output}");
    assert!(output.contains("export namespace items {"), "{output}");
    assert!(output.contains("export const Item: __thaw_support_"), "{output}");
    assert_eq!(output.matches("type __thaw_private_support_marker__ = never").count(), 1, "{output}");
    assert!(output.contains("hidden: __thaw_support_"), "{output}");
    assert!(thaw_parser::parse_declarations(&output).is_ok(), "{output}");
    let _ = fs::remove_dir_all(dir);
}
