fn npm_install(scratch: &Path, package: &str) -> Result<(), String> {
    let output = Command::new("npm")
        .arg("install")
        .arg("--prefix")
        .arg(scratch)
        .args(["--no-audit", "--no-fund", "--ignore-scripts", package])
        .output()
        .map_err(|e| format!("failed to invoke `npm` (is Node.js/npm installed?): {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`npm install {package}` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn read_manifest(package_dir: &Path) -> Result<serde_json::Value, String> {
    let manifest_path = package_dir.join("package.json");
    let manifest_source = fs::read_to_string(&manifest_path).map_err(|e| {
        format!(
            "failed to read `{}` after `npm install`: {e}",
            manifest_path.display()
        )
    })?;
    serde_json::from_str(&manifest_source)
        .map_err(|e| format!("`{}` is not valid JSON: {e}", manifest_path.display()))
}

fn package_export_target<'a>(
    manifest: &'a serde_json::Value,
    subpath: Option<&str>,
    conditions: &[&str],
) -> Option<&'a str> {
    let exports = manifest.get("exports")?;
    let target = match subpath {
        None => {
            if exports.is_string() {
                exports
            } else {
                exports
                    .as_object()
                    .and_then(|object| object.get("."))
                    .unwrap_or(exports)
            }
        }
        Some(subpath) => exports.as_object()?.get(&format!("./{subpath}"))?,
    };
    select_export_condition(target, conditions)
}

enum ExportCondition<'a> {
    Target(&'a str),
    Blocked,
    Unmatched,
}

fn select_export_condition<'a>(
    value: &'a serde_json::Value,
    conditions: &[&str],
) -> Option<&'a str> {
    match select_export_condition_inner(value, conditions) {
        ExportCondition::Target(path) => Some(path),
        ExportCondition::Blocked | ExportCondition::Unmatched => None,
    }
}

fn select_export_condition_inner<'a>(
    value: &'a serde_json::Value,
    conditions: &[&str],
) -> ExportCondition<'a> {
    if value.is_null() {
        return ExportCondition::Blocked;
    }
    if let Some(path) = value.as_str() {
        if conditions == ["types"]
            && ![".d.ts", ".d.cts", ".d.mts"]
                .iter()
                .any(|extension| path.ends_with(extension))
        {
            return ExportCondition::Unmatched;
        }
        return ExportCondition::Target(path);
    }
    if let Some(candidates) = value.as_array() {
        // Array entries may fall through, but a terminal null (or an empty
        // array) still blocks later conditional keys.
        let mut blocked = candidates.is_empty();
        for candidate in candidates {
            match select_export_condition_inner(candidate, conditions) {
                ExportCondition::Target(path) => return ExportCondition::Target(path),
                ExportCondition::Blocked => blocked = true,
                ExportCondition::Unmatched => {}
            }
        }
        return if blocked { ExportCondition::Blocked } else { ExportCondition::Unmatched };
    }
    let Some(object) = value.as_object() else {
        return ExportCondition::Unmatched;
    };
    for (condition, value) in object {
        if conditions.contains(&condition.as_str()) {
            match select_export_condition_inner(value, conditions) {
                ExportCondition::Target(path) => return ExportCondition::Target(path),
                ExportCondition::Blocked => return ExportCondition::Blocked,
                ExportCondition::Unmatched => {}
            }
        }
    }
    if conditions == ["types"] {
        for condition in ["require", "node", "default", "import"] {
            if let Some(value) = object.get(condition) {
                match select_export_condition_inner(value, conditions) {
                    ExportCondition::Target(path) => return ExportCondition::Target(path),
                    ExportCondition::Blocked => return ExportCondition::Blocked,
                    ExportCondition::Unmatched => {}
                }
            }
        }
        for child in object.values() {
            match select_export_condition_inner(child, conditions) {
                ExportCondition::Target(path) => return ExportCondition::Target(path),
                ExportCondition::Blocked => return ExportCondition::Blocked,
                ExportCondition::Unmatched => {}
            }
        }
    }
    ExportCondition::Unmatched
}

fn validate_export_subpath(subpath: &str) -> Result<(), String> {
    if subpath.is_empty()
        || Path::new(subpath).is_absolute()
        || subpath
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(format!("invalid package export subpath `{subpath}`"));
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
struct PackageSubpathExport {
    subpath: String,
    runtime_entry: String,
    types_entry: String,
}

fn collect_relative_files(root: &Path, dir: &Path, output: &mut Vec<String>) -> Result<(), String> {
    for entry in
        fs::read_dir(dir).map_err(|error| format!("failed to read `{}`: {error}", dir.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_relative_files(root, &path, output)?;
        } else if path.is_file() {
            output.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

fn wildcard_capture<'a>(pattern: &str, path: &'a str) -> Option<&'a str> {
    let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
    let (prefix, suffix) = pattern.split_once('*')?;
    if suffix.contains('*')
        || path.len() < prefix.len() + suffix.len()
        || !path.starts_with(prefix)
        || !path.ends_with(suffix)
    {
        return None;
    }
    Some(&path[prefix.len()..path.len() - suffix.len()])
}

fn matched_package_subpath_export<'a, 'b>(
    exports: &'a serde_json::Map<String, serde_json::Value>,
    subpath: &'b str,
) -> Option<(&'a serde_json::Value, Option<&'b str>)> {
    if let Some(exact) = exports.get(&format!("./{subpath}")) {
        return Some((exact, None));
    }
    let mut matches = exports.iter().filter_map(|(key, value)| {
        let pattern = key.strip_prefix("./")?;
        let capture = wildcard_capture(pattern, subpath)?;
        let (prefix, suffix) = pattern.split_once('*')?;
        Some((prefix.len(), suffix.len(), value, capture))
    }).collect::<Vec<_>>();
    matches.sort_by_key(|(prefix, suffix, _, _)| {
        (std::cmp::Reverse(*prefix), std::cmp::Reverse(*suffix))
    });
    matches.first().map(|(_, _, value, capture)| (*value, Some(*capture)))
}

fn package_subpath_exports(
    manifest: &serde_json::Value,
    package_dir: &Path,
) -> Result<Vec<PackageSubpathExport>, String> {
    let Some(exports) = manifest.get("exports").and_then(|value| value.as_object()) else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    collect_relative_files(package_dir, package_dir, &mut files)?;
    let mut subpaths = Vec::new();
    for (key, target) in exports {
        let Some(subpath) = key.strip_prefix("./") else {
            continue;
        };
        let Some(runtime) = select_export_condition(target, &["require", "node", "default"])
        else {
            continue;
        };
        let Some(types) = select_export_condition(target, &["types"]) else {
            continue;
        };
        if subpath.contains('*') {
            if subpath.matches('*').count() != 1
                || runtime.matches('*').count() < 1
                || types.matches('*').count() < 1
            {
                return Err(format!(
                    "package export pattern `{key}` must contain exactly one `*` in its key and at least one in its runtime and types targets"
                ));
            }
            for file in &files {
                let Some(capture) = wildcard_capture(types, file) else {
                    continue;
                };
                let expanded = subpath.replacen('*', capture, 1);
                validate_export_subpath(&expanded)?;
                subpaths.push(expanded);
            }
            continue;
        }
        validate_export_subpath(subpath)?;
        subpaths.push(subpath.to_string());
    }
    subpaths.sort();
    subpaths.dedup();
    Ok(subpaths.into_iter().filter_map(|subpath| {
        // The most specific key wins, including an exact or pattern `null`
        // that excludes a file enumerated by a broader wildcard.
        let (target, capture) = matched_package_subpath_export(exports, &subpath)?;
        let runtime = select_export_condition(target, &["require", "node", "default"])?;
        let types = select_export_condition(target, &["types"])?;
        let runtime_entry = capture.map_or_else(|| runtime.to_string(), |value| runtime.replace('*', value));
        let types_entry = capture.map_or_else(|| types.to_string(), |value| types.replace('*', value));
        Some(PackageSubpathExport { subpath, runtime_entry, types_entry })
    }).collect())
}

/// `package` has no bundled type declarations of its own -- fetch the
/// corresponding DefinitelyTyped package instead. `@types/*` packages
/// carry no runtime JS of their own (`main` is typically empty), just
/// declarations, so this only ever contributes the `.d.ts`; the actual
/// `bundle.js` still always comes from `package`.
fn fetch_types_package_dts(scratch: &Path, package: &str) -> Result<(String, String), String> {
    let types_package = types_package_name(package);
    npm_install(scratch, &types_package).map_err(|e| {
        format!("`{package}` has no bundled type declarations, and fetching `{types_package}` also failed: {e}")
    })?;

    let types_dir = scratch.join("node_modules").join(&types_package);
    let manifest = read_manifest(&types_dir).map_err(|e| {
        format!("`{package}` has no bundled type declarations, and reading `{types_package}`'s manifest failed: {e}")
    })?;
    let (rel, abs) = find_own_dts(&manifest, &types_dir).ok_or_else(|| {
        format!("`{package}` has no bundled type declarations, and `{types_package}` doesn't provide a usable one either")
    })?;
    let source = fs::read_to_string(&abs)
        .map_err(|e| format!("failed to read `{rel}` from `{types_package}`: {e}"))?;
    // Same flattening a same-package `.d.ts` already gets (barrel
    // re-exports inlined, `/// <reference path="...">` files -- common in
    // an older, multi-file DefinitelyTyped package like `@types/lodash`
    // -- inlined too): this file is about to be the *only* one thaw-registry
    // keeps around for `package`, so anything it reaches now needs
    // following before that scratch checkout disappears.
    let source = dts_source_with_reexported_functions(&abs, &source)?;

    Ok((format!("{types_package}/{rel}"), source))
}

fn installed_types_package_dts(
    node_modules: &Path,
    package: &str,
) -> Result<Option<(String, String)>, String> {
    let types_package = types_package_name(package);
    let types_dir = node_modules.join(&types_package);
    if !types_dir.join("package.json").is_file() {
        return Ok(None);
    }
    let manifest = read_manifest(&types_dir)?;
    let Some((rel, abs)) = find_own_dts(&manifest, &types_dir) else {
        return Err(format!("`{types_package}` doesn't provide a usable type declaration"));
    };
    let source = fs::read_to_string(&abs)
        .map_err(|error| format!("failed to read `{rel}` from `{types_package}`: {error}"))?;
    Ok(Some((
        format!("{types_package}/{rel}"),
        dts_source_with_reexported_functions(&abs, &source)?,
    )))
}

/// The DefinitelyTyped naming convention: `foo` -> `@types/foo`,
/// `@scope/name` -> `@types/scope__name` (the `/` becomes `__`, since a
/// types package itself is unscoped).
fn types_package_name(package: &str) -> String {
    match package
        .strip_prefix('@')
        .and_then(|rest| rest.split_once('/'))
    {
        Some((scope, name)) => format!("@types/{scope}__{name}"),
        None => format!("@types/{package}"),
    }
}

/// `types`/`typings` field first (in that order -- both spellings are
/// common in the wild), then a same-named `index.d.ts` next to `main` as
/// a last resort (common for older packages predating the `types` field
/// convention, e.g. left-pad/slugify). Returns both the path as recorded
/// (relative to `package_dir`) and the resolved absolute path to read.
fn find_own_dts(manifest: &serde_json::Value, package_dir: &Path) -> Option<(String, PathBuf)> {
    if let Some(path) = package_export_target(manifest, None, &["types"]) {
        let absolute = package_dir.join(path);
        if absolute.is_file() {
            return Some((path.to_string(), absolute));
        }
    }
    for field in ["types", "typings"] {
        if let Some(path) = manifest.get(field).and_then(|v| v.as_str()) {
            return Some((path.to_string(), package_dir.join(path)));
        }
    }
    let index = package_dir.join("index.d.ts");
    if index.is_file() {
        return Some(("index.d.ts".to_string(), index));
    }
    None
}

/// Approximates enough of Node's CommonJS resolution algorithm to read a
/// module path relative to `package_dir`: try the path exactly as
/// written, then with a `.js` extension appended, then as a directory
/// containing `index.js`. Real packages commonly write `"main": "./index"`
/// (no extension, resolved by Node at require-time) or `"main": "./lib"`
/// (a directory) -- reading the literal string as a path fails for both.
/// Also used, the same way, to resolve a same-package relative `require`
/// spec against the requiring file's directory (`bundle_commonjs_package`).
/// Doesn't attempt the rest of Node's real algorithm (`package.json`
/// `exports` maps, `.json`/`.node` candidates, etc.) -- just these two
/// common shapes.
fn is_directory_module_request(path: &str) -> bool {
    path.ends_with('/')
        || path == "."
        || path == ".."
        || path.ends_with("/.")
        || path.ends_with("/..")
}

fn resolve_module_path(package_dir: &Path, path: &str) -> Result<(String, PathBuf), String> {
    let mut active_directories = std::collections::HashSet::new();
    let mut cyclic_entry = false;
    let mut identity_error = None;
    resolve_module_path_inner(
        package_dir,
        path,
        &mut active_directories,
        &mut cyclic_entry,
        &mut identity_error,
    )
}

fn resolve_module_path_inner(
    package_dir: &Path,
    path: &str,
    active_directories: &mut std::collections::HashSet<PathBuf>,
    cyclic_entry: &mut bool,
    identity_error: &mut Option<String>,
) -> Result<(String, PathBuf), String> {
    let trimmed = path.trim_end_matches('/');
    let candidates = [
        path.to_string(),
        format!("{trimmed}.js"),
        format!("{trimmed}.cjs"),
        // Real ESM packages commonly use an explicit `.mjs` extension
        // (sometimes alongside a separate `.cjs` build) rather than
        // relying on `package.json`'s `"type": "module"`.
        format!("{trimmed}.mjs"),
        format!("{trimmed}.json"),
        format!("{trimmed}/index.js"),
        format!("{trimmed}/index.cjs"),
        format!("{trimmed}/index.mjs"),
        format!("{trimmed}/index.json"),
    ];
    // Explicit directory requests skip file extension probes (as in Node's
    // CommonJS loader). Otherwise a sibling file beats a directory entry.
    let directory_only = is_directory_module_request(path);
    if !directory_only {
        for candidate in &candidates[..5] {
            let resolved = package_dir.join(candidate);
            if resolved.is_file() {
                return Ok((candidate.clone(), resolved));
            }
        }
    }
    let directory = package_dir.join(trimmed);
    if directory.is_dir() {
        // Canonical identity detects both lexical `.`/`..` cycles and symlink aliases.
        // Keep only the active recursion chain: a candidate may be visited again
        // after another branch has finished without forming a cycle.
        match fs::canonicalize(&directory) {
            Ok(identity) if active_directories.insert(identity.clone()) => {
                let nested = read_manifest(&directory).ok().and_then(|manifest| {
                    let entry =
                        package_export_target(&manifest, None, &["require", "node", "default"])
                            .or_else(|| manifest.get("main").and_then(|value| value.as_str()));
                    entry.and_then(|entry| {
                        resolve_module_path_inner(
                            &directory,
                            entry,
                            active_directories,
                            cyclic_entry,
                            identity_error,
                        )
                        .ok()
                    })
                });
                active_directories.remove(&identity);
                if let Some((relative, absolute)) = nested {
                    return Ok((
                        normalize_path_string(&format!("{trimmed}/{relative}")),
                        absolute,
                    ));
                }
            }
            Ok(_) => *cyclic_entry = true,
            Err(error) => {
                *identity_error = Some(format!(
                    "couldn't identify JS module directory `{}`: {error}",
                    directory.display()
                ));
            },
        }
    }
    for candidate in &candidates[5..] {
        let resolved = package_dir.join(candidate);
        if resolved.is_file() {
            return Ok((candidate.clone(), resolved));
        }
    }
    if let Some(error) = identity_error {
        return Err(error.clone());
    }
    if *cyclic_entry {
        return Err(format!(
            "circular directory module entry for `\"{path}\"` (tried `{}`)",
            candidates.join("`, `")
        ));
    }
    Err(format!(
        "couldn't find a JS module for `\"{path}\"` (tried `{}`)",
        candidates.join("`, `")
    ))
}
