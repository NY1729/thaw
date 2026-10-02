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
    validate_installed_package_name(split_package_spec(package).0)?;
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

fn validate_installed_package_name(name: &str) -> Result<(), String> {
    let parts: Vec<&str> = name.split('/').collect();
    let valid = if name.starts_with('@') {
        parts.len() == 2 && parts[0].len() > 1
    } else {
        parts.len() == 1
    };
    if !valid || parts.iter().any(|part| part.is_empty() || *part == "." || *part == ".." || part.contains('\\')) {
        return Err(format!("invalid installed package name `{name}`"));
    }
    Ok(())
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

// The stage names are exclusive but do not serialize two writers that read
// the same installed root. Hold this sibling sidecar through the root read,
// stage, publication, and rollback. A stale sidecar is reported rather than
// guessed dead from its timestamp/PID.
struct PackageWriterGuard {
    path: PathBuf,
    token: String,
}

impl PackageWriterGuard {
    fn acquire(destination: &Path) -> Result<Self, String> {
        let parent = destination.parent().ok_or_else(|| format!("package `{}` has no parent", destination.display()))?;
        fs::create_dir_all(parent).map_err(|error| format!("failed to create `{}`: {error}", parent.display()))?;
        let name = destination.file_name().unwrap_or_default().to_string_lossy();
        let path = parent.join(format!(".{name}.writer-lock"));
        static NEXT_WRITER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let token = format!("{}-{}", std::process::id(), NEXT_WRITER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path)
            .map_err(|error| format!("cannot reserve package writer `{}`: {error}; if stale, verify no writer is active before removing it", path.display()))?;
        if let Err(error) = file.write_all(token.as_bytes()) {
            // A partial token cannot prove ownership at cleanup. Leave the
            // sidecar fail-closed for explicit recovery.
            return Err(format!("failed to write package writer `{}`: {error}; verify no writer is active before removing it", path.display()));
        }
        Ok(Self { path, token })
    }
}

impl Drop for PackageWriterGuard {
    fn drop(&mut self) {
        // A manually replaced sidecar is no longer ours. Cooperating writers
        // never remove another holder's file, including on error paths.
        if fs::read_to_string(&self.path).ok().as_deref() == Some(&self.token) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredInstanceIndex {
    format: u32,
    // Empty key is the package main; every other key is an exact export.
    // Keeping this only at the package root avoids colliding with an export
    // named `foo/instances.json` or `foo/lock.json`.
    exports: BTreeMap<String, Vec<StoredInstance>>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct StoredInstance {
    identity: String,
    name: String,
    locator: String,
    version: String,
}

type InstanceVersions = BTreeMap<PathBuf, (String, String, String)>;

fn instance_identity(node_modules_dir: &Path, directory: &Path) -> Result<String, String> {
    if let Ok(relative) = directory.strip_prefix(node_modules_dir) {
        if relative.as_os_str().is_empty()
            || relative.components().any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(format!("invalid package instance `{}`", directory.display()));
        }
        let path = relative.to_str().ok_or_else(|| format!("non-UTF-8 package instance `{}`", directory.display()))?;
        Ok(format!("node_modules:{path}"))
    } else {
        let absolute = if directory.is_absolute() {
            directory.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| format!("cannot locate package instance `{}`: {error}", directory.display()))?
                .join(directory)
        };
        let path = absolute.to_str().ok_or_else(|| format!("non-UTF-8 package instance `{}`", directory.display()))?;
        Ok(format!("external:{path}"))
    }
}

fn stored_instances(node_modules_dir: &Path, versions: &InstanceVersions) -> Result<Vec<StoredInstance>, String> {
    versions.iter().map(|(path, (name, locator, version))| {
        Ok(StoredInstance {
            identity: instance_identity(node_modules_dir, path)?,
            name: name.clone(), locator: locator.clone(), version: version.clone(),
        })
    }).collect()
}

fn write_instance_index(directory: &Path, index: &StoredInstanceIndex) -> Result<(), String> {
    let source = serde_json::to_string_pretty(index)
        .map_err(|error| format!("failed to serialize instance index: {error}"))?;
    fs::write(directory.join("instances.json"), source)
        .map_err(|error| format!("failed to write `{}`: {error}", directory.join("instances.json").display()))
}

fn read_instance_index(directory: &Path) -> Result<StoredInstanceIndex, String> {
    let path = directory.join("instances.json");
    let source = fs::read_to_string(&path).map_err(|error| {
        format!("cannot read `{}`: {error}; re-register the installed package before adding a lazy subpath", path.display())
    })?;
    let index: StoredInstanceIndex = serde_json::from_str(&source)
        .map_err(|error| format!("invalid `{}`: {error}; re-register the installed package", path.display()))?;
    if index.format != 2 || !index.exports.contains_key("") {
        return Err(format!("unsupported `{}`; re-register the installed package", path.display()));
    }
    for (export, instances) in &index.exports {
        if !export.is_empty() {
            validate_export_subpath(export)?;
        }
        instance_versions(instances, &path)?;
    }
    Ok(index)
}

fn instance_versions(instances: &[StoredInstance], path: &Path) -> Result<InstanceVersions, String> {
    let mut versions = BTreeMap::new();
    for instance in instances {
        let (kind, key) = instance.identity.split_once(':').ok_or_else(|| format!("invalid identity in `{}`", path.display()))?;
        let valid = match kind {
            "node_modules" => !key.is_empty() && Path::new(key).components().all(|part| matches!(part, std::path::Component::Normal(_))),
            "external" => Path::new(key).is_absolute(),
            _ => false,
        };
        if !valid || instance.name.is_empty() || instance.locator.is_empty() || instance.version.is_empty() {
            return Err(format!("invalid instance in `{}`", path.display()));
        }
        let previous = versions.insert(PathBuf::from(&instance.identity), (instance.name.clone(), instance.locator.clone(), instance.version.clone()));
        if previous.is_some() { return Err(format!("duplicate identity in `{}`", path.display())); }
    }
    Ok(versions)
}

fn project_all_instances(index: &StoredInstanceIndex, path: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut versions = BTreeMap::new();
    for records in index.exports.values() {
        merge_instance_versions(&mut versions, instance_versions(records, path)?)?;
    }
    project_package_versions(versions)
}

fn merge_instance_versions(into: &mut InstanceVersions, from: InstanceVersions) -> Result<(), String> {
    for (identity, record) in from {
        if let Some(previous) = into.get(&identity) {
            if previous != &record {
                return Err(format!("package instance `{}` changed between registered bundles; re-register the installed package", identity.display()));
            }
        } else {
            into.insert(identity, record);
        }
    }
    Ok(())
}

fn write_projected_lock(directory: &Path, versions: &BTreeMap<String, String>) -> Result<(), String> {
    let path = directory.join("lock.json");
    if versions.len() <= 1 {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("failed to remove `{}`: {error}", path.display())),
        }
        return Ok(());
    }
    let source = serde_json::to_string_pretty(versions)
        .map_err(|error| format!("failed to serialize `{}`: {error}", path.display()))?;
    fs::write(&path, source).map_err(|error| format!("failed to write `{}`: {error}", path.display()))
}

/// Reserve sibling paths on the same filesystem as the package destination.
/// The candidate directory is created exclusively; the backup name is checked
/// before publication, so an abandoned backup from an earlier run is never reused.
fn staged_package_paths(destination: &Path) -> Result<(PathBuf, PathBuf), String> {
    let parent = destination.parent().ok_or_else(|| {
        format!("package destination `{}` has no parent", destination.display())
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create `{}`: {error}", parent.display()))?;
    static NEXT_STAGE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = destination.file_name().unwrap_or_default().to_string_lossy();
    loop {
        let serial = NEXT_STAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let suffix = format!("{}-{serial}", std::process::id());
        let staged = parent.join(format!(".{name}.staged-{suffix}"));
        let backup = parent.join(format!(".{name}.previous-{suffix}"));
        match fs::create_dir(&staged) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("failed to create `{}`: {error}", staged.display()));
            }
        }
        match fs::symlink_metadata(&backup) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((staged, backup));
            }
            Ok(_) => {
                fs::remove_dir(&staged)
                    .map_err(|error| format!("failed to release `{}`: {error}", staged.display()))?;
            }
            Err(error) => {
                let _ = fs::remove_dir(&staged);
                return Err(format!("failed to inspect `{}`: {error}", backup.display()));
            }
        }
    }
}

fn remove_saved_package(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn discard_staged_package(staged: &Path, cause: String) -> String {
    match fs::remove_dir_all(staged) {
        Ok(()) => cause,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => cause,
        Err(error) => format!(
            "{cause}; failed to remove staged package `{}`: {error}",
            staged.display()
        ),
    }
}

fn publish_staged_package(staged: &Path, destination: &Path, backup: &Path) -> Result<(), String> {
    let had_previous = match fs::symlink_metadata(destination) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(format!("failed to inspect `{}`: {error}", destination.display()));
        }
    };
    if had_previous {
        match fs::symlink_metadata(backup) {
            Ok(_) => {
                return Err(format!("backup path `{}` is already occupied", backup.display()));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("failed to inspect `{}`: {error}", backup.display()));
            }
        }
        fs::rename(destination, backup).map_err(|error| {
            format!("failed to save `{}` as `{}`: {error}", destination.display(), backup.display())
        })?;
    }
    if let Err(error) = fs::rename(staged, destination) {
        if had_previous {
            if let Err(restore) = fs::rename(backup, destination) {
                return Err(format!(
                    "failed to install `{}`: {error}; rollback failed: {restore}; previous package remains in `{}`",
                    destination.display(), backup.display()
                ));
            }
        }
        return Err(format!("failed to install `{}`: {error}", destination.display()));
    }
    if had_previous {
        // Publication succeeded. Cleanup failure leaves only an orphaned backup;
        // it must not turn a successful replacement into a reported failed add.
        let _ = remove_saved_package(backup);
    }
    Ok(())
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
    let main_field = package_export_target(&manifest, None, &["require", "node", "default"])
        .or_else(|| manifest.get("main").and_then(|v| v.as_str()))
        .unwrap_or("index.js");
    // The main may itself be a .node file supplied by the download below.
    // Defer an early bundle error until after that opportunity, while using
    // an available JS bundle to discover hidden generated addon packages.
    let early_bundle = bundle_commonjs_package_cached(
        &node_modules_dir,
        name,
        &package_dir,
        main_field,
        &mut SourceCache::default(),
    );
    let js_source = early_bundle.as_ref().ok().map(|(source, _, _, _)| source.as_str());
    let mut candidate_diagnostics = Vec::new();
    if select_installed_addon(
        &package_dir,
        &node_modules_dir,
        &manifest,
        js_source,
        &mut candidate_diagnostics,
    )?.is_none() {
        if let Err(download_error) = download_prebuild_install_addon(&package_dir, &manifest) {
            return Err(match early_bundle {
                Err(source_error) => format!("{source_error}; prebuild download also failed: {download_error}"),
                Ok(_) => download_error,
            });
        }
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
    validate_installed_package_name(name)?;
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
    validate_installed_package_name(name)?;
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
    validate_installed_package_name(name)?;
    let package_dir = node_modules_dir.join(name);
    let manifest = read_manifest(&package_dir)?;

    let resolved_version = manifest
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();

    let main_field = package_export_target(&manifest, None, &["require", "node", "default"])
        .or_else(|| manifest.get("main").and_then(|v| v.as_str()))
        .unwrap_or("index.js");
    let mut bundle_source_cache = SourceCache::default();
    let (js_source, js_relative_path, bundled_file_count, mut dependency_versions) =
        bundle_commonjs_package_cached(
            node_modules_dir,
            name,
            &package_dir,
            main_field,
            &mut bundle_source_cache,
        )?;
    let root_instances = bundle_source_cache.last_bundle_versions.clone();
    let root_records = stored_instances(node_modules_dir, &root_instances)?;

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

    let mut candidate_diagnostics = Vec::new();
    let selected_addon = select_installed_addon(
        &package_dir,
        node_modules_dir,
        &manifest,
        Some(js_source.as_str()),
        &mut candidate_diagnostics,
    )?;
    // Inspect and read the chosen binary before touching a previous install.
    let prepared_addon = selected_addon.map(|selected| {
        let bytes = fs::read(&selected.path).map_err(|error| {
            format!("failed to read native addon `{}`: {error}", selected.path.display())
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
        Ok::<_, String>((bytes, metadata, metadata_json))
    }).transpose()?;
    let native_diagnostic = if prepared_addon.is_none() && !candidate_diagnostics.is_empty() {
        Some(candidate_diagnostics.join("; "))
    } else {
        None
    };

    let destination = registry_dir.join(name);
    let _writer = PackageWriterGuard::acquire(&destination)?;
    let (dest_dir, backup_dir) = staged_package_paths(&destination)?;
    let staged_result = (|| -> Result<AddedPackage, String> {
        let native_addon = match prepared_addon {
            Some((bytes, metadata, metadata_json)) => {
                fs::write(dest_dir.join("native.node"), &bytes).map_err(|error| {
                    format!("failed to write `{}`: {error}", dest_dir.join("native.node").display())
                })?;
                fs::write(dest_dir.join("native-addon.json"), metadata_json).map_err(|error| {
                    format!("failed to write `{}`: {error}", dest_dir.join("native-addon.json").display())
                })?;
                Some(metadata)
            }
            None => None,
        };
        let dependencies = dest_dir.join("native-dependencies");
        let shared_libraries = platform_shared_libraries(node_modules_dir, &manifest)?;
        if !shared_libraries.is_empty() {
            fs::create_dir_all(&dependencies).map_err(|error| error.to_string())?;
            for (index, source) in shared_libraries.iter().enumerate() {
                let name = source.file_name().unwrap_or_default().to_string_lossy();
                fs::copy(source, dependencies.join(format!("{index}-{name}")))
                    .map_err(|error| format!("failed to copy `{}`: {error}", source.display()))?;
            }
        }
        let platform_executable = dest_dir.join("platform-executable");
        if let Some(executable) = select_optional_dependency_executable(node_modules_dir, &manifest)? {
            fs::copy(&executable, &platform_executable).map_err(|error| {
                format!(
                    "failed to copy platform executable `{}`: {error}",
                    executable.display()
                )
            })?;
        }
        fs::write(dest_dir.join("package.d.ts"), dts_source).map_err(|e| {
            format!(
                "failed to write `{}`: {e}",
                dest_dir.join("package.d.ts").display()
            )
        })?;
        write_bundle(&dest_dir, &js_source)?;
        let mut index = StoredInstanceIndex { format: 2, exports: BTreeMap::new() };
        index.exports.insert(String::new(), root_records);
        if eager_subpaths {
            for export in package_subpath_exports(&manifest, &package_dir)? {
                write_installed_subpath(
                    &dest_dir.join("subpaths").join(&export.subpath),
                    node_modules_dir,
                    name,
                    &package_dir,
                    &export,
                    &mut bundle_source_cache,
                )?;
                index.exports.insert(
                    export.subpath,
                    stored_instances(node_modules_dir, &bundle_source_cache.last_bundle_versions)?,
                );
            }
            dependency_versions = project_package_versions(bundle_source_cache.package_versions.clone())?;
        }
        write_instance_index(&dest_dir, &index)?;
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
        write_projected_lock(&dest_dir, &dependency_versions)?;

        Ok(AddedPackage {
            dts_relative_path,
            js_relative_path,
            bundled_file_count,
            resolved_version,
            dependency_versions,
            native_addon,
            native_diagnostic,
        })
    })();
    let added = match staged_result {
        Ok(added) => added,
        Err(error) => return Err(discard_staged_package(&dest_dir, error)),
    };
    if let Err(error) = publish_staged_package(&dest_dir, &destination, &backup_dir) {
        return Err(discard_staged_package(&dest_dir, error));
    }
    Ok(added)
}

// Copy a whole installed package for one parent-level publication. A lazy
// export can itself be the parent of another materialized export (`foo` and
// `foo/bar`); replacing its own files must retain its descendants.
fn copy_installed_subpath_tree(source: &Path, staged: &Path) -> Result<std::fs::Permissions, String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("failed to inspect `{}`: {error}", source.display()))?;
    if !metadata.is_dir() {
        return Err(format!("installed subpath `{}` is not a directory", source.display()));
    }
    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read `{}`: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read `{}`: {error}", source.display()))?;
        let from = entry.path();
        let to = staged.join(entry.file_name());
        let kind = entry.file_type()
            .map_err(|error| format!("failed to inspect `{}`: {error}", from.display()))?;
        if kind.is_dir() {
            fs::create_dir(&to)
                .map_err(|error| format!("failed to create `{}`: {error}", to.display()))?;
            let permissions = copy_installed_subpath_tree(&from, &to)?;
            fs::set_permissions(&to, permissions)
                .map_err(|error| format!("failed to preserve `{}` permissions: {error}", to.display()))?;
        } else if kind.is_file() {
            fs::copy(&from, &to)
                .map_err(|error| format!("failed to copy `{}`: {error}", from.display()))?;
        } else {
            return Err(format!("cannot preserve unsupported installed entry `{}`", from.display()));
        }
    }
    Ok(metadata.permissions())
}

fn collect_materialized_exports(
    root: &Path,
    directory: &Path,
    exports: &mut std::collections::BTreeSet<String>,
) -> Result<(), String> {
    if !directory.exists() { return Ok(()); }
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("failed to read `{}`: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read `{}`: {error}", directory.display()))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| format!("failed to inspect `{}`: {error}", path.display()))?;
        if kind.is_file() { continue; }
        if !kind.is_dir() {
            return Err(format!("unsupported installed subpath entry `{}`", path.display()));
        }
        let has_dts = path.join("package.d.ts").is_file();
        let has_bundle = path.join("bundle.js.gz").is_file() || path.join("bundle.js").is_file();
        if has_dts != has_bundle {
            return Err(format!("incomplete installed export `{}`; re-register the package", path.display()));
        }
        if has_dts {
            let relative = path.strip_prefix(root).map_err(|error| format!("invalid installed export path: {error}"))?;
            let key = relative.to_str().ok_or_else(|| format!("non-UTF-8 installed export `{}`", path.display()))?.replace('\\', "/");
            validate_export_subpath(&key)?;
            exports.insert(key);
        }
        collect_materialized_exports(root, &path, exports)?;
    }
    Ok(())
}

/// Materializes one exact package export from an existing `node_modules`.
pub fn add_installed_subpath(
    registry_dir: &Path,
    node_modules_dir: &Path,
    specifier: &str,
) -> Result<(), String> {
    let (name, subpath) = split_bare_spec(specifier);
    validate_installed_package_name(name)?;
    let subpath = subpath.ok_or_else(|| format!("`{specifier}` has no package subpath"))?;
    let package_dir = node_modules_dir.join(name);
    let manifest = read_manifest(&package_dir)?;
    let export = package_subpath_exports(&manifest, &package_dir)?
        .into_iter()
        .find(|export| export.subpath == subpath)
        .ok_or_else(|| format!("package `{name}` has no export named `./{subpath}`"))?;
    let destination = registry_dir.join(name);
    let _writer = PackageWriterGuard::acquire(&destination)?;
    // A root made by an older writer has only a projected lock; it cannot
    // identify a same-name nested package instance across lazy calls.
    let mut index = read_instance_index(&destination)?;
    let stored_version = fs::read_to_string(destination.join("version.txt"))
        .map_err(|error| format!("cannot read installed root version: {error}; re-register `{name}`"))?;
    let current_version = manifest.get("version").and_then(|value| value.as_str()).unwrap_or("0.0.0");
    if stored_version.trim() != current_version {
        return Err(format!("installed `{name}` changed version; re-register it before adding `{specifier}`"));
    }
    let (staged, backup) = staged_package_paths(&destination)?;
    let staged_result = (|| -> Result<(), String> {
        let permissions = copy_installed_subpath_tree(&destination, &staged)?;
        let export_dir = staged.join("subpaths").join(&export.subpath);
        let mut cache = SourceCache::default();
        write_installed_subpath(&export_dir, node_modules_dir, name, &package_dir, &export, &mut cache)?;
        index.exports.insert(
            export.subpath.clone(),
            stored_instances(node_modules_dir, &cache.last_bundle_versions)?,
        );
        let subpaths = staged.join("subpaths");
        let mut materialized = std::collections::BTreeSet::new();
        collect_materialized_exports(&subpaths, &subpaths, &mut materialized)?;
        let recorded = index.exports.keys().filter(|key| !key.is_empty()).cloned().collect::<std::collections::BTreeSet<_>>();
        if materialized != recorded {
            return Err(format!("installed `{name}` has exports without matching instance metadata; re-register the package"));
        }
        let projected = project_all_instances(&index, &staged.join("instances.json"))?;
        write_instance_index(&staged, &index)?;
        write_projected_lock(&staged, &projected)?;
        fs::set_permissions(&staged, permissions)
            .map_err(|error| format!("failed to preserve `{}` permissions: {error}", destination.display()))?;
        Ok(())
    })();
    if let Err(error) = staged_result {
        return Err(discard_staged_package(&staged, error));
    }
    if let Err(error) = publish_staged_package(&staged, &destination, &backup) {
        return Err(discard_staged_package(&staged, error));
    }
    Ok(())
}

fn write_installed_subpath(
    destination: &Path,
    node_modules_dir: &Path,
    name: &str,
    package_dir: &Path,
    export: &PackageSubpathExport,
    bundle_source_cache: &mut SourceCache,
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
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create `{}`: {error}", destination.display()))?;
    fs::write(destination.join("package.d.ts"), subpath_dts)
        .map_err(|error| format!("failed to write `{}`: {error}", destination.join("package.d.ts").display()))?;
    write_bundle(destination, &subpath_js)?;
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
