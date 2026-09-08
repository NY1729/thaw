/// Fetches `package` via `npm install` (into a throwaway scratch
/// directory -- `--ignore-scripts`, since this runs an arbitrary
/// third-party package's install unattended and its `postinstall` is not
/// something to execute automatically) and copies its declared type
/// definitions and CommonJS `main` entry point into `registry_dir/
/// <package>/` as `package.d.ts`/`bundle.js` -- the layout `resolve`
/// expects. This is the "automatic" half of the registry story (see
/// docs/design/registry.md): turns a bare package name into a usable
/// `--use <package>` entry, no hand-curation.
///
/// If `package` doesn't bundle its own type declarations (no `types`/
/// `typings` field in `package.json`, no same-named `index.d.ts`), this
/// falls back to fetching the corresponding DefinitelyTyped package
/// (`@types/<package>`, or `@types/<scope>__<name>` for a scoped
/// `@<scope>/<name>` package) and uses *its* declarations -- the JS
/// still always comes from `package` itself, since `@types/*` packages
/// carry no runtime code. Native addons never produce a `native.a`: bundled
/// `.node` files, platform optional dependencies, and GitHub-hosted
/// `prebuild-install` assets are selected independently of the JS fallback.
///
/// `package` may carry a version/tag/range specifier the same way `npm
/// install` accepts one (`left-pad@1.3.0`, `left-pad@^1.2.0`,
/// `left-pad@next`, or a bare `left-pad` for "whatever `npm` calls
/// latest") -- passed through to `npm install` completely unexamined
/// (`split_package_spec` only peels it off to know the bare package name
/// for the registry's own on-disk directory and for the `@types/*`
/// fallback name, which is versioned independently of whatever version
/// of `package` itself was requested). The version `npm` actually
/// resolved the specifier to is read back from the fetched package's own
/// `package.json` and recorded in `version.txt` next to `package.d.ts`/
/// `bundle.js` -- there's still no lockfile or cross-package version
/// *graph* (each `add` call is independent, exactly like one `npm
/// install <spec>` would be), but a specific version can now actually be
/// requested and later confirmed, rather than every `add` silently
/// meaning "whatever's newest today".
pub fn add(registry_dir: &Path, package: &str) -> Result<AddedPackage, String> {
    let scratch = std::env::temp_dir().join(format!(
        "thaw-registry-add-{}-{}",
        package.replace(['/', '@'], "_"),
        std::process::id()
    ));
    fs::create_dir_all(&scratch).map_err(|e| {
        format!(
            "failed to create scratch directory `{}`: {e}",
            scratch.display()
        )
    })?;

    let result = fetch_and_copy(&scratch, registry_dir, package);
    let _ = fs::remove_dir_all(&scratch);
    result
}

/// Splits an `add`/`npm install`-style package specifier into the bare
/// package name and an optional version/tag/range suffix. A scoped
/// package's leading `@scope/` is never mistaken for a version separator
/// -- only an `@` *after* that (or, for an unscoped name, anywhere at
/// all) starts a version: `"left-pad"` -> `("left-pad", None)`,
/// `"left-pad@1.3.0"` -> `("left-pad", Some("1.3.0"))`, `"@hapi/hoek"` ->
/// `("@hapi/hoek", None)`, `"@hapi/hoek@9.0.0"` -> `("@hapi/hoek",
/// Some("9.0.0"))`.
/// The bare-name half of [`split_package_spec`], exposed for callers
/// (thaw-cli's `registry add` reporting) that need to know which
/// registry directory a possibly-versioned `add` argument actually
/// landed in without duplicating the parsing themselves.
pub fn package_name(spec: &str) -> &str {
    split_package_spec(spec).0
}

fn split_package_spec(spec: &str) -> (&str, Option<&str>) {
    let search_from = if spec.starts_with('@') {
        spec.find('/').map(|i| i + 1).unwrap_or(spec.len())
    } else {
        0
    };
    match spec[search_from..].find('@') {
        Some(rel_i) => {
            let at = search_from + rel_i;
            (&spec[..at], Some(&spec[at + 1..]))
        }
        None => (spec, None),
    }
}

fn fetch_and_copy(
    scratch: &Path,
    registry_dir: &Path,
    package: &str,
) -> Result<AddedPackage, String> {
    let (name, _version_spec) = split_package_spec(package);

    npm_install(scratch, package)?;
    let node_modules_dir = scratch.join("node_modules");
    let package_dir = node_modules_dir.join(name);
    let manifest = read_manifest(&package_dir)?;
    if select_prebuilt_addon(&package_dir)?.is_none()
        && select_optional_dependency_addon(&node_modules_dir, &manifest)?.is_none()
    {
        download_prebuild_install_addon(&package_dir, &manifest)?;
    }
    let fallback_dts = if find_own_dts(&manifest, &package_dir).is_none() {
        Some(fetch_types_package_dts(scratch, name)?)
    } else {
        None
    };
    add_installed_inner(registry_dir, &node_modules_dir, name, fallback_dts, true)
}

/// Registers a package that already exists under `node_modules_dir`.
/// This is the filesystem half of [`add`], exposed for offline/vendor
/// workflows that have already performed dependency installation.
pub fn add_installed(
    registry_dir: &Path,
    node_modules_dir: &Path,
    name: &str,
) -> Result<AddedPackage, String> {
    let fallback_dts = installed_types_package_dts(node_modules_dir, name)?;
    add_installed_inner(registry_dir, node_modules_dir, name, fallback_dts, true)
}

/// Registers only the package root. Builds that use a project's existing
/// `node_modules` can then materialize imported subpaths individually with
/// [`add_installed_subpath`] instead of eagerly bundling every export.
pub fn add_installed_root(
    registry_dir: &Path,
    node_modules_dir: &Path,
    name: &str,
) -> Result<AddedPackage, String> {
    let fallback_dts = installed_types_package_dts(node_modules_dir, name)?;
    add_installed_inner(registry_dir, node_modules_dir, name, fallback_dts, false)
}

fn add_installed_inner(
    registry_dir: &Path,
    node_modules_dir: &Path,
    name: &str,
    fallback_dts: Option<(String, String)>,
    eager_subpaths: bool,
) -> Result<AddedPackage, String> {
    let package_dir = node_modules_dir.join(name);
    let manifest = read_manifest(&package_dir)?;

    let resolved_version = manifest
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();

    let main_field = package_export_target(&manifest, None, &["require", "import", "default"])
        .or_else(|| manifest.get("main").and_then(|v| v.as_str()))
        .unwrap_or("index.js");
    let mut bundle_source_cache = HashMap::new();
    let (js_source, js_relative_path, bundled_file_count, mut dependency_versions) =
        bundle_commonjs_package_cached(
            node_modules_dir,
            name,
            &package_dir,
            main_field,
            &mut bundle_source_cache,
        )?;

    let (dts_relative_path, dts_source) = match find_own_dts(&manifest, &package_dir) {
        Some((rel, abs)) => {
            let source =
                fs::read_to_string(&abs).map_err(|e| format!("failed to read `{rel}`: {e}"))?;
            (rel, dts_source_with_reexported_functions(&abs, &source)?)
        }
        None => fallback_dts.ok_or_else(|| {
            format!(
                "`{name}` has no bundled type declarations; install its `@types` package or use `registry add`"
            )
        })?,
    };

    let dest_dir = registry_dir.join(name);
    fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("failed to create `{}`: {e}", dest_dir.display()))?;

    // Re-adding a package must never leave a stale binary selected for a
    // previous version/target.
    for stale in [
        dest_dir.join("native.node"),
        dest_dir.join("native-addon.json"),
    ] {
        if stale.is_file() {
            fs::remove_file(&stale).map_err(|error| {
                format!("failed to remove stale `{}`: {error}", stale.display())
            })?;
        }
    }
    let selected_addon = select_prebuilt_addon(&package_dir).and_then(|selected| match selected {
        Some(selected) => Ok(Some(selected)),
        None => select_optional_dependency_addon(node_modules_dir, &manifest)
            .and_then(|selected| match selected {
                Some(selected) => Ok(Some(selected)),
                None => select_generated_addon(node_modules_dir, &js_source),
            }),
    });
    let (native_addon, native_diagnostic) = match selected_addon {
        Ok(Some(selected)) => {
            let bytes = fs::read(&selected.path).map_err(|error| {
                format!(
                    "failed to read native addon `{}`: {error}",
                    selected.path.display()
                )
            })?;
            fs::write(dest_dir.join("native.node"), &bytes).map_err(|error| {
                format!(
                    "failed to write `{}`: {error}",
                    dest_dir.join("native.node").display()
                )
            })?;
            let metadata = NativeAddonMetadata {
                source: selected.source,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                platform: selected.platform,
                arch: selected.arch,
                libc: selected.libc,
            };
            let metadata_json = serde_json::to_string_pretty(&metadata)
                .map_err(|error| format!("failed to serialize native addon metadata: {error}"))?;
            fs::write(dest_dir.join("native-addon.json"), metadata_json).map_err(|error| {
                format!(
                    "failed to write `{}`: {error}",
                    dest_dir.join("native-addon.json").display()
                )
            })?;
            (Some(metadata), None)
        }
        Ok(None) => (None, None),
        Err(diagnostic) => (None, Some(diagnostic)),
    };
    let dependencies = dest_dir.join("native-dependencies");
    if dependencies.is_dir() {
        fs::remove_dir_all(&dependencies).map_err(|error| error.to_string())?;
    }
    let shared_libraries = platform_shared_libraries(node_modules_dir, &manifest)?;
    if !shared_libraries.is_empty() {
        fs::create_dir_all(&dependencies).map_err(|error| error.to_string())?;
        for (index, source) in shared_libraries.iter().enumerate() {
            let name = source.file_name().unwrap_or_default().to_string_lossy();
            fs::copy(source, dependencies.join(format!("{index}-{name}")))
                .map_err(|error| format!("failed to copy `{}`: {error}", source.display()))?;
        }
    }
    fs::write(dest_dir.join("package.d.ts"), dts_source).map_err(|e| {
        format!(
            "failed to write `{}`: {e}",
            dest_dir.join("package.d.ts").display()
        )
    })?;
    write_bundle(&dest_dir, &js_source)?;
    let subpaths_dir = dest_dir.join("subpaths");
    if subpaths_dir.is_dir() {
        fs::remove_dir_all(&subpaths_dir).map_err(|error| {
            format!(
                "failed to remove stale `{}`: {error}",
                subpaths_dir.display()
            )
        })?;
    }
    if eager_subpaths {
        for export in package_subpath_exports(&manifest, &package_dir)? {
            dependency_versions.extend(write_installed_subpath(
                registry_dir,
                node_modules_dir,
                name,
                &package_dir,
                &export,
                &mut bundle_source_cache,
            )?);
        }
    }
    fs::write(dest_dir.join("version.txt"), &resolved_version).map_err(|e| {
        format!(
            "failed to write `{}`: {e}",
            dest_dir.join("version.txt").display()
        )
    })?;
    // Only worth a `lock.json` at all once there's something beyond
    // `package`'s own entry (which `version.txt` already covers) --
    // a single-file package with no dependencies would otherwise get an
    // uninformative one-entry file next to `version.txt` saying the same
    // thing twice.
    if dependency_versions.len() > 1 {
        let lock_json = serde_json::to_string_pretty(&dependency_versions)
            .map_err(|e| format!("failed to serialize `lock.json` for `{name}`: {e}"))?;
        fs::write(dest_dir.join("lock.json"), lock_json).map_err(|e| {
            format!(
                "failed to write `{}`: {e}",
                dest_dir.join("lock.json").display()
            )
        })?;
    }

    Ok(AddedPackage {
        dts_relative_path,
        js_relative_path,
        bundled_file_count,
        resolved_version,
        dependency_versions,
        native_addon,
        native_diagnostic,
    })
}

/// Materializes one exact package export from an existing `node_modules`.
pub fn add_installed_subpath(
    registry_dir: &Path,
    node_modules_dir: &Path,
    specifier: &str,
) -> Result<(), String> {
    let (name, subpath) = split_bare_spec(specifier);
    let subpath = subpath.ok_or_else(|| format!("`{specifier}` has no package subpath"))?;
    let package_dir = node_modules_dir.join(name);
    let manifest = read_manifest(&package_dir)?;
    let export = package_subpath_exports(&manifest, &package_dir)?
        .into_iter()
        .find(|export| export.subpath == subpath)
        .ok_or_else(|| format!("package `{name}` has no export named `./{subpath}`"))?;
    write_installed_subpath(
        registry_dir,
        node_modules_dir,
        name,
        &package_dir,
        &export,
        &mut HashMap::new(),
    )?;
    Ok(())
}

fn write_installed_subpath(
    registry_dir: &Path,
    node_modules_dir: &Path,
    name: &str,
    package_dir: &Path,
    export: &PackageSubpathExport,
    bundle_source_cache: &mut HashMap<PathBuf, (String, ModuleAnalysis)>,
) -> Result<BTreeMap<String, String>, String> {
    let (subpath_js, _, _, dependencies) = bundle_commonjs_package_cached(
        node_modules_dir,
        name,
        package_dir,
        &export.runtime_entry,
        bundle_source_cache,
    )?;
    let types_path = package_dir.join(&export.types_entry);
    let subpath_dts = fs::read_to_string(&types_path).map_err(|error| {
        format!(
            "failed to read package export `./{}` types `{}`: {error}",
            export.subpath,
            types_path.display()
        )
    })?;
    let subpath_dts = dts_source_with_reexported_functions(&types_path, &subpath_dts)?;
    let destination = registry_dir.join(name).join("subpaths").join(&export.subpath);
    fs::create_dir_all(&destination)
        .map_err(|error| format!("failed to create `{}`: {error}", destination.display()))?;
    fs::write(destination.join("package.d.ts"), subpath_dts)
        .map_err(|error| format!("failed to write `{}`: {error}", destination.join("package.d.ts").display()))?;
    write_bundle(&destination, &subpath_js)?;
    Ok(dependencies)
}

fn write_bundle(directory: &Path, source: &str) -> Result<(), String> {
    let path = directory.join("bundle.js.gz");
    let file = fs::File::create(&path)
        .map_err(|error| format!("failed to create `{}`: {error}", path.display()))?;
    let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    encoder
        .write_all(source.as_bytes())
        .and_then(|_| encoder.finish())
        .map_err(|error| format!("failed to write `{}`: {error}", path.display()))?;
    let legacy = directory.join("bundle.js");
    if legacy.is_file() {
        fs::remove_file(&legacy)
            .map_err(|error| format!("failed to remove `{}`: {error}", legacy.display()))?;
    }
    Ok(())
}
