/// Where a freshly `add`ed package's declarations/JS entry came from,
/// inside the fetched package itself -- informational only, since the
/// scratch directory these were read from is deleted before `add`
/// returns. `bundled_file_count` is how many of the package's own files
/// (`main` plus everything it reaches via same-package relative
/// `require`s) got folded into `bundle.js` -- 1 for a package that's
/// just a single file.
#[derive(Debug, PartialEq)]
pub struct AddedPackage {
    pub dts_relative_path: String,
    pub js_relative_path: String,
    pub bundled_file_count: usize,
    /// The exact version `npm install` actually resolved `package`'s
    /// version-or-range specifier to (read back from the fetched
    /// package's own `package.json`, so a bare package name with no
    /// specifier at all still reports the real version `npm` picked as
    /// "latest", not just an echo of the empty request).
    pub resolved_version: String,
    /// Every real npm package folded into `bundle.js` -- `package` itself
    /// plus every transitive same-install dependency `bundle_commonjs_
    /// package`'s worklist actually walked into (Node builtin polyfills
    /// are not real npm packages and are never included here) -- each
    /// mapped to the version `npm install` resolved *it* to. Always
    /// contains at least `package`'s own entry. Written to `lock.json`
    /// only when it has more than that one entry (see this module's
    /// top-level doc comment).
    pub dependency_versions: BTreeMap<String, String>,
    /// Metadata for an automatically selected bundled `.node` prebuild.
    pub native_addon: Option<NativeAddonMetadata>,
    /// A non-fatal explanation when prebuilds existed but none matched the
    /// current target. The JavaScript fallback remains usable in that case.
    pub native_diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NativeAddonMetadata {
    pub source: String,
    pub sha256: String,
    pub platform: String,
    pub arch: String,
    pub libc: String,
}

#[derive(Debug)]
struct SelectedPrebuild {
    path: PathBuf,
    source: String,
    platform: String,
    arch: String,
    libc: String,
}

fn target_prebuild_components() -> (&'static str, &'static str, &'static str) {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    let libc = if cfg!(target_env = "musl") {
        "musl"
    } else if platform == "linux" {
        "glibc"
    } else {
        "system"
    };
    (platform, arch, libc)
}

fn select_prebuilt_addon(package_dir: &Path) -> Result<Option<SelectedPrebuild>, String> {
    let prebuilds = package_dir.join("prebuilds");
    if !prebuilds.is_dir() {
        return Ok(None);
    }
    let (platform, arch, libc) = target_prebuild_components();
    let target_dir = prebuilds.join(format!("{platform}-{arch}"));
    if !target_dir.is_dir() {
        let mut available = fs::read_dir(&prebuilds)
            .map_err(|error| format!("failed to inspect `{}`: {error}", prebuilds.display()))?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        available.sort();
        return Err(format!(
            "no bundled native addon matches {platform}-{arch}-{libc}; available targets: {}",
            if available.is_empty() {
                "none".into()
            } else {
                available.join(", ")
            }
        ));
    }
    let mut candidates = fs::read_dir(&target_dir)
        .map_err(|error| format!("failed to inspect `{}`: {error}", target_dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "node")
        })
        .filter(|path| {
            let musl = path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains(".musl."));
            (libc == "musl") == musl || platform != "linux"
        })
        .collect::<Vec<_>>();
    candidates.sort();
    let Some(path) = candidates.into_iter().next() else {
        return Err(format!(
            "bundled addons exist for {platform}-{arch}, but none match libc `{libc}`"
        ));
    };
    let relative_path = path
        .strip_prefix(package_dir)
        .unwrap_or(&path)
        .to_string_lossy()
        .into_owned();
    let source = fs::read_to_string(package_dir.join(".thaw-prebuild-source"))
        .ok()
        .map(|source| source.trim().to_string())
        .filter(|source| !source.is_empty())
        .unwrap_or(relative_path);
    Ok(Some(SelectedPrebuild {
        path,
        source,
        platform: platform.into(),
        arch: arch.into(),
        libc: libc.into(),
    }))
}

fn select_optional_dependency_addon(
    node_modules_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<Option<SelectedPrebuild>, String> {
    let Some(optional) = manifest
        .get("optionalDependencies")
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(None);
    };
    let (platform, arch, libc) = target_prebuild_components();
    let suffix = if platform == "linux" {
        format!("-{platform}-{arch}-{libc}")
    } else {
        format!("-{platform}-{arch}")
    };
    let mut names = optional
        .keys()
        .filter(|name| name.ends_with(&suffix))
        .collect::<Vec<_>>();
    names.sort();
    for name in names {
        let dependency_dir = node_modules_dir.join(name);
        if !dependency_dir.is_dir() {
            continue;
        }
        let dependency_manifest = read_manifest(&dependency_dir)?;
        let main = dependency_manifest
            .get("main")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("index.js");
        let path = dependency_dir.join(main);
        if path
            .extension()
            .is_some_and(|extension| extension == "node")
            && path.is_file()
        {
            let relative_path = path
                .strip_prefix(node_modules_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            return Ok(Some(SelectedPrebuild {
                path,
                source: relative_path,
                platform: platform.into(),
                arch: arch.into(),
                libc: libc.into(),
            }));
        }
    }
    Ok(None)
}

fn github_repository(manifest: &serde_json::Value) -> Option<String> {
    let repository = manifest.get("repository")?;
    let raw = repository
        .as_str()
        .or_else(|| repository.get("url").and_then(serde_json::Value::as_str))?;
    let normalized = raw
        .strip_prefix("git+")
        .unwrap_or(raw)
        .strip_prefix("https://github.com/")?
        .trim_end_matches(".git")
        .trim_end_matches('/');
    let mut parts = normalized.split('/');
    let owner = parts.next()?;
    let repository = parts.next()?;
    if owner.is_empty() || repository.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{repository}"))
}

fn prebuild_install_asset(
    manifest: &serde_json::Value,
) -> Option<(String, String, String, String)> {
    let binary = manifest.get("binary")?;
    let napi = binary
        .get("napi_versions")?
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_u64)
        .filter(|version| *version <= 8)
        .max()?;
    let name = manifest.get("name")?.as_str()?;
    let version = manifest.get("version")?.as_str()?;
    let repository = github_repository(manifest)?;
    let (platform, arch, libc) = target_prebuild_components();
    let platform = if platform == "linux" && libc == "musl" {
        "linuxmusl"
    } else {
        platform
    };
    let asset = format!("{name}-v{version}-napi-v{napi}-{platform}-{arch}.tar.gz");
    let url = format!("https://github.com/{repository}/releases/download/v{version}/{asset}");
    Some((url, asset, platform.to_string(), arch.to_string()))
}

fn find_node_file(root: &Path) -> Result<Option<PathBuf>, String> {
    let mut directories = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("failed to inspect `{}`: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to inspect an entry in `{}`: {error}",
                    directory.display()
                )
            })?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("failed to inspect `{}`: {error}", path.display()))?;
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "node")
            {
                matches.push(path);
            }
        }
    }
    matches.sort();
    if matches.len() > 1 {
        return Err(format!(
            "downloaded prebuild contains multiple `.node` files: {}",
            matches
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(matches.pop())
}

fn download_prebuild_install_addon(
    package_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<(), String> {
    if package_dir.join("prebuilds").is_dir() {
        return Ok(());
    }
    let Some((url, asset, _, _)) = prebuild_install_asset(manifest) else {
        return Ok(());
    };
    let mut response = ureq::get(&url)
        .call()
        .map_err(|error| format!("failed to download native prebuild `{url}`: {error}"))?;
    let archive_bytes = response
        .body_mut()
        .with_config()
        .limit(128 * 1024 * 1024)
        .read_to_vec()
        .map_err(|error| format!("failed to read native prebuild `{url}`: {error}"))?;
    let unpack_dir = package_dir.join(".thaw-prebuild");
    if unpack_dir.is_dir() {
        fs::remove_dir_all(&unpack_dir)
            .map_err(|error| format!("failed to clear `{}`: {error}", unpack_dir.display()))?;
    }
    fs::create_dir_all(&unpack_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", unpack_dir.display()))?;
    let decoder = flate2::read::GzDecoder::new(archive_bytes.as_slice());
    tar::Archive::new(decoder)
        .unpack(&unpack_dir)
        .map_err(|error| format!("failed to unpack native prebuild `{asset}`: {error}"))?;
    let addon = find_node_file(&unpack_dir)?
        .ok_or_else(|| format!("native prebuild `{asset}` contains no `.node` file"))?;
    let (platform, arch, _) = target_prebuild_components();
    let target = package_dir
        .join("prebuilds")
        .join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target)
        .map_err(|error| format!("failed to create `{}`: {error}", target.display()))?;
    let destination =
        target.join(addon.file_name().ok_or_else(|| {
            format!("native prebuild path `{}` has no filename", addon.display())
        })?);
    fs::copy(&addon, &destination).map_err(|error| {
        format!(
            "failed to copy downloaded native addon to `{}`: {error}",
            destination.display()
        )
    })?;
    fs::write(package_dir.join(".thaw-prebuild-source"), url)
        .map_err(|error| format!("failed to record downloaded native prebuild source: {error}"))?;
    Ok(())
}

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
    download_prebuild_install_addon(&package_dir, &manifest)?;
    let fallback_dts = if find_own_dts(&manifest, &package_dir).is_none() {
        Some(fetch_types_package_dts(scratch, name)?)
    } else {
        None
    };
    add_installed_inner(registry_dir, &node_modules_dir, name, fallback_dts)
}

/// Registers a package that already exists under `node_modules_dir`.
/// This is the filesystem half of [`add`], exposed for offline/vendor
/// workflows that have already performed dependency installation.
pub fn add_installed(
    registry_dir: &Path,
    node_modules_dir: &Path,
    name: &str,
) -> Result<AddedPackage, String> {
    add_installed_inner(registry_dir, node_modules_dir, name, None)
}

fn add_installed_inner(
    registry_dir: &Path,
    node_modules_dir: &Path,
    name: &str,
    fallback_dts: Option<(String, String)>,
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
    let (js_source, js_relative_path, bundled_file_count, mut dependency_versions) =
        bundle_commonjs_package(node_modules_dir, name, &package_dir, main_field)?;

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
        None => select_optional_dependency_addon(node_modules_dir, &manifest),
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
    fs::write(dest_dir.join("package.d.ts"), dts_source).map_err(|e| {
        format!(
            "failed to write `{}`: {e}",
            dest_dir.join("package.d.ts").display()
        )
    })?;
    fs::write(dest_dir.join("bundle.js"), js_source).map_err(|e| {
        format!(
            "failed to write `{}`: {e}",
            dest_dir.join("bundle.js").display()
        )
    })?;
    let subpaths_dir = dest_dir.join("subpaths");
    if subpaths_dir.is_dir() {
        fs::remove_dir_all(&subpaths_dir).map_err(|error| {
            format!(
                "failed to remove stale `{}`: {error}",
                subpaths_dir.display()
            )
        })?;
    }
    for export in package_subpath_exports(&manifest, &package_dir)? {
        let subpath = export.subpath;
        let (subpath_js, _, _, subpath_dependencies) =
            bundle_commonjs_package(node_modules_dir, name, &package_dir, &export.runtime_entry)?;
        dependency_versions.extend(subpath_dependencies);
        let types_path = package_dir.join(&export.types_entry);
        let subpath_dts = fs::read_to_string(&types_path).map_err(|error| {
            format!(
                "failed to read package export `./{subpath}` types `{}`: {error}",
                types_path.display()
            )
        })?;
        let subpath_dts = dts_source_with_reexported_functions(&types_path, &subpath_dts)?;
        let subpath_dest = subpaths_dir.join(&subpath);
        fs::create_dir_all(&subpath_dest)
            .map_err(|error| format!("failed to create `{}`: {error}", subpath_dest.display()))?;
        fs::write(subpath_dest.join("package.d.ts"), subpath_dts).map_err(|error| {
            format!(
                "failed to write `{}`: {error}",
                subpath_dest.join("package.d.ts").display()
            )
        })?;
        fs::write(subpath_dest.join("bundle.js"), subpath_js).map_err(|error| {
            format!(
                "failed to write `{}`: {error}",
                subpath_dest.join("bundle.js").display()
            )
        })?;
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

fn dts_source_with_reexported_functions(
    entry_path: &Path,
    entry_source: &str,
) -> Result<String, String> {
    use thaw_parser::ast::{ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};

    let module = thaw_parser::parse_typescript(entry_source)?;
    let mut output = entry_source.to_string();
    let mut seen = std::collections::BTreeSet::new();
    for item in module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            continue;
        };
        if export.type_only {
            continue;
        }
        let Some(source) = export.src else {
            continue;
        };
        let Some(source) = source.value.as_str() else {
            continue;
        };
        let Some(target_path) = declaration_reexport_path(entry_path, source) else {
            continue;
        };
        for specifier in export.specifiers {
            let ExportSpecifier::Named(named) = specifier else {
                continue;
            };
            if named.is_type_only {
                continue;
            }
            let export_name = |name: &ModuleExportName| match name {
                ModuleExportName::Ident(name) => Some(name.sym.to_string()),
                ModuleExportName::Str(_) => None,
            };
            let Some(original) = export_name(&named.orig) else {
                continue;
            };
            let exported = named
                .exported
                .as_ref()
                .and_then(export_name)
                .unwrap_or_else(|| original.clone());
            if seen.contains(&exported) {
                continue;
            }
            let mut visited = std::collections::BTreeSet::new();
            let declarations = reexported_function_declarations(
                &target_path,
                &original,
                &mut visited,
            )?;
            if !declarations.is_empty() {
                seen.insert(exported.clone());
            }
            for mut snippet in declarations {
                if exported != original {
                    snippet = snippet.replacen(
                        &format!("function {original}"),
                        &format!("function {exported}"),
                        1,
                    );
                }
                output.push('\n');
                output.push_str(&snippet);
            }
        }
    }
    Ok(output)
}

fn reexported_function_declarations(
    path: &Path,
    name: &str,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};
    use thaw_parser::common::{SourceMapper, Spanned};

    if !visited.insert((path.to_path_buf(), name.to_string())) {
        return Ok(Vec::new());
    }
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read re-exported declarations `{}`: {error}",
            path.display()
        )
    })?;
    let (module, source_map) = thaw_parser::parse_typescript_with_source_map(&source)?;
    let mut declarations = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(declaration)) = item else {
            continue;
        };
        let Decl::Fn(function) = &declaration.decl else {
            continue;
        };
        if function.ident.sym == name {
            declarations.push(
                source_map
                    .span_to_snippet(declaration.span())
                    .map_err(|error| {
                        format!("failed to read declaration for `{name}`: {error:?}")
                    })?,
            );
        }
    }
    if !declarations.is_empty() {
        return Ok(declarations);
    }
    for item in module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            continue;
        };
        if export.type_only {
            continue;
        }
        let Some(source) = export.src.and_then(|source| source.value.as_str().map(str::to_owned))
        else {
            continue;
        };
        let Some(target_path) = declaration_reexport_path(path, &source) else {
            continue;
        };
        for specifier in export.specifiers {
            let ExportSpecifier::Named(named) = specifier else {
                continue;
            };
            if named.is_type_only {
                continue;
            }
            let export_name = |name: &ModuleExportName| match name {
                ModuleExportName::Ident(name) => Some(name.sym.to_string()),
                ModuleExportName::Str(_) => None,
            };
            let original = export_name(&named.orig);
            let exported = named.exported.as_ref().and_then(export_name).or_else(|| original.clone());
            if exported.as_deref() == Some(name) {
                return reexported_function_declarations(
                    &target_path,
                    original.as_deref().unwrap_or(name),
                    visited,
                );
            }
        }
    }
    Ok(Vec::new())
}

fn declaration_reexport_path(entry_path: &Path, source: &str) -> Option<PathBuf> {
    if !source.starts_with('.') {
        return None;
    }
    let path = entry_path.parent()?.join(source);
    [path.with_extension("d.ts"), path.join("index.d.ts")]
        .into_iter()
        .find(|candidate| candidate.is_file())
}

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

fn select_export_condition<'a>(
    value: &'a serde_json::Value,
    conditions: &[&str],
) -> Option<&'a str> {
    if let Some(path) = value.as_str() {
        return Some(path);
    }
    if let Some(candidates) = value.as_array() {
        return candidates
            .iter()
            .find_map(|candidate| select_export_condition(candidate, conditions));
    }
    let object = value.as_object()?;
    for condition in conditions {
        if let Some(path) = object
            .get(*condition)
            .and_then(|value| select_export_condition(value, conditions))
        {
            return Some(path);
        }
    }
    if conditions == ["types"] {
        for child in object.values() {
            if let Some(path) = select_export_condition(child, conditions) {
                return Some(path);
            }
        }
    }
    None
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
    if suffix.contains('*') || !path.starts_with(prefix) || !path.ends_with(suffix) {
        return None;
    }
    Some(&path[prefix.len()..path.len() - suffix.len()])
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
        let Some(runtime) = select_export_condition(target, &["require", "import", "default"])
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
                subpaths.push(PackageSubpathExport {
                    subpath: expanded,
                    runtime_entry: runtime.replace('*', capture),
                    types_entry: types.replace('*', capture),
                });
            }
            continue;
        }
        validate_export_subpath(subpath)?;
        subpaths.push(PackageSubpathExport {
            subpath: subpath.to_string(),
            runtime_entry: runtime.to_string(),
            types_entry: types.to_string(),
        });
    }
    subpaths.sort_by(|left, right| left.subpath.cmp(&right.subpath));
    subpaths.dedup_by(|left, right| left.subpath == right.subpath);
    Ok(subpaths)
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

    Ok((format!("{types_package}/{rel}"), source))
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
fn resolve_module_path(package_dir: &Path, path: &str) -> Result<(String, PathBuf), String> {
    let trimmed = path.trim_end_matches('/');
    let directory = package_dir.join(trimmed);
    if directory.is_dir() {
        if let Ok(manifest) = read_manifest(&directory) {
            let entry =
                package_export_target(&manifest, None, &["require", "import", "node", "default"])
                    .or_else(|| manifest.get("main").and_then(|value| value.as_str()));
            if let Some(entry) = entry {
                if let Ok((relative, absolute)) = resolve_module_path(&directory, entry) {
                    return Ok((
                        normalize_path_string(&format!("{trimmed}/{relative}")),
                        absolute,
                    ));
                }
            }
        }
    }
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
    for candidate in &candidates {
        let resolved = package_dir.join(candidate);
        if resolved.is_file() {
            return Ok((candidate.clone(), resolved));
        }
    }
    Err(format!(
        "couldn't find a JS module for `\"{path}\"` (tried `{}`)",
        candidates.join("`, `")
    ))
}
