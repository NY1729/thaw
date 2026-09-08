/// Bundles `root_package`'s own CommonJS module graph -- starting from
/// `main_relative` (its `main` field, or a default) -- into a single
/// self-contained JS string with a small embedded module-system
/// emulation, so `thaw-bridge`'s `wrap_as_commonjs_module` (which only
/// ever sees one JS string per package) can still run it correctly.
/// Recurses across *and within* package boundaries: both same-package
/// relative `require`s and `require`s of another real npm package
/// (found under `node_modules_dir`, e.g. a `dependencies` entry) get
/// bundled in, transitively -- `npm install` already fetched the whole
/// dependency tree into a flat `node_modules_dir` before this runs
/// (confirmed by inspecting a real install: `qs`'s own dependency,
/// `side-channel`, and *its* transitive dependencies all landed at the
/// top level), so no second network round-trip is needed here, only
/// local filesystem lookups.
///
/// Found necessary by running two real npm packages through `add`: `qs`
/// (whose `main`, `lib/index.js`, does `require('./stringify')` etc --
/// same-package relative requires, nothing to do with an external
/// dependency) and then, once that worked, `qs`'s own real dependency on
/// `side-channel` (a genuine external package) which the previous
/// implementation left for `wrap_as_commonjs_module`'s global `require`
/// stub to reject outright, exactly like a truly unresolvable dependency
/// would. Before either fix, *any* package split across more than one
/// file -- or depending on another package at all -- failed at load
/// time, which covers most real npm packages beyond a trivial
/// single-file utility.
///
/// A bare specifier's package (or, for a "deep import" spec like
/// `es-errors/type`, the package the subpath is resolved against --
/// `resolve_bare_require`/`split_bare_spec`) that isn't found under
/// `node_modules_dir` at all -- a Node core builtin like `fs`, or a
/// dependency that genuinely wasn't installed -- is left alone, same
/// as an unresolvable relative require -- it falls through to whatever
/// `require` is in scope at runtime, i.e. thaw-bridge's global stub that
/// throws a clear "not supported" error, rather than aborting the whole
/// bundle.
///
/// Returns the bundle text, the resolved key of `main_relative` (the
/// bundle's entry point, for `AddedPackage` reporting), the total number
/// of files folded in (across every package the bundle reaches), and
/// every real npm package the walk touched (`root_package` plus every
/// bare-specifier dependency actually resolved under `node_modules_dir`,
/// Node builtin polyfills excluded) mapped to its own resolved version --
/// see `AddedPackage::dependency_versions`.
fn add_builtin_module(
    name: &str,
    key: String,
    visited: &mut Vec<String>,
    modules: &mut Vec<BundledModule>,
) {
    if visited.contains(&key) {
        return;
    }
    let Some(source) = builtin_module_source(name) else {
        return;
    };
    visited.push(key.clone());
    let mut requires = Vec::new();
    for specifier in analyze_module(source).specs {
        let (resolution, suffix) = split_module_suffix(&specifier);
        let dependency_name = resolution
            .strip_prefix("node:")
            .unwrap_or(resolution)
            .to_string();
        if builtin_module_source(&dependency_name).is_none() {
            continue;
        }
        let dependency_key = format!("node:{dependency_name}{suffix}");
        requires.push((specifier, dependency_key.clone()));
        add_builtin_module(&dependency_name, dependency_key, visited, modules);
    }
    modules.push(BundledModule {
        key,
        source: source.to_string(),
        requires,
        static_esm_specs: Vec::new(),
        has_esm: false,
        has_top_level_await: false,
        async_module: false,
    });
}

fn bundle_builtin_module(name: &str) -> Result<String, String> {
    let main_key = format!("node:{name}");
    let mut visited = Vec::new();
    let mut modules = Vec::new();
    add_builtin_module(name, main_key.clone(), &mut visited, &mut modules);
    prepare_async_modules(&mut modules)?;
    Ok(render_bundle(&main_key, &modules))
}

#[cfg(test)]
fn bundle_commonjs_package(
    node_modules_dir: &Path,
    root_package: &str,
    root_package_dir: &Path,
    main_relative: &str,
) -> Result<(String, String, usize, BTreeMap<String, String>), String> {
    bundle_commonjs_package_cached(
        node_modules_dir,
        root_package,
        root_package_dir,
        main_relative,
        &mut HashMap::new(),
    )
}

fn bundle_commonjs_package_cached(
    node_modules_dir: &Path,
    root_package: &str,
    root_package_dir: &Path,
    main_relative: &str,
    source_cache: &mut HashMap<PathBuf, (String, ModuleAnalysis)>,
) -> Result<(String, String, usize, BTreeMap<String, String>), String> {
    let (main_relative_key, main_abs) = resolve_module_path(root_package_dir, main_relative)?;
    let main_key = format!("{root_package}/{main_relative_key}");

    let mut modules: Vec<BundledModule> = Vec::new();
    let mut visited: Vec<String> = vec![main_key.clone()];
    let mut worklist: Vec<(String, PathBuf, String, PathBuf)> = vec![(
        main_key.clone(),
        main_abs,
        root_package.to_string(),
        root_package_dir.to_path_buf(),
    )];
    let mut dependency_versions: BTreeMap<String, String> = BTreeMap::new();
    let mut uses_global_fetch = false;
    record_package_version(&mut dependency_versions, root_package, root_package_dir);

    while let Some((key, abs_path, pkg_name, pkg_dir)) = worklist.pop() {
        if abs_path
            .extension()
            .is_some_and(|extension| extension == "node" || extension == "wasm")
        {
            continue;
        }
        let (source, analysis) = if let Some(cached) = source_cache.get(&abs_path) {
            cached.clone()
        } else {
            let source = fs::read_to_string(&abs_path)
                .map_err(|e| format!("failed to read `{key}` while bundling: {e}"))?;
            let source = source
                .strip_prefix("#!")
                .and_then(|source| source.split_once('\n').map(|(_, rest)| rest.to_string()))
                .unwrap_or(source);
            let source = if abs_path.extension().is_some_and(|ext| ext == "json") {
                let value: serde_json::Value = serde_json::from_str(&source)
                    .map_err(|error| format!("invalid JSON module `{key}`: {error}"))?;
                format!("module.exports = {};", value)
            } else {
                source
            };
            let source = rewrite_static_worker_urls(&source, &abs_path, &pkg_name, &pkg_dir)?;
            let analysis = analyze_module(&source);
            source_cache.insert(abs_path.clone(), (source.clone(), analysis.clone()));
            (source, analysis)
        };
        uses_global_fetch |= analysis.uses_global_fetch;
        if let Some(error) = &analysis.attribute_error {
            return Err(format!("invalid import attributes in `{key}`: {error}"));
        }
        let module_specs = analysis.specs;

        let relative_in_pkg = key
            .strip_prefix(&format!("{pkg_name}/"))
            .unwrap_or(key.as_str());
        let requiring_dir = Path::new(relative_in_pkg).parent().unwrap_or(Path::new(""));

        let mut requires = Vec::new();

        if analysis.has_nonliteral_module_load {
            let mut candidates = Vec::new();
            collect_relative_files(&pkg_dir, &pkg_dir, &mut candidates)?;
            for relative in candidates.into_iter().filter(|path| {
                !path.split('/').any(|component| component == "node_modules")
                    && matches!(
                        Path::new(path)
                            .extension()
                            .and_then(|extension| extension.to_str()),
                        Some("js" | "cjs" | "mjs" | "json")
                    )
            }) {
                let target_key = format!("{pkg_name}/{relative}");
                if target_key == key {
                    continue;
                }
                let specifier = relative_module_specifier(requiring_dir, Path::new(&relative));
                if !requires.iter().any(|(source, _)| source == &specifier) {
                    requires.push((specifier.clone(), target_key.clone()));
                }
                if let Some(extensionless) = specifier
                    .strip_suffix(".js")
                    .or_else(|| specifier.strip_suffix(".mjs"))
                    .or_else(|| specifier.strip_suffix(".cjs"))
                {
                    if !requires.iter().any(|(source, _)| source == extensionless) {
                        requires.push((extensionless.to_string(), target_key.clone()));
                    }
                }
                if !visited.contains(&target_key) {
                    visited.push(target_key.clone());
                    worklist.push((
                        target_key,
                        pkg_dir.join(&relative),
                        pkg_name.clone(),
                        pkg_dir.clone(),
                    ));
                }
            }
            for specifier in declared_runtime_dependencies(&pkg_dir) {
                if requires.iter().any(|(source, _)| source == &specifier) {
                    continue;
                }
                if let Some((dep_name, dep_relative, dep_abs, dep_dir)) =
                    resolve_bare_require(node_modules_dir, &specifier)
                {
                    let dep_key = format!("{dep_name}/{dep_relative}");
                    requires.push((specifier.clone(), dep_key.clone()));
                    if !visited.contains(&dep_key) {
                        visited.push(dep_key.clone());
                        record_package_version(&mut dependency_versions, &dep_name, &dep_dir);
                        worklist.push((dep_key, dep_abs, dep_name.clone(), dep_dir.clone()));
                    }
                    for subpath in runtime_export_specifiers(&specifier, &dep_dir)? {
                        if requires.iter().any(|(source, _)| source == &subpath) {
                            continue;
                        }
                        if let Some((sub_name, relative, absolute, directory)) =
                            resolve_bare_require(node_modules_dir, &subpath)
                        {
                            let target = format!("{sub_name}/{relative}");
                            requires.push((subpath, target.clone()));
                            if !visited.contains(&target) {
                                visited.push(target.clone());
                                record_package_version(
                                    &mut dependency_versions,
                                    &sub_name,
                                    &directory,
                                );
                                worklist.push((target, absolute, sub_name, directory));
                            }
                        }
                    }
                }
            }
        }

        for spec in module_specs
            .iter()
            .filter(|spec| {
                matches!(spec.as_str(), "." | "..")
                    || spec.starts_with("./")
                    || spec.starts_with("../")
            })
            .cloned()
        {
            let (resolution_spec, suffix) = split_module_suffix(&spec);
            let combined = if requiring_dir.as_os_str().is_empty() {
                resolution_spec.to_string()
            } else {
                format!("{}/{resolution_spec}", requiring_dir.display())
            };
            let normalized = normalize_path_string(&combined);
            // An unresolvable relative require (e.g. it targets a
            // `.json` file, which `resolve_module_path`'s candidates
            // don't cover) is left out of the map on purpose -- that one
            // call falls through to the external-require stub at
            // runtime instead of aborting the whole bundle.
            if let Ok((resolved_relative, resolved_abs)) =
                resolve_module_path(&pkg_dir, &normalized)
            {
                let resolved_key = format!("{pkg_name}/{resolved_relative}{suffix}");
                requires.push((spec, resolved_key.clone()));
                if !visited.contains(&resolved_key) {
                    visited.push(resolved_key.clone());
                    worklist.push((
                        resolved_key,
                        resolved_abs,
                        pkg_name.clone(),
                        pkg_dir.clone(),
                    ));
                }
            }
        }

        for spec in module_specs
            .iter()
            .filter(|spec| {
                !matches!(spec.as_str(), "." | "..")
                    && !spec.starts_with("./")
                    && !spec.starts_with("../")
            })
            .cloned()
        {
            let (resolution_spec, suffix) = split_module_suffix(&spec);
            if resolution_spec.starts_with('#') {
                if let Some((_, resolved_abs)) =
                    resolve_package_import(&abs_path, resolution_spec)
                {
                    let resolved_relative = resolved_abs
                        .strip_prefix(&pkg_dir)
                        .ok()
                        .and_then(|path| path.to_str())
                        .map(normalize_path_string)
                        .unwrap_or_default();
                    if resolved_relative.is_empty() {
                        continue;
                    }
                    let resolved_key = format!("{pkg_name}/{resolved_relative}{suffix}");
                    requires.push((spec, resolved_key.clone()));
                    if !visited.contains(&resolved_key) {
                        visited.push(resolved_key.clone());
                        worklist.push((
                            resolved_key,
                            resolved_abs,
                            pkg_name.clone(),
                            pkg_dir.clone(),
                        ));
                    }
                }
                continue;
            }
            if let Some((dep_name, dep_relative, dep_abs, dep_dir)) =
                resolve_bare_require(node_modules_dir, resolution_spec)
            {
                let dep_key = format!("{dep_name}/{dep_relative}{suffix}");
                requires.push((spec, dep_key.clone()));
                if !visited.contains(&dep_key) {
                    visited.push(dep_key.clone());
                    record_package_version(&mut dependency_versions, &dep_name, &dep_dir);
                    worklist.push((dep_key, dep_abs, dep_name, dep_dir));
                }
                continue;
            }
            // Not a real npm package under `node_modules_dir` -- maybe a
            // Node core builtin Thaw has a polyfill for.
            let builtin_name = resolution_spec
                .strip_prefix("node:")
                .unwrap_or(resolution_spec);
            if builtin_module_source(builtin_name).is_some() {
                let builtin_key = format!("node:{builtin_name}{suffix}");
                requires.push((spec.clone(), builtin_key.clone()));
                if !visited.contains(&builtin_key) {
                    add_builtin_module(builtin_name, builtin_key, &mut visited, &mut modules);
                }
            }
            // Otherwise left unresolved -- falls through to the runtime
            // external-require stub, same as always.
        }

        modules.push(BundledModule {
            key,
            source,
            requires,
            static_esm_specs: analysis.static_esm_specs,
            has_esm: analysis.has_esm,
            has_top_level_await: analysis.has_top_level_await,
            async_module: false,
        });
    }

    if uses_global_fetch {
        add_builtin_module(
            "https",
            "node:https".to_string(),
            &mut visited,
            &mut modules,
        );
    }

    prepare_async_modules(&mut modules)?;

    let file_count = modules.len();
    Ok((
        render_bundle(&main_key, &modules),
        main_key,
        file_count,
        dependency_versions,
    ))
}

/// Records `name`'s resolved version (from its own real `package.json`)
/// into `versions`, if it has one -- best-effort: a package that somehow
/// lacks a readable `package.json`/`version` field (shouldn't happen for
/// anything `npm install` actually fetched, but this is metadata, not a
/// correctness dependency) is just silently left out rather than failing
/// the whole bundle over it.
fn record_package_version(versions: &mut BTreeMap<String, String>, name: &str, dir: &Path) {
    if let Ok(manifest) = read_manifest(dir) {
        if let Some(v) = manifest.get("version").and_then(|v| v.as_str()) {
            versions.insert(name.to_string(), v.to_string());
        }
    }
}
