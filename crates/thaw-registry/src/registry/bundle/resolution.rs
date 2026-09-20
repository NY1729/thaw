/// Splits a bare require spec into its package name and, if present, a
/// "deep import" subpath: `"lodash"` -> `("lodash", None)`, `"lodash/fp"`
/// -> `("lodash", Some("fp"))`, `"@babel/core"` -> `("@babel/core", None)`,
/// `"@babel/core/lib/x"` -> `("@babel/core", Some("lib/x"))`. A scoped
/// name needs two `/`-segments (`@scope/name`) before any subpath starts.
fn split_bare_spec(spec: &str) -> (&str, Option<&str>) {
    if spec.starts_with('@') {
        match spec.match_indices('/').nth(1) {
            Some((idx, _)) => (&spec[..idx], Some(&spec[idx + 1..])),
            None => (spec, None),
        }
    } else {
        match spec.find('/') {
            Some(idx) => (&spec[..idx], Some(&spec[idx + 1..])),
            None => (spec, None),
        }
    }
}
/// Finds the real directory a bare dependency name resolves to, given
/// the directory of the package doing the requiring -- Node's actual
/// `node_modules` resolution walks up from the requiring module's own
/// directory, checking `<ancestor>/node_modules/<dep_name>` at every
/// level, not just the flat top-level `node_modules_dir`. npm only
/// hoists a dependency to that top level when every package needing it
/// agrees on a compatible version; a real version conflict (found via
/// `cheerio` -> `htmlparser2` -> `entities`: `htmlparser2` needs
/// `entities@^7`, but something else in the tree pins `entities@4`,
/// which wins the top-level slot) leaves the conflicting version nested
/// under the dependent's own `node_modules` instead, exactly like a real
/// `npm install` would. Checking `requiring_pkg_dir`'s own ancestors
/// first (before falling back to `node_modules_dir`) reproduces that
/// real algorithm; `node_modules_dir` itself is `requiring_pkg_dir`'s
/// own eventual ancestor when nothing more specific shadows it (a plain
/// top-level dependency, the overwhelmingly common case), so this is a
/// pure generalization, not a behavior change for anything that already
/// worked.
fn resolve_dependency_dir(
    node_modules_dir: &Path,
    requiring_pkg_dir: &Path,
    dep_name: &str,
) -> PathBuf {
    let project_root = node_modules_dir.parent().unwrap_or(node_modules_dir);
    for ancestor in requiring_pkg_dir.ancestors() {
        let candidate = ancestor.join("node_modules").join(dep_name);
        if read_manifest(&candidate).is_ok() {
            return candidate;
        }
        if ancestor == project_root {
            break;
        }
    }
    node_modules_dir.join(dep_name)
}

/// Resolves a bare require spec to the file it actually points to,
/// searching `requiring_pkg_dir`'s own nested `node_modules` before
/// falling back to the flat `node_modules_dir` (see
/// `resolve_dependency_dir`). With no subpath, that's the target
/// package's own `main` field (or the `index.js` default); with a "deep
/// import" subpath (e.g. `require('es-errors/type')`, found necessary by
/// `qs`'s own transitive dependency chain), Node resolves the subpath
/// directly against the package root -- the target package's `main`
/// field is irrelevant in that case. Returns `(package name, path
/// resolved relative to the package root, its absolute path, the
/// package's own directory)`, or `None` if it can't be resolved (not
/// installed anywhere in the tree, or -- rare -- the subpath itself
/// doesn't exist) -- left for the runtime external-require stub to
/// report, same as any other unresolvable require.
fn resolve_bare_require(
    node_modules_dir: &Path,
    requiring_pkg_dir: &Path,
    spec: &str,
) -> Option<(String, String, PathBuf, PathBuf)> {
    let (dep_name, subpath) = split_bare_spec(spec);
    let dep_dir = resolve_dependency_dir(node_modules_dir, requiring_pkg_dir, dep_name);
    let target = match subpath {
        Some(sub) => read_manifest(&dep_dir)
            .ok()
            .and_then(|manifest| package_subpath_runtime_target(&manifest, sub))
            .unwrap_or_else(|| sub.to_string()),
        None => {
            let manifest = read_manifest(&dep_dir).ok()?;
            package_export_target(&manifest, None, &["require", "node", "default", "import"])
                .or_else(|| manifest.get("main").and_then(|v| v.as_str()))
                .unwrap_or("index.js")
                .to_string()
        }
    };
    let (dep_relative, dep_abs) = resolve_module_path(&dep_dir, &target).ok()?;
    Some((dep_name.to_string(), dep_relative, dep_abs, dep_dir))
}

fn package_subpath_runtime_target(manifest: &serde_json::Value, subpath: &str) -> Option<String> {
    if let Some(target) = package_export_target(
        manifest,
        Some(subpath),
        &["require", "node", "default", "import"],
    ) {
        return Some(target.to_string());
    }
    let exports = manifest.get("exports")?.as_object()?;
    for (key, value) in exports {
        let Some(pattern) = key.strip_prefix("./") else {
            continue;
        };
        let Some(capture) = wildcard_capture(pattern, subpath) else {
            continue;
        };
        if let Some(target) =
            select_export_condition(value, &["require", "node", "default", "import"])
        {
            return Some(target.replace('*', capture));
        }
    }
    None
}

fn resolve_package_import(path: &Path, spec: &str) -> Option<(String, PathBuf)> {
    let package_dir = path
        .ancestors()
        .find(|directory| read_manifest(directory).is_ok())?;
    let manifest = read_manifest(package_dir).ok()?;
    let imports = manifest.get("imports")?.as_object()?;
    if let Some(value) = imports.get(spec) {
        let target = select_export_condition(value, &["require", "node", "default", "import"])?;
        return resolve_module_path(package_dir, target).ok();
    }
    for (pattern, value) in imports {
        let Some(capture) = wildcard_capture(pattern, spec) else {
            continue;
        };
        if let Some(target) =
            select_export_condition(value, &["require", "node", "default", "import"])
        {
            return resolve_module_path(package_dir, &target.replace('*', capture)).ok();
        }
    }
    None
}

/// Collapses `.`/`..` segments in a `/`-separated path string (npm
/// `require` specs always use `/`, regardless of host OS). No crate
/// dependency needed for this -- e.g. `"lib/./../lib/parse"` ->
/// `"lib/parse"`.
fn normalize_path_string(path: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    stack.join("/")
}

fn relative_module_specifier(from_dir: &Path, target: &Path) -> String {
    let from: Vec<_> = from_dir
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let to: Vec<_> = target
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = vec!["..".to_string(); from.len() - common];
    parts.extend(to[common..].iter().cloned());
    let path = parts.join("/");
    if path.starts_with("../") {
        path
    } else {
        format!("./{path}")
    }
}
