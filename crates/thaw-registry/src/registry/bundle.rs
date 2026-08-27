/// One file pulled into a `bundle_commonjs_package` bundle. `key` is
/// `"<owning package name>/<path resolved by resolve_module_path,
/// relative to that package's own root>"` -- package-qualified so two
/// different packages' identically-named files (`index.js` is extremely
/// common) can't collide in the same bundle. Used both as the emitted
/// module map's key and, by stripping the package-name prefix and
/// resolving against that package's own directory, to read the file.
/// `requires` is every `require` spec this file's source contains that
/// was actually resolved (same-package relative, or a bare specifier
/// resolved to another bundled package), each mapped to its own target
/// `key`.
struct BundledModule {
    key: String,
    source: String,
    requires: Vec<(String, String)>,
    static_esm_specs: Vec<String>,
    has_esm: bool,
    has_top_level_await: bool,
    async_module: bool,
}

fn declared_runtime_dependencies(package_dir: &Path) -> Vec<String> {
    let Ok(source) = fs::read_to_string(package_dir.join("package.json")) else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&source) else {
        return Vec::new();
    };
    let mut dependencies = Vec::new();
    for field in ["dependencies", "optionalDependencies", "peerDependencies"] {
        let Some(entries) = manifest.get(field).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for name in entries.keys() {
            if !dependencies.contains(name) {
                dependencies.push(name.clone());
            }
        }
    }
    dependencies
}

fn split_module_suffix(specifier: &str) -> (&str, &str) {
    specifier
        .char_indices()
        .find_map(|(index, character)| {
            (character == '?' || (character == '#' && index > 0)).then_some(index)
        })
        .map(|index| specifier.split_at(index))
        .unwrap_or((specifier, ""))
}

fn runtime_export_specifiers(
    package_name: &str,
    package_dir: &Path,
) -> Result<Vec<String>, String> {
    let Ok(source) = fs::read_to_string(package_dir.join("package.json")) else {
        return Ok(Vec::new());
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&source) else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    let mut specifiers = Vec::new();
    let Some(exports_value) = manifest.get("exports") else {
        collect_relative_files(package_dir, package_dir, &mut files)?;
        for file in files.iter().filter(|file| {
            !file.split('/').any(|component| component == "node_modules")
                && matches!(
                    Path::new(file)
                        .extension()
                        .and_then(|extension| extension.to_str()),
                    Some("js" | "cjs" | "mjs" | "json")
                )
        }) {
            specifiers.push(format!("{package_name}/{file}"));
            if let Some(extensionless) = file
                .strip_suffix(".js")
                .or_else(|| file.strip_suffix(".cjs"))
                .or_else(|| file.strip_suffix(".mjs"))
            {
                specifiers.push(format!("{package_name}/{extensionless}"));
            }
            if let Some(directory) = file
                .strip_suffix("/index.js")
                .or_else(|| file.strip_suffix("/index.cjs"))
                .or_else(|| file.strip_suffix("/index.mjs"))
                .or_else(|| file.strip_suffix("/index.json"))
            {
                specifiers.push(format!("{package_name}/{directory}"));
            }
        }
        specifiers.sort();
        specifiers.dedup();
        return Ok(specifiers);
    };
    let Some(exports) = exports_value.as_object() else {
        return Ok(Vec::new());
    };
    for (key, target) in exports {
        let Some(subpath) = key.strip_prefix("./") else {
            continue;
        };
        let Some(runtime) = select_export_condition(target, &["require", "import", "default"])
        else {
            continue;
        };
        if subpath.contains('*') {
            if files.is_empty() {
                collect_relative_files(package_dir, package_dir, &mut files)?;
            }
            for file in &files {
                if let Some(capture) = wildcard_capture(runtime, file) {
                    let expanded = subpath.replacen('*', capture, 1);
                    if validate_export_subpath(&expanded).is_ok() {
                        specifiers.push(format!("{package_name}/{expanded}"));
                    }
                }
            }
        } else if validate_export_subpath(subpath).is_ok() {
            specifiers.push(format!("{package_name}/{subpath}"));
        }
    }
    specifiers.sort();
    specifiers.dedup();
    Ok(specifiers)
}

include!("module_transform.rs");
include!("builtins.rs");

include!("bundle/workers.rs");
include!("bundle/collection.rs");
include!("bundle/resolution.rs");
include!("bundle/async_modules.rs");
include!("bundle/render.rs");
