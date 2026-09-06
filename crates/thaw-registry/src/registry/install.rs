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
    candidates.sort_by_key(|path| {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        (usize::from(!name.starts_with("node.")), name.into_owned())
    });
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
    let suffixes = if platform == "linux" && libc == "musl" {
        vec![format!("-{platform}-{arch}-{libc}"), format!("-linuxmusl-{arch}")]
    } else if platform == "linux" {
        vec![format!("-{platform}-{arch}-{libc}"), format!("-{platform}-{arch}")]
    } else {
        vec![format!("-{platform}-{arch}")]
    };
    let mut names = optional
        .keys()
        .filter(|name| suffixes.iter().any(|suffix| name.ends_with(suffix)))
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
        let main_path = dependency_dir.join(main);
        let path = if main_path
            .extension()
            .is_some_and(|extension| extension == "node")
            && main_path.is_file()
        {
            Some(main_path)
        } else {
            find_node_file(&dependency_dir)?
        };
        if let Some(path) = path {
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

/// Follows `/// <reference path="..." />` directives (the classic
/// DefinitelyTyped-style split for a package whose real API is spread
/// across many files, e.g. lodash's `index.d.ts` referencing a dozen files
/// under `common/`), inlining each referenced file's content -- since
/// thaw-registry discards every individual `.d.ts` source file except the
/// one flattened `package.d.ts` it writes out, a reference to a file about
/// to disappear needs to be followed now or never.
///
/// A referenced file commonly augments the *entry* file's own exported
/// namespace via `declare module "<path back to the entry file>" { ... }`
/// (TS module augmentation) rather than declaring its own top-level types
/// -- lodash's `common/array.d.ts` opens with `declare module "../index" {
/// interface LoDashStatic { chunk(...): ...; } }`. When that module
/// specifier resolves back to `entry_path` itself, and `entry_path` exports
/// a namespace via `export as namespace <name>;`, the augmentation is
/// rewritten to `declare namespace <name> { ... }` so the (repeated, once
/// per referenced file) `interface LoDashStatic { ... }` blocks land in the
/// same namespace the entry file's own declaration lives in, matching how
/// a real TS compiler resolves the augmentation. An augmentation targeting
/// some other module is inlined as its own `declare module "..." { ... }`,
/// unresolved but harmless.
fn inline_triple_slash_references(
    entry_path: &Path,
    source: &str,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<String, String> {
    use thaw_parser::ast::{Decl, ModuleItem, Stmt, TsModuleName, TsNamespaceBody};
    use thaw_parser::common::{SourceMapper, Spanned};

    let namespace = export_as_namespace_name(source)?;
    let mut output = String::new();
    for reference in triple_slash_reference_paths(source) {
        let Some(target_path) = entry_path
            .parent()
            .map(|dir| dir.join(&reference))
            .filter(|path| path.is_file())
        else {
            continue;
        };
        let canonical = target_path
            .canonicalize()
            .unwrap_or_else(|_| target_path.clone());
        if !visited.insert(canonical) {
            continue;
        }
        let referenced_source = fs::read_to_string(&target_path).map_err(|error| {
            format!(
                "failed to read referenced declarations `{}`: {error}",
                target_path.display()
            )
        })?;
        let (referenced_module, source_map) =
            thaw_parser::parse_typescript_with_source_map(&referenced_source)?;
        let canonical_entry = entry_path
            .canonicalize()
            .unwrap_or_else(|_| entry_path.to_path_buf());
        for item in &referenced_module.body {
            let ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(module_decl))) = item else {
                continue;
            };
            let TsModuleName::Str(target) = &module_decl.id else {
                continue;
            };
            let Some(target_specifier) = target.value.as_str() else {
                continue;
            };
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                continue;
            };
            let targets_entry = declaration_reexport_path(&target_path, target_specifier)
                .map(|resolved| resolved.canonicalize().unwrap_or(resolved))
                .is_some_and(|resolved| resolved == canonical_entry);
            let snippets = block
                .body
                .iter()
                .map(|item| {
                    source_map.span_to_snippet(item.span()).map_err(|error| {
                        format!("failed to read module augmentation body: {error:?}")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if targets_entry {
                // A self-targeting augmentation with no exported namespace
                // to merge it into has nothing sensible to attach to;
                // drop it rather than leaving a dangling, unresolvable
                // reference to a namespace that was never declared.
                if let Some(namespace) = &namespace {
                    output.push_str(&format!("\ndeclare namespace {namespace} {{\n"));
                    for snippet in &snippets {
                        output.push_str(snippet);
                        output.push('\n');
                    }
                    output.push_str("}\n");
                }
            } else {
                output.push_str(&format!("\ndeclare module \"{target_specifier}\" {{\n"));
                for snippet in &snippets {
                    output.push_str(snippet);
                    output.push('\n');
                }
                output.push_str("}\n");
            }
        }
        // A referenced file can itself carry further references (not
        // exercised by any package tested so far, but the DefinitelyTyped
        // convention allows it).
        output.push_str(&inline_triple_slash_references(
            &target_path,
            &referenced_source,
            visited,
        )?);
    }
    Ok(output)
}

/// The `/// <reference path="..." />` directives in a `.d.ts` source --
/// these are ordinary comments as far as the parser is concerned (and,
/// per the TS convention this follows, only recognized at the very top of
/// the file), so this scans the raw text rather than the AST.
fn triple_slash_reference_paths(source: &str) -> Vec<String> {
    source
        .lines()
        .take_while(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("///") || trimmed.is_empty()
        })
        .filter_map(|line| {
            let start = line.find("path=\"")? + "path=\"".len();
            let end = start + line[start..].find('"')?;
            Some(line[start..end].to_string())
        })
        .collect()
}

/// `export as namespace <name>;` -- the name a `.d.ts` file's own exported
/// value is additionally reachable under as a global namespace, and (for
/// `inline_triple_slash_references`'s purposes) the namespace a referenced
/// file's self-targeting module augmentation actually means to extend.
fn export_as_namespace_name(source: &str) -> Result<Option<String>, String> {
    use thaw_parser::ast::{ModuleDecl, ModuleItem};

    let module = thaw_parser::parse_typescript(source)?;
    Ok(module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsNamespaceExport(export)) => {
            Some(export.id.sym.to_string())
        }
        _ => None,
    }))
}

fn dts_source_with_reexported_functions(
    entry_path: &Path,
    entry_source: &str,
) -> Result<String, String> {
    use thaw_parser::ast::{ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};

    let module = thaw_parser::parse_typescript(entry_source)?;
    let mut output = entry_source.to_string();
    let mut visited_references = std::collections::BTreeSet::new();
    output.push_str(&inline_triple_slash_references(
        entry_path,
        entry_source,
        &mut visited_references,
    )?);
    let mut seen = std::collections::BTreeSet::new();
    // `import Name = require("./path")` + a *local* `export { Name as
    // exported };` (no `from` clause -- `Name` is already a value bound
    // earlier in this same file, not a re-export of another module's own
    // export) -- real-world example: semver's `index.d.ts`, which
    // imports one function per file this way and re-exports them all
    // together. Each such `Name` resolves through its own file's `export
    // = X;` to the identifier actually declared there (see
    // `export_assignment_function_declarations`), so unlike the
    // `from`-clause case below, the target file can differ per specifier
    // even within one `export { ... }` statement.
    let import_equals_targets = import_equals_targets(entry_path, &module);
    output.push_str(&inline_import_equals_value_type(
        &module,
        &import_equals_targets,
    )?);
    let named_import_targets = named_import_targets(entry_path, &module);
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            continue;
        };
        if export.type_only {
            continue;
        }
        let target_path = match &export.src {
            Some(source) => source
                .value
                .as_str()
                .and_then(|source| declaration_reexport_path(entry_path, source)),
            None => None,
        };
        for specifier in &export.specifiers {
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
            let declarations = match &target_path {
                Some(target_path) => {
                    let mut visited = std::collections::BTreeSet::new();
                    let functions =
                        reexported_function_declarations(target_path, &original, &mut visited)?;
                    if !functions.is_empty() {
                        functions
                    } else {
                        reexported_class_or_interface_declarations(target_path, &original)?
                    }
                }
                None => match import_equals_targets.get(&original) {
                    Some(target_path) => export_assignment_function_declarations(target_path)?,
                    None => match named_import_targets.get(&original) {
                        Some((target_path, target_name)) => {
                            let mut visited = std::collections::BTreeSet::new();
                            let functions = reexported_function_declarations(
                                target_path,
                                target_name,
                                &mut visited,
                            )?;
                            if !functions.is_empty() {
                                functions
                            } else {
                                reexported_class_or_interface_declarations(
                                    target_path,
                                    target_name,
                                )?
                            }
                        }
                        None => continue,
                    },
                },
            };
            if !declarations.is_empty() {
                seen.insert(exported.clone());
            }
            for mut snippet in declarations {
                if exported != original {
                    snippet = rename_declared_function(snippet, &exported);
                }
                output.push('\n');
                output.push_str(&snippet);
            }
        }
    }
    // `export import NAME = BASE.MEMBER;` -- a TS import-equals
    // declaration whose module reference is a qualified *entity name* (a
    // property access into an already-imported value), not a
    // `require(...)` call (`import_equals_targets`'s own shape, handled
    // above). Real example: uuid@8's real `.d.ts` (via `@types/uuid`),
    // `import uuid from "./index.js"; export import v1 = uuid.v1; export
    // import v4 = uuid.v4; ...`. `uuid.MEMBER` resolves to whatever
    // `./index.js`'s own `.d.ts` exports under the plain name `MEMBER` --
    // a default-imported value's shape mirrors its target's own named
    // exports -- so this reuses `reexported_function_declarations`
    // exactly the way a named re-export (`export { X } from "..."`)
    // already does above, just keyed off a qualified-name AST shape
    // instead of an `ExportSpecifier`.
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::TsImportEquals(import)) = item else {
            continue;
        };
        if !import.is_export {
            continue;
        }
        let thaw_parser::ast::TsModuleRef::TsEntityName(thaw_parser::ast::TsEntityName::TsQualifiedName(qualified)) =
            &import.module_ref
        else {
            continue;
        };
        let thaw_parser::ast::TsEntityName::Ident(base) = &qualified.left else {
            continue;
        };
        let Some((target_path, _)) = named_import_targets.get(base.sym.as_str()) else {
            continue;
        };
        let member = qualified.right.sym.to_string();
        let exported = import.id.sym.to_string();
        if seen.contains(&exported) {
            continue;
        }
        let mut visited = std::collections::BTreeSet::new();
        let declarations = reexported_function_declarations(target_path, &member, &mut visited)?;
        if !declarations.is_empty() {
            seen.insert(exported.clone());
        }
        for mut snippet in declarations {
            if exported != member {
                snippet = rename_declared_function(snippet, &exported);
            }
            output.push('\n');
            output.push_str(&snippet);
        }
    }
    let mut visited_types = std::collections::BTreeSet::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) = item else {
            continue;
        };
        let Some(source) = export.src.value.as_str() else {
            continue;
        };
        let Some(target_path) = declaration_reexport_path(entry_path, source) else {
            continue;
        };
        for snippet in all_reexported_type_declarations(&target_path, &mut visited_types)? {
            output.push('\n');
            output.push_str(&snippet);
        }
        if export.type_only {
            continue;
        }
        let mut visited = std::collections::BTreeSet::new();
        let declarations = all_reexported_function_declarations(&target_path, &mut visited)?;
        let names = declarations
            .iter()
            .map(|(name, _)| name.clone())
            .filter(|name| !seen.contains(name))
            .collect::<std::collections::BTreeSet<_>>();
        for (name, snippet) in declarations {
            if names.contains(&name) {
                output.push('\n');
                output.push_str(&snippet);
            }
        }
        seen.extend(names);
    }
    // `export * as NAME from "SOURCE";` -- a *namespace* re-export,
    // structurally different from both the named (`export { X } from
    // "..."` above) and wildcard (`export * from "..."` just above) forms:
    // `NAME` isn't a single symbol, it's every one of `SOURCE`'s own
    // exports, reachable one level down (`z.coerce.number(...)`). Real-
    // world example: zod's own re-export barrel declares four of these --
    // `export * as core from "../core/index.cjs";`, `export * as locales
    // from "../locales/index.cjs";`, `export * as iso from "./iso.cjs";`,
    // `export * as coerce from "./coerce.cjs";` -- each exposing its own
    // handful of functions this way, one file below the entry point's own
    // (which reaches them only transitively, through its own plain
    // `export * from "./v4/classic/external.cjs";`) -- so this recurses
    // through plain wildcard re-exports the same way
    // `all_reexported_function_declarations` does, via
    // `collect_namespace_reexports`.
    //
    // Flattened the same way a class method or namespace-nested function
    // elsewhere in this codebase is: each function is renamed to a
    // synthesized, collision-free top-level name (`coerce` and the
    // package's own top-level functions can freely share a bare name --
    // zod's top-level `number()` and `coerce.number()` are unrelated
    // functions, and simply flattening both to bare `number` would silently
    // drop one), then a `declare namespace NAME { export { synthesized as
    // original, ... }; }` block records the mapping back to each function's
    // real member name -- see `thaw_bridge::nested_namespace_members`,
    // which parses this exact shape back out of the finished flattened
    // `.d.ts`.
    let mut namespace_visited = std::collections::BTreeSet::new();
    for (alias, target_path) in
        collect_namespace_reexports(entry_path, &module, &mut namespace_visited)?
    {
        let mut visited = std::collections::BTreeSet::new();
        let declarations = all_reexported_function_declarations(&target_path, &mut visited)?;
        if declarations.is_empty() {
            continue;
        }
        let mut members = Vec::new();
        for (name, snippet) in declarations {
            let synthetic = format!("__thaw_ns_{alias}_{name}");
            output.push('\n');
            output.push_str(&rename_declared_function(snippet, &synthetic));
            members.push(format!("{synthetic} as {name}"));
        }
        output.push_str(&format!(
            "\ndeclare namespace {alias} {{\n    export {{ {} }};\n}}\n",
            members.join(", ")
        ));
    }
    let mut self_referential_visited = std::collections::BTreeSet::new();
    for snippet in
        self_referential_namespace_alias_snippets(entry_path, &mut self_referential_visited)?
    {
        output.push('\n');
        output.push_str(&snippet);
    }
    Ok(output)
}

fn all_reexported_type_declarations(
    path: &Path,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem};
    use thaw_parser::common::{SourceMapper, Spanned};

    if !visited.insert(path.to_path_buf()) {
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
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export))
                if matches!(
                    &export.decl,
                    Decl::Class(_)
                        | Decl::TsInterface(_)
                        | Decl::TsTypeAlias(_)
                        | Decl::TsEnum(_)
                        | Decl::TsModule(_)
                ) =>
            {
                declarations.push(source_map.span_to_snippet(export.span()).map_err(|error| {
                    format!("failed to read type declaration in `{}`: {error:?}", path.display())
                })?);
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) => {
                if let Some(source) = export.src.value.as_str() {
                    if let Some(target) = declaration_reexport_path(path, source) {
                        declarations.extend(all_reexported_type_declarations(&target, visited)?);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(declarations)
}

/// A same-file `import * as X from "SOURCE"; export { X[, X as Y], ... };`
/// (and/or `export default X;`) -- the shape `thaw_bridge::self_
/// referential_namespace_aliases` recognizes in an already-flattened
/// `.d.ts`, but only at the top level of *one* file. Real npm packages
/// sometimes declare this in a file reached only *transitively* through
/// the entry point's own `export * from "...";`, not the entry point
/// itself -- real example: zod v3's `lib/index.d.ts` (`import * as z
/// from "./external"; export * from "./external"; export { z }; export
/// default z;`), one file below the package's own entry point
/// (`index.d.ts`, just `export * from "./lib";`) -- unlike zod v4, whose
/// entry point declares this alias directly, so it was already visible
/// to `self_referential_namespace_aliases` without this. Recurses through
/// wildcard re-exports the same way `collect_namespace_reexports` does,
/// returning the raw import/export snippet text verbatim for each
/// namespace-imported name that's re-exported this way -- its import
/// source doesn't need to resolve to anything real in the flattened
/// output (`self_referential_namespace_aliases` only checks the AST
/// shape, never follows the source path), so a caller can splice it into
/// the flattened output to make the alias visible to that same
/// downstream detector.
fn self_referential_namespace_alias_snippets(
    entry_path: &Path,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{
        Expr, ExportSpecifier, ImportSpecifier, ModuleDecl, ModuleExportName, ModuleItem,
    };
    use thaw_parser::common::{SourceMapper, Span, Spanned};

    if !visited.insert(entry_path.to_path_buf()) {
        return Ok(Vec::new());
    }
    let source = fs::read_to_string(entry_path).map_err(|error| {
        format!(
            "failed to read re-exported declarations `{}`: {error}",
            entry_path.display()
        )
    })?;
    let (module, source_map) = thaw_parser::parse_typescript_with_source_map(&source)?;

    let mut namespace_imports: std::collections::HashMap<String, (Span, Vec<Span>)> =
        std::collections::HashMap::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
            continue;
        };
        for specifier in &import.specifiers {
            if let ImportSpecifier::Namespace(namespace) = specifier {
                namespace_imports
                    .entry(namespace.local.sym.to_string())
                    .or_insert_with(|| (import.span(), Vec::new()));
            }
        }
    }
    let mut found = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export))
                if !export.type_only && export.src.is_none() =>
            {
                for specifier in &export.specifiers {
                    let ExportSpecifier::Named(named) = specifier else {
                        continue;
                    };
                    if named.is_type_only {
                        continue;
                    }
                    let ModuleExportName::Ident(ident) = &named.orig else {
                        continue;
                    };
                    if let Some((_, exports)) = namespace_imports.get_mut(ident.sym.as_str()) {
                        exports.push(export.span());
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(default_expr)) => {
                if let Expr::Ident(ident) = default_expr.expr.as_ref() {
                    if let Some((_, exports)) = namespace_imports.get_mut(ident.sym.as_str()) {
                        exports.push(default_expr.span());
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) if !export.type_only => {
                if let Some(source_path) = export.src.value.as_str() {
                    if let Some(target_path) = declaration_reexport_path(entry_path, source_path) {
                        found.extend(self_referential_namespace_alias_snippets(
                            &target_path,
                            visited,
                        )?);
                    }
                }
            }
            _ => {}
        }
    }
    for (name, (import_span, export_spans)) in &namespace_imports {
        if export_spans.is_empty() {
            continue;
        }
        let mut snippet = source_map
            .span_to_snippet(*import_span)
            .map_err(|error| format!("failed to read namespace import `{name}`: {error:?}"))?;
        for export_span in export_spans {
            snippet.push('\n');
            snippet.push_str(&source_map.span_to_snippet(*export_span).map_err(|error| {
                format!("failed to read self-referential export for `{name}`: {error:?}")
            })?);
        }
        found.push(snippet);
    }
    Ok(found)
}

/// Every `export * as NAME from "SOURCE";` reachable from `entry_path`'s
/// own `module` -- including one declared in a file only reached
/// transitively through a plain `export * from "...";` (real zod: the
/// namespace exports live one file below the package's own entry point).
/// Doesn't recurse through a *named* re-export (`export { x } from
/// "...";"`) -- that forwards one specific symbol, not a whole module's
/// worth of further bindings, and no package seen so far needs it to.
fn collect_namespace_reexports(
    entry_path: &Path,
    module: &thaw_parser::ast::Module,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<(String, PathBuf)>, String> {
    use thaw_parser::ast::{ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};

    if !visited.insert(entry_path.to_path_buf()) {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) if !export.type_only => {
                let Some(source) = export.src.as_ref().and_then(|s| s.value.as_str()) else {
                    continue;
                };
                for specifier in &export.specifiers {
                    let ExportSpecifier::Namespace(namespace) = specifier else {
                        continue;
                    };
                    let ModuleExportName::Ident(alias) = &namespace.name else {
                        continue;
                    };
                    if let Some(target_path) = declaration_reexport_path(entry_path, source) {
                        found.push((alias.sym.to_string(), target_path));
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) if !export.type_only => {
                let Some(source) = export.src.value.as_str() else {
                    continue;
                };
                let Some(target_path) = declaration_reexport_path(entry_path, source) else {
                    continue;
                };
                let target_source = fs::read_to_string(&target_path).map_err(|error| {
                    format!(
                        "failed to read re-exported declarations `{}`: {error}",
                        target_path.display()
                    )
                })?;
                let target_module = thaw_parser::parse_typescript(&target_source)?;
                found.extend(collect_namespace_reexports(
                    &target_path,
                    &target_module,
                    visited,
                )?);
            }
            _ => {}
        }
    }
    Ok(found)
}

/// Whether `name` is a full ECMAScript reserved word -- one that can
/// never be used as an ordinary binding/declaration name (a function
/// declared `declare function null(...)` is a syntax error, full stop,
/// not just an unusual identifier choice). Deliberately excludes a
/// "future reserved word" like `enum` and a merely-predefined global
/// like `undefined`, neither of which is actually reserved in this
/// position -- both parse fine as a function's own name, and real npm
/// packages rename an internal helper to exactly these words (zod's
/// `z.enum(...)`, `z.undefined()`) since they're the desired public API
/// name (see the `enum`/`catch`/`instanceof`/etc. rename-export handling
/// above, whose alias this guards).
fn is_ecmascript_keyword(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
    )
}

/// Given one `const`/`let`/`var` declarator, if its type annotation names
/// a callable shape -- a same-file interface with a call signature
/// (`SQLiteTableFn`-style), or a *direct* inline function type with no
/// interface involved at all (zod v3's `declare const objectType: <T
/// extends ZodRawShape>(shape: T, params?) => ZodObject<...>;`, one per
/// primitive) -- the declaration text needed to make its call signature
/// visible in a flattened `.d.ts`: just `decl_span`'s own snippet for a
/// direct function type (self-contained), or that snippet plus the
/// referenced interface's own snippet for a `TsTypeRef` (the call
/// signature lives there, not in the const itself). `None` if the type
/// doesn't name a callable shape at all (an ordinary data constant, or a
/// type this doesn't resolve).
///
/// `decl_span` is the caller's choice of "the const's own declaration
/// text" -- an `ExportDecl`'s span (to keep an `export` prefix) for an
/// exported const (`callable_const_declarations`, below), or a bare
/// `VarDecl`'s own span for one reached only through a later local
/// rename-export (`all_reexported_function_declarations`'s own
/// `local_declarations` loop, mirroring how it already handles a bare
/// `Decl::Fn`).
///
/// The interface lookup is restricted to a *same-file* interface, not one
/// reached through a further import: every real package seen so far
/// declares the factory-value const and its call-signature interface
/// side by side in one file, and resolving a cross-file interface would
/// need machinery parallel to `reexported_class_or_interface_declarations`
/// -- not worth building speculatively.
fn callable_const_declaration_snippet(
    module: &thaw_parser::ast::Module,
    source_map: &thaw_parser::common::SourceMap,
    decl_span: thaw_parser::common::Span,
    binding: &thaw_parser::ast::BindingIdent,
) -> Result<Option<String>, String> {
    use thaw_parser::ast::{
        Decl, ModuleDecl, ModuleItem, TsEntityName, TsFnOrConstructorType, TsType, TsTypeElement,
    };
    use thaw_parser::common::{SourceMapper, Spanned};

    let Some(annotation) = binding.type_ann.as_ref() else {
        return Ok(None);
    };
    match annotation.type_ann.as_ref() {
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(_)) => {
            let const_snippet = source_map.span_to_snippet(decl_span).map_err(|error| {
                format!(
                    "failed to read declaration for `{}`: {error:?}",
                    binding.id.sym
                )
            })?;
            Ok(Some(const_snippet))
        }
        TsType::TsTypeRef(ty_ref) => {
            let iface_name = match &ty_ref.type_name {
                TsEntityName::Ident(ident) => ident.sym.to_string(),
                TsEntityName::TsQualifiedName(qualified) => qualified.right.sym.to_string(),
            };
            let mut matched_iface_export = None;
            for other in &module.body {
                let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(iface_export)) = other else {
                    continue;
                };
                let Decl::TsInterface(iface) = &iface_export.decl else {
                    continue;
                };
                if iface.id.sym.as_ref() == iface_name.as_str() {
                    matched_iface_export = Some((iface_export, iface));
                    break;
                }
            }
            if let Some((iface_export, iface)) = matched_iface_export {
                if iface
                    .body
                    .body
                    .iter()
                    .any(|member| matches!(member, TsTypeElement::TsCallSignatureDecl(_)))
                {
                    let const_snippet = source_map.span_to_snippet(decl_span).map_err(|error| {
                        format!(
                            "failed to read declaration for `{}`: {error:?}",
                            binding.id.sym
                        )
                    })?;
                    let iface_snippet =
                        source_map.span_to_snippet(iface_export.span()).map_err(|error| {
                            format!("failed to read declaration for `{iface_name}`: {error:?}")
                        })?;
                    return Ok(Some(format!("{const_snippet}\n{iface_snippet}")));
                }
            }
            // A local (bare or exported) type alias, or an interface with
            // no call signature of its own, whose shape might still
            // resolve to something callable through a chain of further
            // local aliases/interfaces -- rather than resolving that
            // chain ourselves (thaw-bridge's own `classify_ts_type`/
            // `resolve_interfaces` already does, downstream, once the
            // text is present), carry the const's own snippet together
            // with *every* type alias and interface declared in this
            // same file (bare or exported). `.d.ts` type declarations are
            // erasable and side-effect-free, so including ones that turn
            // out unrelated to this particular const is harmless. Real
            // example: uuid's own `.d.ts` (via `@types/uuid`), where
            // `v1`'s declared type (`type v1 = v1Buffer & v1String;`) is
            // a *local, unexported* type alias chaining through several
            // more (`v1Buffer`, `v1String`, `V1Options`, ...). Conditioned
            // on the referenced name actually resolving to *something*
            // local at all -- a truly unknown/external type name still
            // returns `None`, unchanged from before.
            let resolves_locally = module.body.iter().any(|item| {
                let decl = match item {
                    ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(decl)) => decl,
                    ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => &export.decl,
                    _ => return false,
                };
                match decl {
                    Decl::TsInterface(iface) => iface.id.sym.as_ref() == iface_name.as_str(),
                    Decl::TsTypeAlias(alias) => alias.id.sym.as_ref() == iface_name.as_str(),
                    _ => false,
                }
            });
            if !resolves_locally {
                return Ok(None);
            }
            let mut combined = source_map.span_to_snippet(decl_span).map_err(|error| {
                format!(
                    "failed to read declaration for `{}`: {error:?}",
                    binding.id.sym
                )
            })?;
            for item in &module.body {
                let decl = match item {
                    ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(decl)) => decl,
                    ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => &export.decl,
                    _ => continue,
                };
                let span = match decl {
                    Decl::TsInterface(iface) => iface.span(),
                    Decl::TsTypeAlias(alias) => alias.span(),
                    _ => continue,
                };
                combined.push('\n');
                combined.push_str(&source_map.span_to_snippet(span).map_err(|error| {
                    format!("failed to read a local type declaration: {error:?}")
                })?);
            }
            Ok(Some(combined))
        }
        _ => Ok(None),
    }
}

/// Every top-level `export declare const NAME: T;` in `module` whose type
/// `T` names a callable shape -- see `callable_const_declaration_snippet`
/// for the two shapes recognized and real examples of each. The
/// `Decl::Var` counterpart to a plain exported `Decl::Fn`.
fn callable_const_declarations(
    module: &thaw_parser::ast::Module,
    source_map: &thaw_parser::common::SourceMap,
) -> Result<Vec<(String, String)>, String> {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Pat};
    use thaw_parser::common::Spanned;

    let mut declarations = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) = item else {
            continue;
        };
        let Decl::Var(var_decl) = &export.decl else {
            continue;
        };
        for declarator in &var_decl.decls {
            let Pat::Ident(binding) = &declarator.name else {
                continue;
            };
            if let Some(snippet) =
                callable_const_declaration_snippet(module, source_map, export.span(), binding)?
            {
                declarations.push((binding.id.sym.to_string(), snippet));
            }
        }
    }
    Ok(declarations)
}

/// Renames a single function declaration snippet's own declared name to
/// `exported`, wherever it actually is -- rather than assuming it matches
/// whatever name it was originally looked up under. That assumption holds
/// for a plain `export { original as exported }`, but not when `original`
/// was `"default"`: the snippet `reexported_function_declarations` returns
/// for a `default` lookup is resolved through `export default <ident>;` (or
/// an inline `export default function <ident>(...) {}`) and keeps that
/// real `<ident>`, which need not equal the literal string `"default"`.
fn rename_declared_function(snippet: String, exported: &str) -> String {
    // `"const "` covers a callable-const snippet (`callable_const_
    // declaration_snippet`, e.g. zod v3's `declare const objectType:
    // (...) => ...;`) -- for the interface-backed shape, whose snippet
    // concatenates the const's own declaration with its referenced
    // interface's, `"const "` always appears before that interface's own
    // `"interface "`, so this still renames the *const's* binding name
    // (the one actually being re-exported), not the interface's.
    let Some((keyword_index, keyword)) = ["function ", "class ", "interface ", "const "]
        .into_iter()
        .filter_map(|keyword| snippet.find(keyword).map(|index| (index, keyword)))
        .min_by_key(|(index, _)| *index)
    else {
        return snippet;
    };
    let name_start = keyword_index + keyword.len();
    let name_end = snippet[name_start..]
        .find(|character: char| !(character.is_alphanumeric() || character == '_' || character == '$'))
        .map(|offset| name_start + offset)
        .unwrap_or(snippet.len());
    if name_start == name_end || &snippet[name_start..name_end] == exported {
        return snippet;
    }
    let mut renamed = String::with_capacity(snippet.len());
    renamed.push_str(&snippet[..name_start]);
    renamed.push_str(exported);
    renamed.push_str(&snippet[name_end..]);
    renamed
}

fn all_reexported_function_declarations(
    path: &Path,
    visited: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<(String, String)>, String> {
    use thaw_parser::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};
    use thaw_parser::common::{SourceMapper, Spanned};

    if !visited.insert(path.to_path_buf()) {
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
        if let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(declaration)) = item {
            if let Decl::Fn(function) = &declaration.decl {
                declarations.push((
                    function.ident.sym.to_string(),
                    source_map
                        .span_to_snippet(declaration.span())
                        .map_err(|error| {
                            format!(
                                "failed to read declaration for `{}`: {error:?}",
                                function.ident.sym
                            )
                        })?,
                ));
            }
        }
    }
    // A *local*, non-exported `declare function` statement -- collected
    // up front (by name, supporting more than one overload) so a
    // same-file, no-`from`-clause rename-export below (`export { _enum
    // as enum };`) can resolve to one even though it was never itself
    // `export`-prefixed. Real example: zod's own `schemas.d.cts`
    // declares `declare function _enum(...)` (two overloads) bare, then
    // separately does `export { _enum as enum };` -- `enum` being a
    // reserved word, it can't be the function's own declared name.
    // Without this, `_enum` (and thus `z.enum(...)`) was silently
    // missing from the flattened `package.d.ts` entirely, not even
    // present under its own internal name.
    let mut local_declarations: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for item in &module.body {
        if let ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Fn(function))) = item {
            let snippet = source_map.span_to_snippet(function.span()).map_err(|error| {
                format!(
                    "failed to read declaration for `{}`: {error:?}",
                    function.ident.sym
                )
            })?;
            local_declarations
                .entry(function.ident.sym.to_string())
                .or_default()
                .push(snippet);
        }
    }
    // A *local*, non-exported callable `const` -- the `Decl::Var`
    // counterpart to the bare `Decl::Fn` loop just above, for the exact
    // same reason (a same-file rename-export below needs to resolve it).
    // Real example: zod v3's `lib/types.d.ts`, which declares `declare
    // const objectType: <T extends ZodRawShape>(shape: T, params?) =>
    // ZodObject<...>;` (and ~30 siblings) bare, then separately does
    // `export { ..., objectType as object, ... };` -- without this,
    // `object`/`string`/`number`/etc. (essentially all of zod v3's own
    // API) were silently missing from the flattened `package.d.ts`
    // entirely.
    for item in &module.body {
        let ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Var(var_decl))) = item else {
            continue;
        };
        for declarator in &var_decl.decls {
            let thaw_parser::ast::Pat::Ident(binding) = &declarator.name else {
                continue;
            };
            if let Some(snippet) =
                callable_const_declaration_snippet(&module, &source_map, var_decl.span(), binding)?
            {
                local_declarations
                    .entry(binding.id.sym.to_string())
                    .or_default()
                    .push(snippet);
            }
        }
    }
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            continue;
        };
        if export.type_only || export.src.is_some() {
            continue;
        }
        for specifier in &export.specifiers {
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
            let Some(snippets) = local_declarations.get(&original) else {
                continue;
            };
            for snippet in snippets {
                let snippet = if exported == original {
                    snippet.clone()
                } else {
                    rename_declared_function(snippet.clone(), &exported)
                };
                // The exported alias can be any identifier-like text at
                // all in TS export-specifier syntax, including a real
                // ECMAScript reserved word (`null`, `void`, `function`,
                // `catch`, `instanceof`) that can never actually be a
                // function's own declared name -- real example: zod's
                // own `export { _null as null };`. Renaming to one of
                // those would produce text no parser accepts (`declare
                // function null(...)`), and letting that reach the
                // flattened file poisons parsing for every other
                // declaration in it, not just this one -- confirmed via
                // a standalone repro of each word below. `enum` (a
                // future-reserved word, not a full keyword) and
                // `undefined` (not reserved at all, just a predefined
                // global) are deliberately not in this list -- both are
                // real, valid function names to this parser, and zod
                // uses both (`z.enum(...)`, `z.undefined()`).
                if is_ecmascript_keyword(&exported) {
                    continue;
                }
                declarations.push((exported.clone(), snippet));
            }
        }
    }
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) if !export.type_only => {
                if let Some(source) = export.src.value.as_str() {
                    if let Some(target) = declaration_reexport_path(path, source) {
                        declarations.extend(all_reexported_function_declarations(
                            &target, visited,
                        )?);
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export))
                if !export.type_only && export.src.is_some() =>
            {
                let source = export.src.as_ref().unwrap();
                let Some(source) = source.value.as_str() else {
                    continue;
                };
                let Some(target) = declaration_reexport_path(path, source) else {
                    continue;
                };
                for specifier in &export.specifiers {
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
                    let mut named_visited = std::collections::BTreeSet::new();
                    for mut snippet in reexported_function_declarations(
                        &target,
                        &original,
                        &mut named_visited,
                    )? {
                        if exported != original {
                            snippet = rename_declared_function(snippet, &exported);
                        }
                        declarations.push((exported.clone(), snippet));
                    }
                }
            }
            _ => {}
        }
    }
    declarations.extend(callable_const_declarations(&module, &source_map)?);
    Ok(declarations)
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
    if declarations.is_empty() {
        for (const_name, snippet) in callable_const_declarations(&module, &source_map)? {
            if const_name == name {
                declarations.push(snippet);
            }
        }
    }
    if declarations.is_empty() {
        let local_name = module.body.iter().find_map(|item| {
            let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
                return None;
            };
            if export.type_only || export.src.is_some() {
                return None;
            }
            export.specifiers.iter().find_map(|specifier| {
                let ExportSpecifier::Named(named) = specifier else {
                    return None;
                };
                let exported = named.exported.as_ref().unwrap_or(&named.orig);
                let ModuleExportName::Ident(exported) = exported else {
                    return None;
                };
                if exported.sym.as_ref() != name {
                    return None;
                }
                let ModuleExportName::Ident(original) = &named.orig else {
                    return None;
                };
                Some(original.sym.to_string())
            })
        });
        if let Some(local_name) = local_name {
            for item in &module.body {
                let ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Var(var_decl))) = item
                else {
                    continue;
                };
                for declarator in &var_decl.decls {
                    let thaw_parser::ast::Pat::Ident(binding) = &declarator.name else {
                        continue;
                    };
                    if binding.id.sym == local_name {
                        if let Some(snippet) = callable_const_declaration_snippet(
                            &module,
                            &source_map,
                            var_decl.span(),
                            binding,
                        )? {
                            declarations.push(snippet);
                        }
                    }
                }
            }
        }
    }
    if !declarations.is_empty() {
        return Ok(declarations);
    }
    // `export { default as v4 } from './v4.js'` (a barrel re-exporting
    // another file's *default* export under a name, the common shape for
    // packages that split one function per file, e.g. `uuid`) resolves to
    // `name == "default"` here, but a `.d.ts` file never declares a
    // function literally named `default` -- it either exports an inline
    // `export default function v4(...) {}` or (more commonly, so multiple
    // overloads can share one export) declares `function v4(...)` one or
    // more times as a plain top-level statement and separately writes
    // `export default v4;`. Resolve that indirection before giving up.
    if name == "default" {
        for item in &module.body {
            match item {
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(default_expr)) => {
                    let thaw_parser::ast::Expr::Ident(ident) = default_expr.expr.as_ref() else {
                        continue;
                    };
                    let resolved = ident.sym.as_ref();
                    for item in &module.body {
                        let ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Fn(function))) =
                            item
                        else {
                            continue;
                        };
                        if function.ident.sym.as_ref() == resolved {
                            declarations.push(source_map.span_to_snippet(function.span()).map_err(
                                |error| format!("failed to read declaration for `{resolved}`: {error:?}"),
                            )?);
                        }
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default_decl)) => {
                    if let thaw_parser::ast::DefaultDecl::Fn(fn_expr) = &default_decl.decl {
                        if fn_expr.ident.is_some() {
                            declarations.push(source_map.span_to_snippet(default_decl.span()).map_err(
                                |error| format!("failed to read default function declaration: {error:?}"),
                            )?);
                        }
                    }
                }
                _ => {}
            }
        }
        if !declarations.is_empty() {
            return Ok(declarations);
        }
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

/// Every plain (non-exported) `declare function X(...)` overload matching
/// whatever identifier `path`'s own `export = X;` names -- the shape
/// `import Name = require("./path")` (a TS import-equals declaration)
/// binds `Name` to, one file per function, real-world example: semver's
/// `functions/valid.d.ts` (`declare function valid(...): ...;\nexport =
/// valid;`). A plain `export = X;` module has no re-export chain of its
/// own to follow beyond this (unlike `reexported_function_declarations`,
/// which also handles `export { X } from another`), so this only ever
/// looks inside `path` itself.
fn export_assignment_function_declarations(path: &Path) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, Expr, ModuleDecl, ModuleItem, Stmt};
    use thaw_parser::common::{SourceMapper, Spanned};

    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read re-exported declarations `{}`: {error}",
            path.display()
        )
    })?;
    let (module, source_map) = thaw_parser::parse_typescript_with_source_map(&source)?;
    let Some(target) = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => match export.expr.as_ref() {
            Expr::Ident(ident) => Some(ident.sym.to_string()),
            _ => None,
        },
        _ => None,
    }) else {
        return Ok(Vec::new());
    };
    module
        .body
        .iter()
        .filter_map(|item| {
            let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) = item else {
                return None;
            };
            (function.ident.sym.as_ref() == target).then(|| {
                source_map
                    .span_to_snippet(function.span())
                    .map_err(|error| format!("failed to read declaration for `{target}`: {error:?}"))
            })
        })
        .collect()
}

/// The relative path each `import Name = require("./path")` (a TS
/// import-equals declaration) in `module` resolves to, keyed by `Name` --
/// used to follow a *local* `export { Name as exported };` (no `from`
/// clause: `Name` is already a value imported earlier in this same file,
/// not a re-export of another module's own export), real-world example:
/// semver's `index.d.ts`, which imports one function per file this way
/// and re-exports all of them together.
fn import_equals_targets(
    entry_path: &Path,
    module: &thaw_parser::ast::Module,
) -> std::collections::HashMap<String, PathBuf> {
    use thaw_parser::ast::{ModuleDecl, ModuleItem, TsModuleRef};

    module
        .body
        .iter()
        .filter_map(|item| {
            let ModuleItem::ModuleDecl(ModuleDecl::TsImportEquals(import)) = item else {
                return None;
            };
            let TsModuleRef::TsExternalModuleRef(reference) = &import.module_ref else {
                return None;
            };
            let source = reference.expr.value.as_str()?;
            let target_path = declaration_reexport_path(entry_path, source)?;
            Some((import.id.sym.to_string(), target_path))
        })
        .collect()
}

/// The `(target_path, name_in_target)` each *ordinary* ES import
/// (`import { X } from "./y"`, `import { X as Local } from "./y"`, or a
/// default import `import Local from "./y"`) in `module` resolves to,
/// keyed by the local binding name -- the ES-module counterpart of
/// `import_equals_targets` above, used the same way: to follow a local
/// `export { Local };` (no `from` clause) back to whatever `Local` was
/// actually bound to. Real-world example: hono's `index.d.ts`, which
/// does `import { Hono } from './hono'; export { Hono };` -- a class,
/// not a function, split into its own file and re-exported under the
/// same local name. Only `.`-relative specifiers resolve (matching
/// `declaration_reexport_path`); a bare-specifier import (a real
/// dependency, not an internal file split) is left alone.
fn named_import_targets(
    entry_path: &Path,
    module: &thaw_parser::ast::Module,
) -> std::collections::HashMap<String, (PathBuf, String)> {
    use thaw_parser::ast::{ImportSpecifier, ModuleDecl, ModuleItem};

    let mut targets = std::collections::HashMap::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
            continue;
        };
        if import.type_only {
            continue;
        }
        let Some(source) = import.src.value.as_str() else {
            continue;
        };
        let Some(target_path) = declaration_reexport_path(entry_path, source) else {
            continue;
        };
        for specifier in &import.specifiers {
            match specifier {
                ImportSpecifier::Named(named) if !named.is_type_only => {
                    let imported_name = named
                        .imported
                        .as_ref()
                        .and_then(|name| match name {
                            thaw_parser::ast::ModuleExportName::Ident(id) => {
                                Some(id.sym.to_string())
                            }
                            thaw_parser::ast::ModuleExportName::Str(_) => None,
                        })
                        .unwrap_or_else(|| named.local.sym.to_string());
                    targets.insert(
                        named.local.sym.to_string(),
                        (target_path.clone(), imported_name),
                    );
                }
                ImportSpecifier::Default(default) => {
                    targets.insert(
                        default.local.sym.to_string(),
                        (target_path.clone(), "default".to_string()),
                    );
                }
                _ => {}
            }
        }
    }
    targets
}

/// Same shape as `reexported_function_declarations`, but for a class or
/// interface declared directly in `path` under `name` (the counterpart
/// to that function's `Decl::Fn` handling for `Decl::Class`/
/// `Decl::TsInterface`) -- doesn't follow further `export ... from`
/// re-export chains itself, since `named_import_targets` only ever
/// points at the file a name was *imported* from, which for every
/// package seen so far declares the class/interface directly rather
/// than re-exporting it yet again.
fn reexported_class_or_interface_declarations(
    path: &Path,
    name: &str,
) -> Result<Vec<String>, String> {
    let mut visited = std::collections::BTreeSet::new();
    reexported_class_or_interface_declarations_inner(path, name, &mut visited)
}

fn reexported_class_or_interface_declarations_inner(
    path: &Path,
    name: &str,
    visited: &mut std::collections::BTreeSet<(PathBuf, String)>,
) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Stmt};
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
    let mut local_name = name.to_string();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            continue;
        };
        if export.src.is_some() {
            continue;
        }
        for specifier in &export.specifiers {
            let ExportSpecifier::Named(named) = specifier else {
                continue;
            };
            let exported = named.exported.as_ref().unwrap_or(&named.orig);
            let (ModuleExportName::Ident(exported), ModuleExportName::Ident(original)) =
                (exported, &named.orig)
            else {
                continue;
            };
            if exported.sym == name {
                local_name = original.sym.to_string();
            }
        }
    }

    let mut declarations = Vec::new();
    let mut superclass = None;
    for item in &module.body {
        let declaration = match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(declaration)) => Some(&declaration.decl),
            ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
            _ => None,
        };
        let Some(declaration) = declaration else {
            continue;
        };
        let matches = match declaration {
            Decl::Class(class) => {
                if class.ident.sym == local_name {
                    superclass = class.class.super_class.as_deref().and_then(|expr| match expr {
                        thaw_parser::ast::Expr::Ident(ident) => Some(ident.sym.to_string()),
                        _ => None,
                    });
                    true
                } else {
                    false
                }
            }
            Decl::TsInterface(interface) => interface.id.sym == local_name,
            _ => false,
        };
        if matches {
            let mut snippet =
                source_map
                    .span_to_snippet(declaration.span())
                    .map_err(|error| format!("failed to read declaration for `{name}`: {error:?}"))?;
            if local_name != name {
                snippet = rename_declared_function(snippet, name);
            }
            if !snippet.trim_start().starts_with("export ") {
                snippet = format!("export {snippet}");
            }
            declarations.push(snippet);
        }
    }
    if let Some(superclass) = superclass {
        if let Some((target_path, target_name)) = named_import_targets(path, &module).get(&superclass)
        {
            declarations.extend(reexported_class_or_interface_declarations_inner(
                target_path,
                target_name,
                visited,
            )?);
        }
    }
    Ok(declarations)
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

/// The name of the *type* that `export = X;`'s `X` is declared with in
/// `module` -- e.g. mime's `export = mime;` alongside `declare const
/// mime: Mime;` resolves to `"Mime"`. Mirrors
/// `export_assignment_function_declarations`'s own `export = X` lookup,
/// but reads the type annotation of a `declare const` binding rather
/// than a function body: `X` here names a *value* whose class lives
/// entirely in a different file (see `import_equals_targets`), not a
/// function declared locally.
fn export_assignment_value_type_name(module: &thaw_parser::ast::Module) -> Option<String> {
    use thaw_parser::ast::{Decl, Expr, ModuleDecl, ModuleItem, Pat, Stmt, TsEntityName, TsType};

    let target = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => {
            match export.expr.as_ref() {
                Expr::Ident(ident) => Some(ident.sym.to_string()),
                _ => None,
            }
        }
        _ => None,
    })?;
    module.body.iter().find_map(|item| {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Var(var_decl))) = item else {
            return None;
        };
        var_decl.decls.iter().find_map(|declarator| {
            let Pat::Ident(binding) = &declarator.name else {
                return None;
            };
            if binding.id.sym.as_str() != target {
                return None;
            }
            let annotation = binding.type_ann.as_ref()?;
            let TsType::TsTypeRef(ty_ref) = annotation.type_ann.as_ref() else {
                return None;
            };
            match &ty_ref.type_name {
                TsEntityName::Ident(ident) => Some(ident.sym.to_string()),
                TsEntityName::TsQualifiedName(_) => None,
            }
        })
    })
}

/// Inlines the file backing a `declare const x: Name;` value's *type*
/// (`Name`) when `Name` isn't declared anywhere in this same file but
/// resolves through this file's own `import Name = require("./path")`
/// (see `import_equals_targets`) -- real-world example: mime's
/// `import Mime = require("./Mime"); ... declare const mime: Mime;
/// export = mime;`, `Mime` itself living in `./Mime.d.ts`, a *type*
/// reference across files rather than the *value* re-export
/// `import_equals_targets`'s other callers already follow. Conceptually
/// the same "read the referenced file and splice its declarations in"
/// pattern as `inline_triple_slash_references`, just reached through an
/// import-equals value binding's type annotation instead of a `///
/// <reference path="..." />` comment.
///
/// `Mime.d.ts` itself is a plain ES module (`export default class Mime
/// {...}`, with its own `import { TypeMap } from "./index"` back to the
/// entry file) rather than an ambient declaration -- only the `export
/// default class Name` prefix is rewritten to `declare class Name`
/// (the DefinitelyTyped convention for a single-class module); anything
/// else in the file, notably that `import`, is inlined unchanged since
/// thaw-bridge's own `.d.ts` processing already ignores any top-level
/// item it doesn't specifically look for.
fn inline_import_equals_value_type(
    module: &thaw_parser::ast::Module,
    import_equals_targets: &std::collections::HashMap<String, PathBuf>,
) -> Result<String, String> {
    let Some(type_name) = export_assignment_value_type_name(module) else {
        return Ok(String::new());
    };
    let Some(target_path) = import_equals_targets.get(&type_name) else {
        return Ok(String::new());
    };
    let source = fs::read_to_string(target_path).map_err(|error| {
        format!(
            "failed to read `{}`'s imported type `{type_name}`: {error}",
            target_path.display()
        )
    })?;
    let source = source.replacen(
        &format!("export default class {type_name}"),
        &format!("declare class {type_name}"),
        1,
    );
    Ok(format!("\n{source}\n"))
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
        if conditions == ["types"]
            && ![".d.ts", ".d.cts", ".d.mts"]
                .iter()
                .any(|extension| path.ends_with(extension))
        {
            return None;
        }
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
        for condition in ["require", "import", "default"] {
            if let Some(path) = object
                .get(condition)
                .and_then(|value| select_export_condition(value, conditions))
            {
                return Some(path);
            }
        }
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
    // Same flattening a same-package `.d.ts` already gets (barrel
    // re-exports inlined, `/// <reference path="...">` files -- common in
    // an older, multi-file DefinitelyTyped package like `@types/lodash`
    // -- inlined too): this file is about to be the *only* one thaw-registry
    // keeps around for `package`, so anything it reaches now needs
    // following before that scratch checkout disappears.
    let source = dts_source_with_reexported_functions(&abs, &source)?;

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
