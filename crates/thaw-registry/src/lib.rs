//! V1 package registry: a plain local directory, one subdirectory per
//! package (see docs/design/registry.md):
//!
//! ```text
//! <registry-dir>/<package>/
//!   package.d.ts   (required)  -- fed to thaw-bridge's parse_dts/classify
//!   native.a       (optional)  -- prebuilt static lib providing the
//!                                  package's Fast path native symbols;
//!                                  auto-linked by thaw-cli (replaces a
//!                                  manual `--link`)
//!   native.node    (optional)  -- matching bundled N-API prebuild selected
//!                                  by `add`; loaded by thaw-napi
//!   native-addon.json (optional) -- source path, target tuple, and SHA-256
//!                                  for `native.node`
//!   bundle.js      (optional)  -- real JS implementation backing the
//!                                  package's Fallback functions; its
//!                                  source is fed to thaw-bridge's
//!                                  `generate_module_init` so it's loaded
//!                                  automatically at program startup
//!                                  (replaces a manual `loadScript` call)
//!   subpaths/<path>/package.d.ts -- declarations for an exact `exports`
//!                                  subpath such as `./feature`
//!   subpaths/<path>/bundle.js    -- independently bundled runtime entry
//!                                  for that subpath
//!   version.txt    (optional)  -- the exact version `add` resolved and
//!                                  fetched for the package itself (see
//!                                  `add`'s doc comment); purely
//!                                  informational
//!   lock.json      (optional)  -- every *other* real npm package folded
//!                                  into `bundle.js` (transitive same-
//!                                  registry-install dependencies), each
//!                                  mapped to the version `npm` actually
//!                                  resolved it to; written only when
//!                                  there's at least one (a single-file
//!                                  package with no dependencies has
//!                                  nothing to record here beyond
//!                                  `version.txt`'s own package). Purely
//!                                  informational, same as `version.txt`.
//! ```
//!
//! Still no source build step for native code, and no real dependency-graph
//! *resolution* (no semver range solving of our own -- `npm install`
//! already did that once, for one `add` call, and `lock.json` just
//! records what it picked) -- `add` resolves and records versions for
//! exactly the packages one `npm install <package>@<spec>` call actually
//! pulled in, independently each time `add` runs. This crate only
//! resolves a package name to the files already sitting on disk (plus,
//! now, the versions `add` recorded there). The native-lib build pipeline
//! is still exactly what the project's design doc calls "the actual
//! differentiator" left undone; this is a placeholder for the local half
//! of it, real enough to remove the remaining manual
//! `--bridge`/`--link`/`loadScript` steps for a package that's already
//! been fetched/built by some other means. `add` does select already-bundled
//! `.node` prebuilds; it never runs package install scripts or `node-gyp`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// A package resolved from a local registry directory. `native_lib`,
/// `native_addon`, and `bundle_js` are independent runtime backends.
#[derive(Debug)]
pub struct ResolvedPackage {
    pub name: String,
    pub dts_source: String,
    pub native_lib: Option<PathBuf>,
    /// A synchronous Node-API addon loaded through thaw-napi. Kept separate
    /// from `native_lib` because `.node` uses N-API handles, not the Fast-path
    /// C layout.
    pub native_addon: Option<PathBuf>,
    pub bundle_js: Option<String>,
    /// The version `add` recorded in `version.txt`, if this package went
    /// through `add` (rather than hand-curation, or an `add` run before
    /// this field existed) -- see `AddedPackage::resolved_version`.
    pub version: Option<String>,
    /// Every *other* real npm package `add` folded into `bundle.js`,
    /// mapped to its resolved version, read back from `lock.json` -- see
    /// `AddedPackage::dependency_versions`. `None` if there's no
    /// `lock.json` (a single-file package with no dependencies never gets
    /// one written; nor does a hand-curated or pre-`lock.json` package).
    pub dependency_versions: Option<BTreeMap<String, String>>,
}

/// Resolves `name` against `registry_dir/<name>/`. Fails only if
/// `package.d.ts` is missing or unreadable -- that's the one required
/// file, since a package with neither a native lib nor a bundle would
/// have nothing for thaw-bridge's shim to call.
pub fn resolve(registry_dir: &Path, name: &str) -> Result<ResolvedPackage, String> {
    let (package, subpath) = split_bare_spec(name);
    let mut dir = registry_dir.join(package);
    if let Some(subpath) = subpath {
        validate_export_subpath(subpath)?;
        dir = dir.join("subpaths").join(subpath);
    }

    let dts_path = dir.join("package.d.ts");
    let dts_source = fs::read_to_string(&dts_path).map_err(|e| {
        format!(
            "registry package `{name}`: failed to read `{}`: {e}",
            dts_path.display()
        )
    })?;

    let native_lib_path = dir.join("native.a");
    let native_lib = native_lib_path.is_file().then_some(native_lib_path);
    let native_addon_path = dir.join("native.node");
    let native_addon = native_addon_path.is_file().then_some(native_addon_path);

    let bundle_js_path = dir.join("bundle.js");
    let bundle_js = if bundle_js_path.is_file() {
        Some(fs::read_to_string(&bundle_js_path).map_err(|e| {
            format!(
                "registry package `{name}`: failed to read `{}`: {e}",
                bundle_js_path.display()
            )
        })?)
    } else {
        None
    };

    let version = fs::read_to_string(dir.join("version.txt"))
        .ok()
        .map(|s| s.trim().to_string());

    let dependency_versions = fs::read_to_string(dir.join("lock.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<BTreeMap<String, String>>(&s).ok());

    Ok(ResolvedPackage {
        name: name.to_string(),
        dts_source,
        native_lib,
        native_addon,
        bundle_js,
        version,
        dependency_versions,
    })
}

/// Resolves the deliberately small built-in surface exposed to user imports.
/// The implementation is the same CommonJS polyfill already used while
/// bundling npm dependencies, paired with a Thaw-friendly declaration whose
/// fallback functions accept the existing JSON positional-argument array.
pub fn resolve_builtin(specifier: &str) -> Result<ResolvedPackage, String> {
    let name = specifier.strip_prefix("node:").unwrap_or(specifier);
    let dts_source = match name {
        "util" => "export declare function inspect(argsArray: any): any;\n",
        "path" => {
            "export declare function resolve(argsArray: any): any;\nexport declare function join(argsArray: any): any;\nexport declare function dirname(argsArray: any): any;\nexport declare function basename(argsArray: any): any;\nexport declare function normalize(argsArray: any): any;\n"
        }
        "process" => "export declare function cwd(argsArray: any): any;\n",
        "buffer" => "export declare function byteLength(argsArray: any): any;\n",
        "fs" => {
            "export declare function existsSync(path: string): boolean;\nexport declare function readFileSync(path: string, encoding: string): string;\nexport declare function writeFileSync(path: string, data: string): boolean;\nexport declare function mkdirSync(path: string): boolean;\n"
        }
        "http" => {
            "export declare function serveOnce(port: number, body: string): string;\nexport declare function serveOnceWith(port: number, callback: (target: string) => string): string;\n"
        }
        _ => return Err(format!("unsupported Node built-in module `{specifier}`")),
    };
    let source = builtin_module_source(name)
        .ok_or_else(|| format!("unsupported Node built-in module `{specifier}`"))?;
    Ok(ResolvedPackage {
        name: format!("node:{name}"),
        dts_source: dts_source.to_string(),
        native_lib: None,
        native_addon: None,
        bundle_js: Some(source.to_string()),
        version: None,
        dependency_versions: None,
    })
}

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
    relative_path: String,
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
    Ok(Some(SelectedPrebuild {
        path,
        relative_path,
        platform: platform.into(),
        arch: arch.into(),
        libc: libc.into(),
    }))
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
/// carry no runtime code. An ESM-only package and a native addon (this
/// never produces a `native.a`) are still out of scope; see the design
/// doc's closing section.
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
        bundle_commonjs_package(&node_modules_dir, name, &package_dir, main_field)?;

    let (dts_relative_path, dts_source) = match find_own_dts(&manifest, &package_dir) {
        Some((rel, abs)) => {
            let source =
                fs::read_to_string(&abs).map_err(|e| format!("failed to read `{rel}`: {e}"))?;
            (rel, source)
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
    let (native_addon, native_diagnostic) = match select_prebuilt_addon(&package_dir) {
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
                source: selected.relative_path,
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
            bundle_commonjs_package(&node_modules_dir, name, &package_dir, &export.runtime_entry)?;
        dependency_versions.extend(subpath_dependencies);
        let types_path = package_dir.join(&export.types_entry);
        let subpath_dts = fs::read_to_string(&types_path).map_err(|error| {
            format!(
                "failed to read package export `./{subpath}` types `{}`: {error}",
                types_path.display()
            )
        })?;
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
    let candidates = [
        path.to_string(),
        format!("{trimmed}.js"),
        // Real ESM packages commonly use an explicit `.mjs` extension
        // (sometimes alongside a separate `.cjs` build) rather than
        // relying on `package.json`'s `"type": "module"`.
        format!("{trimmed}.mjs"),
        format!("{trimmed}/index.js"),
        format!("{trimmed}/index.mjs"),
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
fn bundle_commonjs_package(
    node_modules_dir: &Path,
    root_package: &str,
    root_package_dir: &Path,
    main_relative: &str,
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
    record_package_version(&mut dependency_versions, root_package, root_package_dir);

    while let Some((key, abs_path, pkg_name, pkg_dir)) = worklist.pop() {
        let source = fs::read_to_string(&abs_path)
            .map_err(|e| format!("failed to read `{key}` while bundling: {e}"))?;
        // A no-op for a file that's already CommonJS (or doesn't parse as
        // JS at all -- left completely untouched either way, so this can
        // never make an already-working file worse).
        let source = rewrite_esm_to_commonjs(&source).unwrap_or(source);

        let relative_in_pkg = key
            .strip_prefix(&format!("{pkg_name}/"))
            .unwrap_or(key.as_str());
        let requiring_dir = Path::new(relative_in_pkg).parent().unwrap_or(Path::new(""));

        let mut requires = Vec::new();

        for spec in find_relative_require_specs(&source) {
            let combined = if requiring_dir.as_os_str().is_empty() {
                spec.clone()
            } else {
                format!("{}/{spec}", requiring_dir.display())
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
                let resolved_key = format!("{pkg_name}/{resolved_relative}");
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

        for spec in find_bare_require_specs(&source) {
            if let Some((dep_name, dep_relative, dep_abs, dep_dir)) =
                resolve_bare_require(node_modules_dir, &spec)
            {
                let dep_key = format!("{dep_name}/{dep_relative}");
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
            if let Some(builtin_source) = builtin_module_source(&spec) {
                let builtin_key = format!("node:{spec}");
                requires.push((spec, builtin_key.clone()));
                if !visited.contains(&builtin_key) {
                    visited.push(builtin_key.clone());
                    modules.push(BundledModule {
                        key: builtin_key,
                        source: builtin_source.to_string(),
                        requires: Vec::new(),
                    });
                }
            }
            // Otherwise left unresolved -- falls through to the runtime
            // external-require stub, same as always.
        }

        modules.push(BundledModule {
            key,
            source,
            requires,
        });
    }

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

/// The shared text scan behind `find_relative_require_specs`/
/// `find_bare_require_specs`: finds every `require('x')`/`require("y")`
/// call spec in raw JS text, deliberately just a text scan rather than
/// real parsing (this project has no general JS/CommonJS parser, only
/// thaw-parser's TS-oriented one for `.d.ts`/`.ts`). A dynamic
/// `require(someVariable)` or a template-literal spec simply won't be
/// found, which just means that one call falls through to whatever
/// runtime `require` is in scope -- no worse than every `require` call
/// failing outright, which is what happened before any of this existed.
fn find_require_specs(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut specs = Vec::new();
    let mut i = 0;
    while let Some(offset) = source[i..].find("require") {
        let start = i + offset + "require".len();
        i = start;

        let mut j = start;
        while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
            j += 1;
        }
        if bytes.get(j) != Some(&b'(') {
            continue;
        }
        j += 1;
        while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
            j += 1;
        }
        let Some(&quote) = bytes.get(j).filter(|b| **b == b'\'' || **b == b'"') else {
            continue;
        };
        j += 1;
        let content_start = j;
        while bytes.get(j).is_some_and(|b| *b != quote) {
            j += 1;
        }
        if bytes.get(j) != Some(&quote) {
            continue;
        }

        specs.push(source[content_start..j].to_string());
    }
    specs
}

/// Rewrites ESM (`import`/`export`) syntax to the CommonJS shape the rest
/// of this bundler's require-graph resolution already understands:
/// `find_relative_require_specs`/`find_bare_require_specs`'s text scan
/// only ever looks for ordinary `require(...)` calls, so as long as this
/// produces those, nothing else in the pipeline needs to know ESM was
/// ever involved -- deep imports, builtins, and cross-package resolution
/// all keep working unmodified on the rewritten text.
///
/// Returns `None` (caller keeps the original source untouched) if the
/// file doesn't parse as JS at all, or parses but uses no `import`/
/// `export` syntax -- this only ever *adds* a transformation on top of
/// already-working CommonJS, never risks corrupting it.
///
/// A statement this doesn't need to touch is copied out **verbatim** via
/// its original source span (`SourceMap::span_to_snippet`), not
/// re-printed from the AST -- this project carries no general JS code
/// generator, and byte-for-byte preservation of untouched code avoids
/// ever needing one. Only the `import`/`export` declarations themselves
/// are replaced with synthesized `require`/`exports.x = ...` statements.
/// A destructuring `export const { a, b } = obj;` and a re-exported
/// string-literal name (`export { x as "weird name" }`, a rare ES2022
/// form) fall outside what's extracted -- silently contribute nothing to
/// `exports`, rather than aborting the whole rewrite.
fn rewrite_esm_to_commonjs(source: &str) -> Option<String> {
    use thaw_parser::ast::{
        Decl, DefaultDecl, ExportSpecifier, ImportSpecifier, ModuleDecl, ModuleExportName,
        ModuleItem, Pat,
    };
    use thaw_parser::common::{SourceMapper, Spanned};

    let (module, cm) = thaw_parser::parse_javascript_with_source_map(source).ok()?;
    let has_esm_syntax = module
        .body
        .iter()
        .any(|item| matches!(item, ModuleItem::ModuleDecl(_)));
    if !has_esm_syntax {
        return None;
    }

    let snippet = |span: thaw_parser::common::Span| cm.span_to_snippet(span).ok();
    let export_name = |name: &ModuleExportName| match name {
        ModuleExportName::Ident(id) => id.sym.to_string(),
        // `Wtf8Atom` (arbitrary-string export names, a rare ES2022 form)
        // has no `Display`; lossily converting to UTF-8 is fine here --
        // this text only ever ends up embedded in generated JS source.
        ModuleExportName::Str(s) => s.value.to_string_lossy().into_owned(),
    };
    let names_declared_by = |decl: &Decl| -> Vec<String> {
        match decl {
            Decl::Fn(f) => vec![f.ident.sym.to_string()],
            Decl::Class(c) => vec![c.ident.sym.to_string()],
            Decl::Var(v) => v
                .decls
                .iter()
                .filter_map(|d| match &d.name {
                    Pat::Ident(id) => Some(id.id.sym.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    };

    let mut prologue = String::from("module.exports.__esModule = true;\n");
    let mut rest = String::new();
    let mut synthetic_count = 0usize;

    for item in &module.body {
        match item {
            ModuleItem::Stmt(stmt) => {
                if let Some(text) = snippet(stmt.span()) {
                    rest.push_str(&text);
                    rest.push('\n');
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                let var_name = format!("__thaw_esm_import_{synthetic_count}");
                synthetic_count += 1;
                let spec = import.src.value.to_string_lossy();
                prologue.push_str(&format!(
                    "var {var_name} = require({});\n",
                    js_string_literal(&spec)
                ));
                for specifier in &import.specifiers {
                    match specifier {
                        ImportSpecifier::Default(d) => {
                            let local = d.local.sym.to_string();
                            prologue.push_str(&format!(
                                "var {local} = ({var_name} && {var_name}.__esModule) ? {var_name}.default : {var_name};\n"
                            ));
                        }
                        ImportSpecifier::Namespace(n) => {
                            prologue.push_str(&format!("var {} = {var_name};\n", n.local.sym));
                        }
                        ImportSpecifier::Named(n) => {
                            let local = n.local.sym.to_string();
                            let imported = n
                                .imported
                                .as_ref()
                                .map(&export_name)
                                .unwrap_or_else(|| local.clone());
                            prologue.push_str(&format!("var {local} = {var_name}.{imported};\n"));
                        }
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export_decl)) => {
                if let Some(text) = snippet(export_decl.decl.span()) {
                    rest.push_str(&text);
                    rest.push('\n');
                }
                for name in names_declared_by(&export_decl.decl) {
                    rest.push_str(&format!("exports.{name} = {name};\n"));
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default_decl)) => {
                let text = match &default_decl.decl {
                    DefaultDecl::Fn(f) => snippet(f.span()),
                    DefaultDecl::Class(c) => snippet(c.span()),
                    DefaultDecl::TsInterfaceDecl(_) => None,
                };
                if let Some(text) = text {
                    rest.push_str(&format!("module.exports.default = {text};\n"));
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(default_expr)) => {
                if let Some(text) = snippet(default_expr.expr.span()) {
                    rest.push_str(&format!("module.exports.default = {text};\n"));
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(named)) => match &named.src {
                Some(src) => {
                    let var_name = format!("__thaw_esm_reexport_{synthetic_count}");
                    synthetic_count += 1;
                    let spec = src.value.to_string_lossy();
                    prologue.push_str(&format!(
                        "var {var_name} = require({});\n",
                        js_string_literal(&spec)
                    ));
                    for spec in &named.specifiers {
                        if let ExportSpecifier::Named(n) = spec {
                            let orig = export_name(&n.orig);
                            let exported = n
                                .exported
                                .as_ref()
                                .map(&export_name)
                                .unwrap_or_else(|| orig.clone());
                            rest.push_str(&format!("exports.{exported} = {var_name}.{orig};\n"));
                        }
                        // `export * as ns from './y'`/`export v from './y'`:
                        // rare re-export forms, best-effort skipped.
                    }
                }
                None => {
                    for spec in &named.specifiers {
                        if let ExportSpecifier::Named(n) = spec {
                            let orig = export_name(&n.orig);
                            let exported = n
                                .exported
                                .as_ref()
                                .map(&export_name)
                                .unwrap_or_else(|| orig.clone());
                            rest.push_str(&format!("exports.{exported} = {orig};\n"));
                        }
                    }
                }
            },
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export_all)) => {
                let var_name = format!("__thaw_esm_reexport_all_{synthetic_count}");
                synthetic_count += 1;
                let spec = export_all.src.value.to_string_lossy();
                prologue.push_str(&format!(
                    "var {var_name} = require({});\n",
                    js_string_literal(&spec)
                ));
                rest.push_str(&format!(
                    "for (var __thaw_esm_key in {var_name}) {{ exports[__thaw_esm_key] = {var_name}[__thaw_esm_key]; }}\n"
                ));
            }
            // `import foo = require(...)`/`export = foo`/`export as
            // namespace`: TS-only forms that shouldn't appear in real
            // runtime `.js` files; skip gracefully rather than crashing
            // if one somehow does.
            ModuleItem::ModuleDecl(_) => {}
        }
    }

    Some(format!("{prologue}{rest}"))
}

/// Specs starting with `./` or `../` -- same-package relative requires.
fn find_relative_require_specs(source: &str) -> Vec<String> {
    find_require_specs(source)
        .into_iter()
        .filter(|spec| spec.starts_with("./") || spec.starts_with("../"))
        .collect()
}

/// Everything else -- another npm package (or a Node core builtin,
/// which just won't resolve under `node_modules_dir` and is left alone).
fn find_bare_require_specs(source: &str) -> Vec<String> {
    find_require_specs(source)
        .into_iter()
        .filter(|spec| !(spec.starts_with("./") || spec.starts_with("../")))
        .collect()
}

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

/// Resolves a bare require spec (found under `node_modules_dir`) to the
/// file it actually points to. With no subpath, that's the target
/// package's own `main` field (or the `index.js` default); with a "deep
/// import" subpath (e.g. `require('es-errors/type')`, found necessary by
/// `qs`'s own transitive dependency chain), Node resolves the subpath
/// directly against the package root -- the target package's `main`
/// field is irrelevant in that case. Returns `(package name, path
/// resolved relative to the package root, its absolute path, the
/// package's own directory)`, or `None` if it can't be resolved (not
/// installed under `node_modules_dir`, or -- rare -- the subpath itself
/// doesn't exist) -- left for the runtime external-require stub to
/// report, same as any other unresolvable require.
/// A tiny, hand-maintained polyfill for a Node.js core builtin module --
/// *not* a real re-implementation of Node's standard library, just
/// enough surface for whatever a real npm package's dependency chain
/// has actually been found to touch unconditionally at load time, added
/// one module (and one function) at a time the same way every other gap
/// in this file was: hit a real error against a real package, fix
/// exactly that. A Node builtin has no `package.json`/`node_modules`
/// entry at all, so `resolve_bare_require` correctly never finds it;
/// this is the fallback checked only after that lookup fails.
///
/// `util`: found necessary by `qs`'s real dependency chain --
/// `object-inspect` (pulled in via `side-channel`) does `require('util')`
/// unconditionally at the top of `util.inspect.js`, only to read
/// `.inspect`/`.inspect.custom` off the result (as a fallback/symbol
/// source, not for `util.inspect`'s actual pretty-printing behavior,
/// which `object-inspect` itself reimplements) -- a real
/// `util.inspect`-quality implementation is unnecessary for that.
fn builtin_module_source(name: &str) -> Option<&'static str> {
    match name {
        "util" => Some(
            "function inspect(value) { return String(value); }\n\
             inspect.custom = Symbol.for('nodejs.util.inspect.custom');\n\
             module.exports = { inspect: inspect };\n",
        ),
        // Found necessary by a real ESM package (`has-flag`): `import
        // process from 'process'` -- Node exposes `process` as both a
        // global and a core module; this is the module half. `.default`
        // is set too so the ESM-interop convention `rewrite_esm_to_commonjs`
        // generates for a default import (`.__esModule ? .default : ...`)
        // finds the same object either way. Only the couple of fields a
        // real package has actually been found to read.
        "process" => Some(
            "var __thaw_process = { argv: [], env: {}, platform: 'linux', version: '', versions: {}, cwd: function() { return '/'; }, nextTick: function(fn) { fn(); } };\n\
             module.exports = __thaw_process;\n\
             module.exports.default = __thaw_process;\n\
             module.exports.__esModule = true;\n",
        ),
        // Found necessary chasing a real native addon's load path
        // (`bcrypt`, `utf-8-validate`): both depend on `node-gyp-build`,
        // which unconditionally does `require('path')`/`require('os')`/
        // `require('fs')` at the top of its own real, unmodified source
        // (`node-gyp-build.js`) before it ever gets to the actual
        // native-addon lookup. Pure string manipulation, no dependency on
        // any crate -- `path` never touches a real filesystem in Node
        // either (that's what `fs` is for).
        "path" => Some(
            "function __thaw_path_normalize(p) {\n\
             \x20\x20var parts = p.split('/'); var out = [];\n\
             \x20\x20for (var i = 0; i < parts.length; i++) {\n\
             \x20\x20\x20\x20var part = parts[i];\n\
             \x20\x20\x20\x20if (part === '' || part === '.') continue;\n\
             \x20\x20\x20\x20if (part === '..') { out.pop(); } else { out.push(part); }\n\
             \x20\x20}\n\
             \x20\x20var abs = p.charAt(0) === '/';\n\
             \x20\x20return (abs ? '/' : '') + out.join('/');\n\
             }\n\
             function resolve() {\n\
             \x20\x20var p = '';\n\
             \x20\x20for (var i = 0; i < arguments.length; i++) {\n\
             \x20\x20\x20\x20var seg = String(arguments[i]);\n\
             \x20\x20\x20\x20if (seg.charAt(0) === '/') { p = seg; } else { p = p ? p + '/' + seg : seg; }\n\
             \x20\x20}\n\
             \x20\x20var n = __thaw_path_normalize(p);\n\
             \x20\x20return n.charAt(0) === '/' ? n : '/' + n;\n\
             }\n\
             function join() {\n\
             \x20\x20return __thaw_path_normalize(Array.prototype.join.call(arguments, '/'));\n\
             }\n\
             function dirname(p) {\n\
             \x20\x20var n = __thaw_path_normalize(p);\n\
             \x20\x20var idx = n.lastIndexOf('/');\n\
             \x20\x20if (idx <= 0) return n.charAt(0) === '/' ? '/' : '.';\n\
             \x20\x20return n.substring(0, idx);\n\
             }\n\
             function basename(p) {\n\
             \x20\x20var n = __thaw_path_normalize(p);\n\
             \x20\x20var idx = n.lastIndexOf('/');\n\
             \x20\x20return idx === -1 ? n : n.substring(idx + 1);\n\
             }\n\
             module.exports = { resolve: resolve, join: join, dirname: dirname, basename: basename, normalize: __thaw_path_normalize, sep: '/' };\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        // Same real dependency chain as `path` above (`node-gyp-build.js`
        // reads `os.arch()`/`os.platform()` to build its target string).
        "os" => Some(
            "var __thaw_os = { arch: function() { return 'x64'; }, platform: function() { return 'linux'; }, type: function() { return 'Linux'; }, tmpdir: function() { return '/tmp'; }, EOL: '\\n' };\n\
             module.exports = __thaw_os;\n\
             module.exports.default = __thaw_os;\n\
             module.exports.__esModule = true;\n",
        ),
        // Same chain again: `node-gyp-build.js` uses `fs.readdirSync` to
        // look for a prebuilt `.node` binary in `build/Release`,
        // `build/Debug`, and `prebuilds/`, always wrapped in its own
        // `try { fs.readdirSync(...) } catch (err) { return [] }` --
        // consistently reporting "nothing here" (the honest answer: the
        // registry never bundles real binaries, only `package.d.ts`/
        // `bundle.js`, see the "not yet supported" section) lets that real,
        // unmodified upstream logic run to completion and produce its own
        // specific `Error('No native build was found for ...')` -- a much
        // clearer failure than a generic "require('fs') is not supported"
        // would be, without Thaw needing to know anything about native
        // addons itself.
        "fs" => Some(
            "function __thaw_fs_enoent(op, p) {\n\
             \x20\x20var e = new Error('ENOENT: no such file or directory, ' + op + ' \\'' + p + '\\'');\n\
             \x20\x20e.code = 'ENOENT';\n\
             \x20\x20throw e;\n\
             }\n\
             var __thaw_fs = {\n\
             \x20\x20existsSync: function(p) { return false; },\n\
             \x20\x20readdirSync: function(p) { __thaw_fs_enoent('scandir', p); },\n\
             \x20\x20statSync: function(p) { __thaw_fs_enoent('stat', p); },\n\
             \x20\x20readFileSync: function(p) { __thaw_fs_enoent('open', p); },\n\
             };\n\
             module.exports = __thaw_fs;\n\
             module.exports.default = __thaw_fs;\n\
             module.exports.__esModule = true;\n",
        ),
        // Native compilation resolves the typed `node:http` surface through
        // thaw-std. Keep an empty CommonJS module here so dependency discovery
        // can still complete before the native Fast Path is selected.
        "http" => Some("module.exports = {};\n"),
        "buffer" => Some(
            "function byteLength(value) { return String(value).length; }\n\
             module.exports = { byteLength: byteLength };\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        _ => None,
    }
}

fn resolve_bare_require(
    node_modules_dir: &Path,
    spec: &str,
) -> Option<(String, String, PathBuf, PathBuf)> {
    let (dep_name, subpath) = split_bare_spec(spec);
    let dep_dir = node_modules_dir.join(dep_name);
    let (dep_relative, dep_abs) = match subpath {
        Some(sub) => resolve_module_path(&dep_dir, sub).ok()?,
        None => {
            let manifest = read_manifest(&dep_dir).ok()?;
            let main = package_export_target(&manifest, None, &["require", "import", "default"])
                .or_else(|| manifest.get("main").and_then(|v| v.as_str()))
                .unwrap_or("index.js");
            resolve_module_path(&dep_dir, main).ok()?
        }
    };
    Some((dep_name.to_string(), dep_relative, dep_abs, dep_dir))
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

/// Renders `modules` into one JS string: a small embedded CommonJS
/// module-system emulation (a factory + a per-module require-spec-to-key
/// map, both precomputed statically -- no runtime path resolution needed
/// in the emitted JS at all), then a final `module.exports = ...` that
/// invokes `main_key`. The result is exactly the kind of single JS
/// string thaw-bridge's `wrap_as_commonjs_module` already knows how to
/// run: it doesn't need to know or care that this is a bundle rather
/// than one file.
///
/// Everything except the final `module.exports = ` assignment is inside
/// an IIFE, deliberately never touching global scope: multiple
/// `--use`'d packages all get `loadScript`'d into the *same* shared
/// QuickJS-NG global context (thaw-bridge's `wrap_as_commonjs_module`,
/// called once per package), so if these helpers were plain globals, a
/// second package's bundle would stomp the first's `__thaw_bundle_cache`/
/// `__thaw_bundle_require`/etc. the moment it loaded. That's invisible
/// for a module that only calls `require` eagerly at load time (already
/// finished and cached by then), but a *lazy* internal require --
/// deferred inside a function body, called only after a later package
/// has overwritten the globals -- would silently resolve against the
/// wrong package's module map. The IIFE's closures keep each package's
/// module system private to itself regardless of what loads after it.
fn render_bundle(main_key: &str, modules: &[BundledModule]) -> String {
    let mut out = String::from("module.exports = (function() {\n");

    out.push_str("var __thaw_bundle_cache = {};\n");
    out.push_str("var __thaw_bundle_factories = {\n");
    for module in modules {
        out.push_str(&format!(
            "{}: function(module, exports, require) {{\n{}\n}},\n",
            js_string_literal(&module.key),
            module.source
        ));
    }
    out.push_str("};\n");

    out.push_str("var __thaw_bundle_require_maps = {\n");
    for module in modules {
        out.push_str(&format!("{}: {{", js_string_literal(&module.key)));
        for (spec, target) in &module.requires {
            out.push_str(&format!(
                "{}: {}, ",
                js_string_literal(spec),
                js_string_literal(target)
            ));
        }
        out.push_str("},\n");
    }
    out.push_str("};\n");

    out.push_str(
        "function __thaw_bundle_require(key) {\n\
         \x20\x20if (!(key in __thaw_bundle_cache)) {\n\
         \x20\x20\x20\x20var mod = { exports: {} };\n\
         \x20\x20\x20\x20__thaw_bundle_cache[key] = mod;\n\
         \x20\x20\x20\x20var map = __thaw_bundle_require_maps[key] || {};\n\
         \x20\x20\x20\x20__thaw_bundle_factories[key](mod, mod.exports, function(spec) {\n\
         \x20\x20\x20\x20\x20\x20if (Object.prototype.hasOwnProperty.call(map, spec)) {\n\
         \x20\x20\x20\x20\x20\x20\x20\x20return __thaw_bundle_require(map[spec]);\n\
         \x20\x20\x20\x20\x20\x20}\n\
         \x20\x20\x20\x20\x20\x20return require(spec);\n\
         \x20\x20\x20\x20});\n\
         \x20\x20}\n\
         \x20\x20return __thaw_bundle_cache[key].exports;\n\
         }\n",
    );

    out.push_str(&format!(
        "return __thaw_bundle_require({});\n",
        js_string_literal(main_key)
    ));
    out.push_str("})();\n");

    out
}

/// A double-quoted JS string literal for `s` -- used for module-map keys
/// and require specs, which in practice are always simple path-like
/// strings, but escaped properly regardless.
fn js_string_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
            package_export_target(&manifest, None, &["require", "import", "default"]),
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
    fn installed_npm_layout_registers_wildcard_subpath_artifacts() {
        let scratch = temp_registry("installed-wildcard-scratch");
        let registry = temp_registry("installed-wildcard-registry");
        let package = scratch.join("node_modules/feature-kit");
        fs::create_dir_all(package.join("dist/features")).unwrap();
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
            "export default function double(value: number): number;",
        )
        .unwrap();
        fs::write(
            package.join("dist/features/double.js"),
            "module.exports = function(value) { return value * 2; };",
        )
        .unwrap();

        let added = add_installed(&registry, &scratch.join("node_modules"), "feature-kit").unwrap();
        assert_eq!(added.resolved_version, "1.2.3");
        let subpath = resolve(&registry, "feature-kit/features/double").unwrap();
        assert!(subpath.dts_source.contains("double"));
        assert!(subpath.bundle_js.unwrap().contains("value * 2"));
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

        let selected = select_prebuilt_addon(&package).unwrap().unwrap();
        assert_eq!(selected.path, target.join(filename));
        assert_eq!(selected.platform, platform);
        assert_eq!(selected.arch, arch);
        assert_eq!(selected.libc, libc);
        let _ = fs::remove_dir_all(package);
    }

    #[test]
    fn reports_available_targets_when_no_prebuild_matches() {
        let package = temp_registry("mismatched_native_prebuild");
        let target = package.join("prebuilds/imaginary-other");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("binding.node"), b"native bytes").unwrap();
        let diagnostic = select_prebuilt_addon(&package).unwrap_err();
        assert!(diagnostic.contains("no bundled native addon matches"));
        assert!(diagnostic.contains("imaginary-other"));
        let _ = fs::remove_dir_all(package);
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
        let manifest: serde_json::Value =
            serde_json::from_str(r#"{"typings": "index.d.ts"}"#).unwrap();
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
        let specs =
            find_relative_require_specs(r#"var a = require('./a'); var b = require("../lib/b");"#);
        assert_eq!(specs, vec!["./a".to_string(), "../lib/b".to_string()]);
    }

    #[test]
    fn ignores_bare_specifier_requires() {
        let specs = find_relative_require_specs(r#"var x = require('is-number');"#);
        assert!(specs.is_empty());
    }

    #[test]
    fn finds_bare_specifiers_including_scoped_packages() {
        let specs = find_bare_require_specs(
            r#"var a = require('side-channel'); var b = require('@babel/core'); var c = require('./local');"#,
        );
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

        let (name, relative, abs, dir) =
            resolve_bare_require(&node_modules, "es-errors/type").unwrap();
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

        let (name, relative, ..) =
            resolve_bare_require(&node_modules, "@scope/pkg/lib/util").unwrap();
        assert_eq!(name, "@scope/pkg");
        assert_eq!(relative, "lib/util.js");

        let _ = fs::remove_dir_all(&node_modules);
    }

    #[test]
    fn ignores_dynamic_and_malformed_require_calls() {
        // `require(name)` (a variable, not a literal) and a stray
        // "require" that isn't actually a call must not confuse the scan
        // -- and must not stop it from still finding a real one after.
        let specs = find_relative_require_specs(
            "var x = require(name); var note = 'requirements'; var y = require('./y');",
        );
        assert_eq!(specs, vec!["./y".to_string()]);
    }

    #[test]
    fn normalizes_dot_and_dot_dot_segments() {
        assert_eq!(normalize_path_string("lib/./stringify"), "lib/stringify");
        assert_eq!(normalize_path_string("lib/../parse"), "parse");
        assert_eq!(normalize_path_string("a/b/../../c"), "c");
    }

    /// The exact shape found in a real npm package (`qs`): `main` requires
    /// two sibling files by relative path, each with no further requires
    /// of their own.
    #[test]
    fn bundles_a_multi_file_package_reachable_from_main() {
        let dir = temp_registry("bundle_multi_file");
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(
            dir.join("lib/index.js"),
            "var parse = require('./parse');\nvar stringify = require('./stringify');\n\
             module.exports = { parse: parse, stringify: stringify };",
        )
        .unwrap();
        fs::write(
            dir.join("lib/parse.js"),
            "module.exports = function parse(s) { return s; };",
        )
        .unwrap();
        fs::write(
            dir.join("lib/stringify.js"),
            "module.exports = function stringify(s) { return s; };",
        )
        .unwrap();

        let empty_node_modules = temp_registry("bundle_multi_file_node_modules");
        let (bundle, main_key, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "lib/index.js").unwrap();
        assert_eq!(main_key, "pkg/lib/index.js");
        assert_eq!(file_count, 3, "main + parse.js + stringify.js");
        assert!(bundle.contains("pkg/lib/parse.js"));
        assert!(bundle.contains("pkg/lib/stringify.js"));

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    /// `bundle_commonjs_package`'s version-recording half (the other half
    /// of `add`'s 17章 `version.txt` work, extended to cover every
    /// package the bundle actually reaches, not just the root one --
    /// `AddedPackage::dependency_versions`/`lock.json`). Both the root
    /// package (`pkg`) and its one real dependency (`left-pad-ish`) have
    /// their own `package.json` with a `version` field here, mirroring
    /// what `npm install` actually leaves on disk.
    #[test]
    fn bundle_commonjs_package_records_every_reached_packages_version() {
        let dir = temp_registry("bundle_versions_root");
        fs::write(
            dir.join("package.json"),
            r#"{"name": "pkg", "version": "2.5.0", "main": "index.js"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("index.js"),
            "var dep = require('left-pad-ish');\nmodule.exports = dep;",
        )
        .unwrap();

        let node_modules = temp_registry("bundle_versions_node_modules");
        fs::create_dir_all(node_modules.join("left-pad-ish")).unwrap();
        fs::write(
            node_modules.join("left-pad-ish/package.json"),
            r#"{"name": "left-pad-ish", "version": "1.3.0", "main": "index.js"}"#,
        )
        .unwrap();
        fs::write(
            node_modules.join("left-pad-ish/index.js"),
            "module.exports = function () { return 'padded'; };",
        )
        .unwrap();

        let (_, _, _, dependency_versions) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();

        assert_eq!(
            dependency_versions.get("pkg").map(String::as_str),
            Some("2.5.0")
        );
        assert_eq!(
            dependency_versions.get("left-pad-ish").map(String::as_str),
            Some("1.3.0")
        );
        assert_eq!(dependency_versions.len(), 2, "no extra/missing entries");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&node_modules);
    }

    /// A package with no dependencies still gets exactly one entry (its
    /// own) -- `fetch_and_copy` uses this len-1 case to decide *not* to
    /// write a redundant `lock.json` next to `version.txt`.
    #[test]
    fn bundle_commonjs_package_with_no_dependencies_records_only_itself() {
        let dir = temp_registry("bundle_versions_solo");
        fs::write(
            dir.join("package.json"),
            r#"{"name": "solo-pkg", "version": "0.1.0", "main": "index.js"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("index.js"),
            "module.exports = function () { return 1; };",
        )
        .unwrap();

        let empty_node_modules = temp_registry("bundle_versions_solo_node_modules");
        let (_, _, _, dependency_versions) =
            bundle_commonjs_package(&empty_node_modules, "solo-pkg", &dir, "index.js").unwrap();

        assert_eq!(dependency_versions.len(), 1);
        assert_eq!(
            dependency_versions.get("solo-pkg").map(String::as_str),
            Some("0.1.0")
        );

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn bundle_of_a_single_file_package_still_has_one_module() {
        let dir = temp_registry("bundle_single_file");
        fs::write(
            dir.join("index.js"),
            "module.exports = function f() { return 1; };",
        )
        .unwrap();

        let empty_node_modules = temp_registry("bundle_single_file_node_modules");
        let (_, main_key, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(main_key, "pkg/index.js");
        assert_eq!(file_count, 1);

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    /// An unresolvable relative require (here: a `.json` target, which
    /// `resolve_module_path`'s candidates don't cover) must not abort
    /// bundling the rest of the package -- it's just left for the
    /// external-require stub to report clearly if actually called.
    #[test]
    fn unresolvable_relative_require_does_not_abort_bundling() {
        let dir = temp_registry("bundle_unresolvable_require");
        fs::write(
            dir.join("index.js"),
            "var pkg = require('./package.json');\nmodule.exports = function f() { return 1; };",
        )
        .unwrap();
        // Deliberately no package.json written -- this require can never
        // resolve via resolve_module_path's .js/index.js candidates.

        let empty_node_modules = temp_registry("bundle_unresolvable_require_node_modules");
        let (_, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 1);

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    /// The bundle isn't just plausible-looking text -- it must actually
    /// run correctly through the real QuickJS-NG engine, including a
    /// same-package relative require resolving to a sibling module *and*
    /// a bare-specifier require resolving to a real dependency package
    /// under `node_modules` (the actual new capability: `qs`'s own
    /// dependency on `side-channel`, reproduced in miniature). A third,
    /// genuinely external require (something not present under
    /// `node_modules` at all, mirroring a Node core builtin or a
    /// dependency `npm install` didn't fetch) is left inside a function
    /// that's never called -- were it eager and reached, it would throw
    /// immediately, same as `is-odd`'s real `require('is-number')`
    /// (already covered by this session's end-to-end verification); this
    /// test is specifically about what *does* resolve.
    #[test]
    fn bundle_actually_runs_through_quickjs() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("bundle_runs_through_quickjs");
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(
            dir.join("lib/index.js"),
            "var double = require('./double');\n\
             var triple = require('triple-dep');\n\
             function unused() { return require('a-package-that-was-never-installed'); }\n\
             module.exports = function run(n) { return double(triple(n)); };",
        )
        .unwrap();
        fs::write(
            dir.join("lib/double.js"),
            "module.exports = function (n) { return n * 2; };",
        )
        .unwrap();

        let node_modules_dir = temp_registry("bundle_runs_through_quickjs_node_modules");
        fs::create_dir_all(node_modules_dir.join("triple-dep")).unwrap();
        fs::write(
            node_modules_dir.join("triple-dep/package.json"),
            r#"{"main": "index.js"}"#,
        )
        .unwrap();
        fs::write(
            node_modules_dir.join("triple-dep/index.js"),
            "module.exports = function (n) { return n * 3; };",
        )
        .unwrap();

        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules_dir, "pkg", &dir, "lib/index.js").unwrap();
        assert_eq!(
            file_count, 3,
            "pkg's index.js + double.js + triple-dep's index.js"
        );

        // Same environment thaw-bridge's `wrap_as_commonjs_module` sets
        // up: global `module`/`exports`/`require` before running the
        // source, then bind the default export by name afterward.
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.run = module.exports;\n"
        );

        let source = CString::new(script).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source.as_ptr()),
            1,
            "bundle failed to load"
        );

        let func = CString::new("run").unwrap();
        let args = CString::new("[7]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            result, "42",
            "relative require (double) and cross-package bare require (triple-dep) must both resolve correctly"
        );

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&node_modules_dir);
    }

    /// Real `--use pkg-a --use pkg-b` loads each package's bundle into the
    /// *same* shared thread-local QuickJS-NG context, one `loadScript`
    /// call per package (`wrap_as_commonjs_module`, called once per
    /// package). Before wrapping each bundle's module-system helpers in
    /// an IIFE, they were plain globals (`__thaw_bundle_cache` etc.), so
    /// loading package B would silently overwrite package A's -- invisible
    /// for a require resolved eagerly at load time (already finished and
    /// cached by then), but package A's *lazy* internal require (deferred
    /// inside a function body, only actually called after B has loaded)
    /// would then resolve against B's module map instead of its own.
    #[test]
    fn multiple_bundled_packages_dont_stomp_each_others_module_state() {
        use std::ffi::{CStr, CString};

        let node_modules_dir = temp_registry("multi_pkg_node_modules");

        let dir_a = temp_registry("multi_pkg_a");
        fs::write(
            dir_a.join("index.js"),
            "module.exports = function getLazy() { return require('./lazy')(); };",
        )
        .unwrap();
        fs::write(
            dir_a.join("lazy.js"),
            "module.exports = function () { return 'from lazy'; };",
        )
        .unwrap();
        let (bundle_a, _, _, _) =
            bundle_commonjs_package(&node_modules_dir, "pkg-a", &dir_a, "index.js").unwrap();

        let dir_b = temp_registry("multi_pkg_b");
        fs::write(
            dir_b.join("index.js"),
            "module.exports = function () { return 'b'; };",
        )
        .unwrap();
        let (bundle_b, _, _, _) =
            bundle_commonjs_package(&node_modules_dir, "pkg-b", &dir_b, "index.js").unwrap();

        let wrap = |bundle: &str, bind_as: &str| {
            format!(
                "globalThis.module = {{ exports: {{}} }};\n\
                 globalThis.exports = globalThis.module.exports;\n\
                 globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
                 {bundle}\n\
                 globalThis.{bind_as} = module.exports;\n"
            )
        };

        let source_a = CString::new(wrap(&bundle_a, "getLazy")).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source_a.as_ptr()),
            1,
            "package A failed to load"
        );

        // Loaded into the same shared global context *after* A.
        let source_b = CString::new(wrap(&bundle_b, "pkgB")).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source_b.as_ptr()),
            1,
            "package B failed to load"
        );

        // Call A's lazily-requiring function *after* B has loaded.
        let func = CString::new("getLazy").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            result, "\"from lazy\"",
            "package A's lazy internal require must still resolve against its own module map, \
             not package B's, after B has loaded into the shared global context"
        );

        let _ = fs::remove_dir_all(&dir_a);
        let _ = fs::remove_dir_all(&dir_b);
        let _ = fs::remove_dir_all(&node_modules_dir);
    }

    /// The exact shape found in `qs`'s real dependency chain:
    /// `object-inspect` (pulled in transitively) does
    /// `require('util').inspect.custom` unconditionally at load time,
    /// with no matching `node_modules/util` -- must resolve via the
    /// `util` builtin polyfill instead of falling through to the
    /// external-require stub.
    #[test]
    fn bundles_the_util_builtin_polyfill_when_required() {
        let dir = temp_registry("builtin_util");
        fs::write(
            dir.join("index.js"),
            "var inspect = require('util').inspect;\n\
             module.exports = function () { return typeof inspect.custom; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_util_node_modules");

        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2, "pkg's index.js + the util polyfill");
        assert!(bundle.contains("node:util"));

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    /// The exact pattern found in a real ESM npm package (`has-flag`):
    /// `import process from 'process'`, then reading `process.argv`.
    #[test]
    fn process_builtin_polyfill_actually_runs_through_quickjs() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_process");
        fs::write(
            dir.join("index.js"),
            "import process from 'process';\n\
             export default function getPlatform() { return process.platform; }",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_process_node_modules");

        let (bundle, _, _, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.getPlatform = module.exports.default;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source.as_ptr()),
            1,
            "failed to load"
        );

        let func = CString::new("getPlatform").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(result, "\"linux\"");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    /// Not just plausible text -- runs through the real QuickJS-NG
    /// engine, confirming `util.inspect.custom` actually comes back as a
    /// real `Symbol` (what `object-inspect` needs it to be), not merely
    /// present.
    #[test]
    fn util_polyfill_actually_runs_through_quickjs() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_util_runs");
        fs::write(
            dir.join("index.js"),
            "var inspect = require('util').inspect;\n\
             module.exports = function () { return typeof inspect.custom; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_util_runs_node_modules");

        let (bundle, _, _, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.checkInspectCustom = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source.as_ptr()),
            1,
            "bundle failed to load"
        );

        let func = CString::new("checkInspectCustom").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(result, "\"symbol\"");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    /// The exact real-world pattern that motivated `path`/`os`/`fs`: a real
    /// native addon package (`bcrypt`, `utf-8-validate`, ...) depends on
    /// `node-gyp-build`, whose real, unmodified source reads
    /// `fs.readdirSync` (always wrapped in its own try/catch expecting
    /// `[]` back on failure), `path.join`/`path.resolve`/`path.dirname`,
    /// and `os.arch`/`os.platform` while hunting for a prebuilt `.node`
    /// binary that this registry never bundles. Confirms all three
    /// polyfills actually run together through real QuickJS-NG and that
    /// `fs.readdirSync` failing is silently absorbed exactly the way real
    /// Node's `ENOENT` would be, rather than crashing the whole load.
    #[test]
    fn path_os_fs_polyfills_actually_run_through_quickjs() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_addon_chase");
        fs::write(
            dir.join("index.js"),
            "var fs = require('fs');\n\
             var path = require('path');\n\
             var os = require('os');\n\
             function readdirSync(d) { try { return fs.readdirSync(d); } catch (err) { return []; } }\n\
             module.exports = function locate() {\n\
             \x20\x20var dir = path.resolve(__dirname);\n\
             \x20\x20var release = readdirSync(path.join(dir, 'build/Release'));\n\
             \x20\x20return os.platform() + '/' + os.arch() + '/' + path.dirname(dir) + '/' + release.length;\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_addon_chase_node_modules");

        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4, "pkg's index.js + fs/path/os polyfills");

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             globalThis.__dirname = '/thaw_modules/pkg';\n\
             {bundle}\n\
             globalThis.locate = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source.as_ptr()),
            1,
            "bundle failed to load"
        );

        let func = CString::new("locate").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(result, "\"linux/x64//thaw_modules/0\"");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn plain_commonjs_is_left_untouched() {
        assert!(rewrite_esm_to_commonjs("module.exports = function f() { return 1; };").is_none());
    }

    #[test]
    fn rewrites_default_export_to_module_exports_default() {
        let rewritten =
            rewrite_esm_to_commonjs("export default function greet() { return 'hi'; }").unwrap();
        assert!(rewritten.contains("module.exports.default = function greet() { return 'hi'; }"));
        assert!(rewritten.contains("module.exports.__esModule = true;"));
    }

    #[test]
    fn rewrites_named_export_and_binds_it_too() {
        let rewritten =
            rewrite_esm_to_commonjs("export function add(a, b) { return a + b; }").unwrap();
        assert!(rewritten.contains("function add(a, b) { return a + b; }"));
        assert!(rewritten.contains("exports.add = add;"));
    }

    #[test]
    fn rewrites_named_import_to_a_require_call() {
        let rewritten =
            rewrite_esm_to_commonjs("import { add } from './math';\nconsole.log(add(1, 2));")
                .unwrap();
        assert!(rewritten.contains("require(\"./math\")"));
        assert!(rewritten.contains("console.log(add(1, 2));"));
    }

    /// The bundle isn't just plausible-looking text: an ESM main file
    /// importing from an ESM sibling file must actually run correctly
    /// through the real QuickJS-NG engine, exactly like the equivalent
    /// CommonJS package already does (`bundle_actually_runs_through_quickjs`).
    #[test]
    fn esm_bundle_actually_runs_through_quickjs() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("esm_bundle_runs_through_quickjs");
        fs::write(
            dir.join("index.js"),
            "import { double } from './double.js';\nexport default function run(n) { return double(n); }",
        )
        .unwrap();
        fs::write(
            dir.join("double.js"),
            "export function double(n) { return n * 2; }",
        )
        .unwrap();

        let empty_node_modules = temp_registry("esm_bundle_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2, "index.js + double.js");

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.run = module.exports.default;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source.as_ptr()),
            1,
            "ESM bundle failed to load"
        );

        let func = CString::new("run").unwrap();
        let args = CString::new("[21]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(result, "42");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }
}
