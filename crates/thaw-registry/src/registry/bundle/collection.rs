fn package_instance_prefix(node_modules_dir: &Path, name: &str, directory: &Path) -> String {
    let Ok(relative) = directory.strip_prefix(node_modules_dir) else {
        // The test helper and linked packages may provide an external root.
        // Keep its historical key; claim_module_key below rejects a collision.
        return name.to_string();
    };
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| matches!(component, std::path::Component::ParentDir)) {
        return name.to_string();
    }
    normalize_path_string(&relative.to_string_lossy())
}

fn package_module_key(node_modules_dir: &Path, name: &str, directory: &Path, relative: &str, suffix: &str) -> String {
    format!("{}/{}{}", package_instance_prefix(node_modules_dir, name, directory), relative, suffix)
}

fn claim_module_key(identities: &mut HashMap<String, PathBuf>, key: &str, path: &Path) -> Result<(), String> {
    let identity = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(previous) = identities.get(key) {
        if previous != &identity {
            return Err(format!("bundle module key `{key}` names both `{}` and `{}`", previous.display(), identity.display()));
        }
    } else {
        identities.insert(key.to_string(), identity);
    }
    Ok(())
}

fn commonjs_factory_context(path: &Path, package_dir: &Path, has_esm: bool, uses_import_meta: bool) -> bool {
    if has_esm {
        return false;
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("cjs" | "json") => return true,
        Some("mjs") => return false,
        // Explicit files with no suffix (or an arbitrary suffix) are also
        // loaded as JavaScript by resolve_module_path's first candidate.
        _ => {},
    }
    for directory in path.ancestors().skip(1) {
        if !directory.starts_with(package_dir) {
            break;
        }
        if directory.join("package.json").exists() {
            return match read_manifest(directory) {
                Ok(manifest) => match manifest.get("type").and_then(serde_json::Value::as_str) {
                    Some("module") => false,
                    Some("commonjs") => true,
                    None if manifest.get("type").is_none() => !uses_import_meta,
                    _ => false,
                },
                Err(_) => false,
            };
        }
        if directory == package_dir {
            break;
        }
    }
    !uses_import_meta
}


// Only an explicit `.mjs` extension or the nearest valid `type: module`
// package scope declares an ESM source to the native loader.
fn declared_native_esm_context(path: &Path, package_dir: &Path) -> bool {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("mjs") => return true,
        Some("js") => {},
        _ => return false,
    }
    for directory in path.ancestors().skip(1) {
        if !directory.starts_with(package_dir) { break; }
        if directory.join("package.json").exists() {
            return read_manifest(directory).is_ok_and(|manifest|
                manifest.get("type").and_then(serde_json::Value::as_str) == Some("module"));
        }
        if directory == package_dir { break; }
    }
    false
}

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
    let source_name = thaw_parser::common::FileName::Custom(format!("node:{name} (generated builtin module)"));
    let mut requires = Vec::new();
    for specifier in analyze_module_named(source, &source_name).specs {
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
        source_name,
        export_graph: None,
        origin_parameter: None,
        requires,
        imports: Vec::new(),
        known_packages: Vec::new(),
        static_esm_specs: Vec::new(),
        has_esm: false,
        has_top_level_await: false,
        commonjs_context: true,
        native_esm_context: false,
        uses_legacy_bundle_globals: true,
        has_nonliteral_module_load: false,
        uses_import_meta: false,
        async_module: false,
        source_path: None,
    });
}

fn bundle_builtin_module(name: &str) -> Result<String, String> {
    let main_key = format!("node:{name}");
    let mut visited = Vec::new();
    let mut modules = Vec::new();
    add_builtin_module(name, main_key.clone(), &mut visited, &mut modules);
    prepare_async_modules(&mut modules, &mut SourceCache::default())?;
    Ok(render_bundle(&main_key, &modules))
}

// Staged native/CJS candidate assembly. The production selector below stays
// unchanged until the full imported-binding/facade path has been reviewed.
fn prepare_mixed_native_render(main_key: &str, modules: &[BundledModule]) -> Option<MixedBundleRender> {
    use std::collections::{BTreeSet, HashMap};
    // Native link validation can use a known ESM graph or the established
    // synthetic CommonJS default. Other opaque names depend on running a
    // factory, so keep those graphs on the existing renderer.
    fn statically_checkable(
        key: &str, name: &str, modules: &[BundledModule], native: &BTreeSet<String>,
        seen: &mut BTreeSet<(String, String)>,
    ) -> bool {
        if !native.contains(key) { return name == "default"; }
        if !seen.insert((key.to_owned(), name.to_owned())) { return true; }
        let Some(module) = modules.iter().find(|module| module.key == key) else { return false };
        let Some(graph) = module.export_graph.as_ref() else { return true };
        if graph["local"].get(name).is_some() { return true; }
        if let Some(indirect) = graph["indirect"].get(name) {
            let Some(spec) = indirect[0].as_str() else { return false };
            let Some(target) = module.imports.iter().find(|(request, _)| request == spec).map(|(_, target)| target) else { return false };
            if indirect[1].is_null() { return true; }
            let Some(remote) = indirect[1].as_str() else { return false };
            return statically_checkable(target, remote, modules, native, seen);
        }
        if name == "default" { return true; }
        let Some(stars) = graph["stars"].as_array() else { return false };
        stars.iter().all(|star| star.as_str().and_then(|spec|
            module.imports.iter().find(|(request, _)| request == spec).map(|(_, target)| target))
            .is_some_and(|target| statically_checkable(target, name, modules, native, seen)))
    }
    let native = modules.iter().filter(|module| module.native_esm_context
        && !module.commonjs_context && !module.uses_legacy_bundle_globals
        && !module.has_nonliteral_module_load && !module.uses_import_meta)
        .map(|module| module.key.clone()).collect::<BTreeSet<_>>();
    if native.is_empty() || native.len() == modules.len() { return None; }
    // Nonliteral loads, serialized Worker URLs, and legacy globals still use
    // the established factory renderer. Literal CommonJS require edges can use the private native
    // evaluator after the shared synchronous/async guard in the renderer.
    if modules.iter().any(|module| module.has_nonliteral_module_load
        || module.source.contains("data:text/javascript;thaw-bundle,")
        || module.uses_legacy_bundle_globals && native.contains(&module.key)) {
        return None;
    }
    let module_keys = modules.iter().map(|module| module.key.as_str()).collect::<BTreeSet<_>>();
    let opaque = modules.iter().filter(|module| !native.contains(&module.key))
        .map(|module| module.key.clone()).collect::<BTreeSet<_>>();
    let asynchronous_opaque = modules.iter().filter(|module| opaque.contains(&module.key)
        && module.async_module).map(|module| module.key.as_str()).collect::<BTreeSet<_>>();
    // An async opaque dependency that can import back into any native source
    // may await that source's still-pending evaluation. Keep that graph on
    // the legacy bridge until a cycle-aware async boundary is available.
    let mut reaches_native = native.clone();
    loop {
        let mut changed = false;
        for module in modules {
            if module.imports.iter().any(|(_, target)| reaches_native.contains(target)) {
                changed |= reaches_native.insert(module.key.clone());
            }
        }
        if !changed { break; }
    }
    if asynchronous_opaque.iter().any(|key| reaches_native.contains(*key)) { return None; }
    // Dynamic imports from opaque factories use the registration-bound
    // evaluator; synchronous require-to-native remains gated above.
    let mut mixed = opaque.clone();
    loop {
        let mut changed = false;
        for module in modules.iter().filter(|module| native.contains(&module.key)) {
            if module.imports.iter().any(|(_, target)| mixed.contains(target)) {
                changed |= mixed.insert(module.key.clone());
            }
        }
        if !changed { break; }
    }
    let mut records = Vec::new();
    let mut opaque_edges = Vec::new();
    let mut origins = Vec::new();
    let mut removed_link_checks = Vec::new();
    for module in modules.iter().filter(|module| native.contains(&module.key)) {
        if module.static_esm_specs.iter().any(|spec| !module.imports.iter().any(|(request, target)|
            request == spec && module_keys.contains(target.as_str()))) { return None; }
        let rewrite_specs = module.imports.iter().filter(|(_, target)| mixed.contains(target))
            .map(|(spec, _)| spec.clone()).collect::<BTreeSet<_>>();
        for (spec, name) in native_opaque_link_checks_named(&module.source, &rewrite_specs,
            &module.source_name)? {
            let target = module.imports.iter().find(|(request, _)| request == &spec)?.1.as_str();
            if !statically_checkable(target, &name, modules, &native, &mut BTreeSet::new()) {
                return None;
            }
            removed_link_checks.push((module.key.clone(), spec, name));
        }
        let helper_spec = format!("__thaw_private_origin_spec/{}", module.key);
        let helper_key = format!("__thaw_private_origin_key/{}", module.key);
        if module_keys.contains(helper_key.as_str()) || module.imports.iter().any(|(spec, _)| spec == &helper_spec) {
            return None;
        }
        let source = rewrite_native_opaque_edges_named(&module.source, &rewrite_specs,
            &helper_spec, &module.source_name)?;
        let mut imports = module.imports.iter().cloned().collect::<HashMap<_, _>>();
        for (index, (spec, target)) in module.imports.iter().filter(|(_, target)| opaque.contains(target)).enumerate() {
            let shell = format!("__thaw_private_opaque_edge/{}/{}", module.key, index);
            if module_keys.contains(shell.as_str()) { return None; }
            imports.insert(spec.clone(), shell.clone());
            opaque_edges.push(serde_json::json!({ "key": shell, "importer": module.key,
                "specifier": spec, "target": target, "factory": target,
                "asynchronous": asynchronous_opaque.contains(target.as_str()) }));
        }
        // The source rewrite always introduces this lexical capability. It
        // also serves dynamic import selection when there are no static
        // opaque imports, so every native record must map it.
        imports.insert(helper_spec, helper_key.clone());
        origins.push(serde_json::json!({ "key": helper_key, "owner": module.key }));
        records.push(serde_json::json!({ "key": module.key, "source": source, "imports": imports }));
    }
    // Preserve laziness: every native entry validates only its own static
    // dependency closure immediately before that entry is evaluated.
    fn visit_static_closure(
        key: &str, modules: &[BundledModule], native: &BTreeSet<String>,
        seen: &mut BTreeSet<String>, ordered: &mut Vec<String>,
    ) {
        if !native.contains(key) || !seen.insert(key.to_owned()) { return; }
        if let Some(module) = modules.iter().find(|module| module.key == key) {
            for spec in &module.static_esm_specs {
                if let Some(target) = module.imports.iter().find(|(request, _)| request == spec).map(|(_, target)| target) {
                    visit_static_closure(target, modules, native, seen, ordered);
                }
            }
        }
        ordered.push(key.to_owned());
    }
    let mut static_link_checks = std::collections::BTreeMap::new();
    for entry in modules.iter().filter(|module| native.contains(&module.key)) {
        let mut closure = Vec::new();
        visit_static_closure(&entry.key, modules, &native, &mut BTreeSet::new(), &mut closure);
        let checks = closure.iter().flat_map(|owner| removed_link_checks.iter()
            .filter(move |(source, _, _)| source == owner).cloned()).collect::<Vec<_>>();
        static_link_checks.insert(entry.key.clone(), checks);
    }
    let registration = serde_json::json!({ "main": if native.contains(main_key) { main_key } else {
        native.iter().next().map(String::as_str)?
    }, "modules": records,
        "opaqueKeys": opaque, "opaqueEdges": opaque_edges, "origins": origins });
    Some(MixedBundleRender {
        native_keys: native,
        mixed_keys: mixed,
        static_link_checks,
        registration,
    })
}

/// Shared across every subpath bundle of one `registry add`: parsed module
/// sources (`modules`) and their ESM->CommonJS rewrites (`rewritten`), keyed
/// by file and package instance because Worker bootstrap text embeds the
/// instance locator, so a
/// package with many subpaths reuses the parse of every shared dependency
/// instead of redoing it per subpath. Real trigger: drizzle-orm has dozens
/// of subpaths over the same large `dist/cjs` files; without this,
/// `thaw registry add drizzle-orm` re-parsed the whole graph for each one.
#[derive(Default)]
struct SourceCache {
    modules: HashMap<(PathBuf, String), (String, ModuleAnalysis, thaw_parser::common::FileName)>,
    rewritten: HashMap<(PathBuf, String, bool), Option<String>>,
    package_versions: BTreeMap<PathBuf, (String, String, String)>,
    // Raw identities for the most recently completed bundle. The cumulative
    // map above cannot describe one export after its shared dependencies were
    // already visited by another export.
    last_bundle_versions: BTreeMap<PathBuf, (String, String, String)>,
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
        &mut SourceCache::default(),
    )
}

/// The module text of `bytes`, or `None` when the file is a binary asset
/// (invalid UTF-8, or NUL bytes in its first 8 KiB -- text modules have none).
fn module_source_text(bytes: Vec<u8>) -> Option<String> {
    let head = &bytes[..bytes.len().min(8192)];
    if head.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn bundle_commonjs_package_cached(
    node_modules_dir: &Path,
    root_package: &str,
    root_package_dir: &Path,
    main_relative: &str,
    source_cache: &mut SourceCache,
) -> Result<(String, String, usize, BTreeMap<String, String>), String> {
    let (main_relative_key, main_abs) = resolve_module_path(root_package_dir, main_relative)?;
    let main_key = package_module_key(node_modules_dir, root_package, root_package_dir, &main_relative_key, "");

    let mut modules: Vec<BundledModule> = Vec::new();
    let mut visited: Vec<String> = vec![main_key.clone()];
    let mut identities = HashMap::new();
    claim_module_key(&mut identities, &main_key, &main_abs)?;
    let mut worklist: Vec<(String, PathBuf, String, PathBuf)> = vec![(
        main_key.clone(),
        main_abs,
        root_package.to_string(),
        root_package_dir.to_path_buf(),
    )];
    let mut package_versions: BTreeMap<PathBuf, (String, String, String)> = BTreeMap::new();
    let mut uses_global_fetch = false;
    record_package_version(&mut package_versions, node_modules_dir, root_package, root_package_dir);

    while let Some((key, abs_path, pkg_name, pkg_dir)) = worklist.pop() {
        let pkg_key = package_instance_prefix(node_modules_dir, &pkg_name, &pkg_dir);
        let caller_dir = abs_path.parent().unwrap_or(&pkg_dir);
        if abs_path
            .extension()
            .is_some_and(|extension| extension == "node" || extension == "wasm")
        {
            continue;
        }
        let source_cache_key = (abs_path.clone(), pkg_key.clone());
        let (source, analysis, source_name) = if let Some(cached) = source_cache.modules.get(&source_cache_key) {
            cached.clone()
        } else {
            let bytes = fs::read(&abs_path)
                .map_err(|e| format!("failed to read `{key}` while bundling: {e}"))?;
            // A require/exports target that is not text (a shared library, an
            // image, any binary asset a package ships) is not a module: leave it
            // out of the bundle like `.node`/`.wasm`, so it is reached by path.
            let Some(source) = module_source_text(bytes) else {
                continue;
            };
            let had_shebang = source.starts_with("#!") && source.contains('\n');
            let is_json = abs_path.extension().is_some_and(|ext| ext == "json");
            let source = source
                .strip_prefix("#!")
                .and_then(|source| source.split_once('\n').map(|(_, rest)| rest.to_string()))
                .unwrap_or(source);
            let source = if is_json {
                let value: serde_json::Value = serde_json::from_str(&source)
                    .map_err(|error| format!("invalid JSON module `{key}`: {error}"))?;
                format!("module.exports = JSON.parse({});", js_string_literal(&value.to_string()))
            } else {
                source
            };
            let source_name = if is_json {
                thaw_parser::common::FileName::Custom(format!("{} (generated JSON module)", abs_path.display()))
            } else if had_shebang {
                thaw_parser::common::FileName::Custom(format!("{} (after shebang removal)", abs_path.display()))
            } else {
                thaw_parser::common::FileName::Real(abs_path.clone())
            };
            let rewritten = rewrite_static_worker_urls_named(&source, &abs_path, source_name.clone(), &pkg_key, &pkg_dir)?;
            let source_name = if rewritten != source {
                rewritten_js_source_name(&source_name, "Worker URL rewrite")
            } else {
                source_name
            };
            let analysis = analyze_module_named(&rewritten, &source_name);
            source_cache
                .modules
                .insert(source_cache_key, (rewritten.clone(), analysis.clone(), source_name.clone()));
            (rewritten, analysis, source_name)
        };
        uses_global_fetch |= analysis.uses_global_fetch;
        if let Some(error) = &analysis.attribute_error {
            return Err(format!("invalid import attributes in `{key}`: {error}"));
        }
        let module_specs = analysis.specs;

        let relative_in_pkg = abs_path.strip_prefix(&pkg_dir)
            .map_err(|_| format!("module `{}` is outside package `{}`", abs_path.display(), pkg_dir.display()))?;
        let requiring_dir = relative_in_pkg.parent().unwrap_or(Path::new(""));

        let mut requires = Vec::new();
        let mut imports = Vec::new();
        let mut known_packages = Vec::new();
        for spec in &module_specs {
            let (resolution_spec, _) = split_module_suffix(spec);
            if resolution_spec.starts_with('#') || matches!(resolution_spec, "." | "..")
                || resolution_spec.starts_with("./") || resolution_spec.starts_with("../") {
                continue;
            }
            let (package_name, _) = split_bare_spec(resolution_spec);
            let directory = resolve_dependency_dir(node_modules_dir, caller_dir, package_name);
            if read_manifest(&directory).is_ok() && !known_packages.contains(&package_name.to_string()) {
                known_packages.push(package_name.to_string());
            }
        }

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
                let target_key = package_module_key(node_modules_dir, &pkg_name, &pkg_dir, &relative, "");
                if target_key == key {
                    continue;
                }
                let specifier = relative_module_specifier(requiring_dir, Path::new(&relative));
                if !requires.iter().any(|(source, _)| source == &specifier) {
                    requires.push((specifier.clone(), target_key.clone()));
                }
                if !imports.iter().any(|(source, _)| source == &specifier) {
                    imports.push((specifier.clone(), target_key.clone()));
                }
                if let Some(extensionless) = specifier
                    .strip_suffix(".js")
                    .or_else(|| specifier.strip_suffix(".mjs"))
                    .or_else(|| specifier.strip_suffix(".cjs"))
                {
                    if !requires.iter().any(|(source, _)| source == extensionless) {
                        requires.push((extensionless.to_string(), target_key.clone()));
                    }
                    if !imports.iter().any(|(source, _)| source == extensionless) {
                        imports.push((extensionless.to_string(), target_key.clone()));
                    }
                }
                let target_path = pkg_dir.join(&relative);
                claim_module_key(&mut identities, &target_key, &target_path)?;
                if !visited.contains(&target_key) {
                    visited.push(target_key.clone());
                    worklist.push((
                        target_key,
                        target_path,
                        pkg_name.clone(),
                        pkg_dir.clone(),
                    ));
                }
            }
            for specifier in declared_runtime_dependencies(&pkg_dir) {
                let (dep_name, _) = split_bare_spec(&specifier);
                let dep_dir = resolve_dependency_dir(node_modules_dir, caller_dir, dep_name);
                if read_manifest(&dep_dir).is_ok() && !known_packages.contains(&dep_name.to_string()) {
                    known_packages.push(dep_name.to_string());
                }
                for import_condition in [false, true] {
                    let conditions: &[&str] = if import_condition {
                        &["import", "node", "default"]
                    } else {
                        &["require", "node", "default"]
                    };
                    let subpaths = runtime_export_specifiers_with_conditions(
                        &specifier, &dep_dir, conditions,
                    )?;
                    let targets = if import_condition { &mut imports } else { &mut requires };
                    if !targets.iter().any(|(source, _)| source == &specifier) {
                        let resolved = if import_condition {
                            resolve_bare_import(node_modules_dir, caller_dir, &specifier)
                        } else {
                            resolve_bare_require(node_modules_dir, caller_dir, &specifier)
                        };
                        if let Some((name, relative, absolute, directory)) = resolved {
                            let target = package_module_key(node_modules_dir, &name, &directory, &relative, "");
                            targets.push((specifier.clone(), target.clone()));
                            claim_module_key(&mut identities, &target, &absolute)?;
                            if !visited.contains(&target) {
                                visited.push(target.clone());
                                record_package_version(&mut package_versions, node_modules_dir, &name, &directory);
                                worklist.push((target, absolute, name, directory));
                            }
                        }
                    }
                    for subpath in &subpaths {
                        if targets.iter().any(|(source, _)| source == subpath) {
                            continue;
                        }
                        let resolved = if import_condition {
                            resolve_bare_import(node_modules_dir, caller_dir, subpath)
                        } else {
                            resolve_bare_require(node_modules_dir, caller_dir, subpath)
                        };
                        if let Some((sub_name, relative, absolute, directory)) = resolved {
                            let target = package_module_key(node_modules_dir, &sub_name, &directory, &relative, "");
                            targets.push((subpath.clone(), target.clone()));
                            claim_module_key(&mut identities, &target, &absolute)?;
                            if !visited.contains(&target) {
                                visited.push(target.clone());
                                record_package_version(
                                    &mut package_versions,
                                    node_modules_dir,
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
            // Normalizing `./foo/` or `./foo/.` removes the marker that
            // restricts CommonJS resolution to a directory.
            let resolution_path = if is_directory_module_request(resolution_spec) {
                if normalized.is_empty() { ".".to_string() } else { format!("{normalized}/") }
            } else {
                normalized
            };
            // An unresolvable relative require (e.g. it targets a
            // `.json` file, which `resolve_module_path`'s candidates
            // don't cover) is left out of the map on purpose -- that one
            // call falls through to the external-require stub at
            // runtime instead of aborting the whole bundle.
            if let Ok((resolved_relative, resolved_abs)) =
                resolve_module_path(&pkg_dir, &resolution_path)
            {
                let resolved_key = package_module_key(node_modules_dir, &pkg_name, &pkg_dir, &resolved_relative, suffix);
                if analysis.require_condition_specs.contains(&spec) {
                    requires.push((spec.clone(), resolved_key.clone()));
                }
                if analysis.import_condition_specs.contains(&spec) {
                    imports.push((spec, resolved_key.clone()));
                }
                claim_module_key(&mut identities, &resolved_key, &resolved_abs)?;
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
        {
            let (resolution_spec, suffix) = split_module_suffix(spec);
            for import_condition in [false, true] {
                if !(if import_condition {
                    analysis.import_condition_specs.contains(spec)
                } else {
                    analysis.require_condition_specs.contains(spec)
                }) {
                    continue;
                }
                let targets = if import_condition { &mut imports } else { &mut requires };
            if resolution_spec.starts_with('#') {
                let conditions: &[&str] = if import_condition {
                    &["import", "node", "default"]
                } else {
                    &["require", "node", "default"]
                };
                let resolved = resolve_package_import(
                    node_modules_dir, &abs_path, resolution_spec, conditions,
                );
                let (name, relative, absolute, directory) = match resolved {
                    Some(PackageImportResolution::Local(absolute)) => {
                        let Some(relative) = absolute
                            .strip_prefix(&pkg_dir)
                            .ok()
                            .and_then(|path| path.to_str())
                            .map(normalize_path_string)
                        else {
                            continue;
                        };
                        (pkg_name.clone(), relative, absolute, pkg_dir.clone())
                    }
                    Some(PackageImportResolution::External(name, relative, absolute, directory)) => {
                        (name, relative, absolute, directory)
                    }
                    Some(PackageImportResolution::Builtin(name)) => {
                        let builtin_key = format!("node:{name}{suffix}");
                        targets.push((spec.clone(), builtin_key.clone()));
                        add_builtin_module(&name, builtin_key, &mut visited, &mut modules);
                        continue;
                    }
                    None => continue,
                };
                if relative.is_empty() {
                    continue;
                }
                let resolved_key = package_module_key(node_modules_dir, &name, &directory, &relative, suffix);
                targets.push((spec.clone(), resolved_key.clone()));
                claim_module_key(&mut identities, &resolved_key, &absolute)?;
                if !visited.contains(&resolved_key) {
                    visited.push(resolved_key.clone());
                    record_package_version(&mut package_versions, node_modules_dir, &name, &directory);
                    worklist.push((resolved_key, absolute, name, directory));
                }
                continue;
            }
            let resolved = if import_condition {
                resolve_bare_import(node_modules_dir, caller_dir, resolution_spec)
            } else {
                resolve_bare_require(node_modules_dir, caller_dir, resolution_spec)
            };
            if let Some((dep_name, dep_relative, dep_abs, dep_dir)) = resolved {
                let dep_key = package_module_key(node_modules_dir, &dep_name, &dep_dir, &dep_relative, suffix);
                targets.push((spec.clone(), dep_key.clone()));
                claim_module_key(&mut identities, &dep_key, &dep_abs)?;
                if !visited.contains(&dep_key) {
                    visited.push(dep_key.clone());
                    record_package_version(&mut package_versions, node_modules_dir, &dep_name, &dep_dir);
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
                targets.push((spec.clone(), builtin_key.clone()));
                if !visited.contains(&builtin_key) {
                    add_builtin_module(builtin_name, builtin_key, &mut visited, &mut modules);
                }
            }
            // Otherwise left unresolved -- falls through to the runtime
            // external-require stub, same as always.
            }
        }

        modules.push(BundledModule {
            key,
            export_graph: esm_export_graph_named(&source, &source_name),
            origin_parameter: esm_origin_parameter_named(&source, &source_name),
            source,
            source_name,
            requires,
            imports,
            known_packages,
            static_esm_specs: analysis.static_esm_specs,
            has_esm: analysis.has_esm,
            has_top_level_await: analysis.has_top_level_await,
            commonjs_context: commonjs_factory_context(&abs_path, &pkg_dir, analysis.has_esm, analysis.uses_import_meta),
            native_esm_context: declared_native_esm_context(&abs_path, &pkg_dir),
            uses_legacy_bundle_globals: analysis.uses_legacy_bundle_globals,
            has_nonliteral_module_load: analysis.has_nonliteral_module_load,
            uses_import_meta: analysis.uses_import_meta,
            async_module: false,
            source_path: Some(abs_path),
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

    // Native module linking owns the original source only when the complete
    // collected graph is ESM. Mixed graphs retain the existing CJS bridge.
    let native_esm = modules.iter().any(|module| module.has_esm)
        && modules.iter().all(|module| {
            module.native_esm_context && !module.commonjs_context
                && module.requires.is_empty()
                && !module.uses_legacy_bundle_globals
                && !module.has_nonliteral_module_load
                && !module.uses_import_meta
                && module.static_esm_specs.iter().all(|spec| {
                    module.imports.iter().any(|(request, target)| request == spec
                        && modules.iter().any(|candidate| candidate.key == *target))
                })
        });
    // Classify async dependencies while the original ESM sources are still
    // available. Only opaque factories are rewritten; registered native
    // modules must retain the source that QuickJS links.
    let mixed = if native_esm {
        None
    } else {
        mark_async_modules(&mut modules)?;
        prepare_mixed_native_render(&main_key, &modules)
    };
    if !native_esm {
        let preserve_native = mixed.as_ref()
            .map(|route| route.native_keys.clone())
            .unwrap_or_default();
        rewrite_async_modules(&mut modules, source_cache, &preserve_native);
    }

    let file_count = modules.len();
    let dependency_versions = project_package_versions(package_versions.clone())?;
    source_cache.last_bundle_versions = package_versions.clone();
    source_cache.package_versions.extend(package_versions);
    Ok((
        if native_esm { render_native_esm_bundle(&main_key, &modules) }
        else { render_bundle_mode(&main_key, &modules, mixed.as_ref()) },
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
fn record_package_version(
    versions: &mut BTreeMap<PathBuf, (String, String, String)>,
    node_modules_dir: &Path,
    name: &str,
    dir: &Path,
) {
    if let Ok(manifest) = read_manifest(dir) {
        if let Some(v) = manifest.get("version").and_then(|v| v.as_str()) {
            let identity = dir.to_path_buf();
            versions.insert(identity, (name.to_string(), package_instance_prefix(node_modules_dir, name, dir), v.to_string()));
        }
    }
}

fn project_package_versions(versions: BTreeMap<PathBuf, (String, String, String)>) -> Result<BTreeMap<String, String>, String> {
    let mut counts = HashMap::new();
    for (name, _, _) in versions.values() {
        *counts.entry(name.clone()).or_insert(0usize) += 1;
    }
    let mut projected = BTreeMap::new();
    for (name, locator, version) in versions.into_values() {
        let key = if counts[&name] == 1 { name } else { locator };
        if projected.insert(key.clone(), version).is_some() {
            return Err(format!("package instance version key `{key}` is ambiguous"));
        }
    }
    Ok(projected)
}
