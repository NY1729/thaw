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
//!   bundle.js      (optional)  -- real JS implementation backing the
//!                                  package's Fallback functions; its
//!                                  source is fed to thaw-bridge's
//!                                  `generate_module_init` so it's loaded
//!                                  automatically at program startup
//!                                  (replaces a manual `loadScript` call)
//! ```
//!
//! No network fetch, no version resolution, and no build step for the
//! native lib -- this crate only resolves a package name to the files
//! already sitting on disk. Those deferred pieces are exactly what the
//! project's design doc calls "the actual differentiator"; this is a
//! placeholder for the local half of it, real enough to remove the
//! remaining manual `--bridge`/`--link`/`loadScript` steps for a package
//! that's already been fetched/built by some other means.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A package resolved from a local registry directory. `native_lib` and
/// `bundle_js` are independently optional: a pure Fast path package needs
/// no `bundle.js`, and a pure Fallback package needs no `native.a`.
#[derive(Debug)]
pub struct ResolvedPackage {
    pub name: String,
    pub dts_source: String,
    pub native_lib: Option<PathBuf>,
    pub bundle_js: Option<String>,
}

/// Resolves `name` against `registry_dir/<name>/`. Fails only if
/// `package.d.ts` is missing or unreadable -- that's the one required
/// file, since a package with neither a native lib nor a bundle would
/// have nothing for thaw-bridge's shim to call.
pub fn resolve(registry_dir: &Path, name: &str) -> Result<ResolvedPackage, String> {
    let dir = registry_dir.join(name);

    let dts_path = dir.join("package.d.ts");
    let dts_source = fs::read_to_string(&dts_path).map_err(|e| {
        format!(
            "registry package `{name}`: failed to read `{}`: {e}",
            dts_path.display()
        )
    })?;

    let native_lib_path = dir.join("native.a");
    let native_lib = native_lib_path.is_file().then_some(native_lib_path);

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

    Ok(ResolvedPackage {
        name: name.to_string(),
        dts_source,
        native_lib,
        bundle_js,
    })
}

/// Where a freshly `add`ed package's declarations/JS entry came from,
/// inside the fetched package itself -- informational only, since the
/// scratch directory these were read from is deleted before `add`
/// returns.
#[derive(Debug, PartialEq)]
pub struct AddedPackage {
    pub dts_relative_path: String,
    pub js_relative_path: String,
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
/// V1 only handles packages that bundle their own type declarations (a
/// `types`/`typings` field in `package.json`, or a same-named
/// `index.d.ts` next to `main`) -- a package needing a separate
/// `@types/*` package, an ESM-only package, and a native addon (this
/// never produces a `native.a`) are all still out of scope; see the
/// design doc's closing section.
pub fn add(registry_dir: &Path, package: &str) -> Result<AddedPackage, String> {
    let scratch = std::env::temp_dir().join(format!(
        "thaw-registry-add-{}-{}",
        package.replace('/', "_"),
        std::process::id()
    ));
    fs::create_dir_all(&scratch).map_err(|e| {
        format!("failed to create scratch directory `{}`: {e}", scratch.display())
    })?;

    let result = fetch_and_copy(&scratch, registry_dir, package);
    let _ = fs::remove_dir_all(&scratch);
    result
}

fn fetch_and_copy(
    scratch: &Path,
    registry_dir: &Path,
    package: &str,
) -> Result<AddedPackage, String> {
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

    let package_dir = scratch.join("node_modules").join(package);
    let manifest_path = package_dir.join("package.json");
    let manifest_source = fs::read_to_string(&manifest_path).map_err(|e| {
        format!(
            "failed to read `{}` after `npm install`: {e}",
            manifest_path.display()
        )
    })?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_source)
        .map_err(|e| format!("`{}` is not valid JSON: {e}", manifest_path.display()))?;

    let dts_relative_path = find_dts_path(&manifest, &package_dir).ok_or_else(|| {
        format!(
            "`{package}` has no bundled type declarations (no `types`/`typings` field in \
             package.json, no `index.d.ts`) -- packages needing a separate `@types/{package}` \
             aren't supported yet; add a `package.d.ts` by hand instead"
        )
    })?;
    let js_relative_path = manifest
        .get("main")
        .and_then(|v| v.as_str())
        .unwrap_or("index.js")
        .to_string();

    let dts_source = fs::read_to_string(package_dir.join(&dts_relative_path))
        .map_err(|e| format!("failed to read `{dts_relative_path}`: {e}"))?;
    let js_source = fs::read_to_string(package_dir.join(&js_relative_path))
        .map_err(|e| format!("failed to read `{js_relative_path}`: {e}"))?;

    let dest_dir = registry_dir.join(package);
    fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("failed to create `{}`: {e}", dest_dir.display()))?;
    fs::write(dest_dir.join("package.d.ts"), dts_source)
        .map_err(|e| format!("failed to write `{}`: {e}", dest_dir.join("package.d.ts").display()))?;
    fs::write(dest_dir.join("bundle.js"), js_source)
        .map_err(|e| format!("failed to write `{}`: {e}", dest_dir.join("bundle.js").display()))?;

    Ok(AddedPackage {
        dts_relative_path,
        js_relative_path,
    })
}

/// `types`/`typings` field first (in that order -- both spellings are
/// common in the wild), then a same-named `index.d.ts` next to `main` as
/// a last resort (common for older packages predating the `types` field
/// convention, e.g. left-pad/slugify).
fn find_dts_path(manifest: &serde_json::Value, package_dir: &Path) -> Option<String> {
    for field in ["types", "typings"] {
        if let Some(path) = manifest.get(field).and_then(|v| v.as_str()) {
            return Some(path.to_string());
        }
    }
    if package_dir.join("index.d.ts").is_file() {
        return Some("index.d.ts".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn resolves_a_package_with_all_three_files() {
        let registry = temp_registry("full");
        let pkg_dir = registry.join("left-pad");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("package.d.ts"),
            "export declare function pad(s: string): string;",
        )
        .unwrap();
        fs::write(pkg_dir.join("native.a"), b"fake archive").unwrap();
        fs::write(pkg_dir.join("bundle.js"), "function pad(s){return s;}").unwrap();

        let resolved = resolve(&registry, "left-pad").unwrap();
        assert_eq!(resolved.name, "left-pad");
        assert!(resolved.dts_source.contains("declare function pad"));
        assert_eq!(resolved.native_lib, Some(pkg_dir.join("native.a")));
        assert_eq!(
            resolved.bundle_js.as_deref(),
            Some("function pad(s){return s;}")
        );

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
    /// than in `cargo test` -- see docs/design/registry.md). `find_dts_path`
    /// is the one piece of `add` with real decision logic and no network
    /// dependency, so it gets full offline coverage here.
    #[test]
    fn finds_dts_from_types_field() {
        let manifest: serde_json::Value = serde_json::from_str(r#"{"types": "dist/index.d.ts"}"#).unwrap();
        let dir = temp_registry("dts_types_field");
        assert_eq!(find_dts_path(&manifest, &dir), Some("dist/index.d.ts".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_dts_from_typings_field_when_types_is_absent() {
        let manifest: serde_json::Value = serde_json::from_str(r#"{"typings": "index.d.ts"}"#).unwrap();
        let dir = temp_registry("dts_typings_field");
        assert_eq!(find_dts_path(&manifest, &dir), Some("index.d.ts".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prefers_types_field_over_typings_field() {
        let manifest: serde_json::Value =
            serde_json::from_str(r#"{"types": "a.d.ts", "typings": "b.d.ts"}"#).unwrap();
        let dir = temp_registry("dts_prefers_types");
        assert_eq!(find_dts_path(&manifest, &dir), Some("a.d.ts".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn falls_back_to_index_d_ts_when_no_field_is_present() {
        let manifest: serde_json::Value = serde_json::from_str(r#"{"main": "index.js"}"#).unwrap();
        let dir = temp_registry("dts_index_fallback");
        fs::write(dir.join("index.d.ts"), "declare function f(): void;").unwrap();
        assert_eq!(find_dts_path(&manifest, &dir), Some("index.d.ts".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_no_dts_when_nothing_is_bundled() {
        let manifest: serde_json::Value = serde_json::from_str(r#"{"main": "index.js"}"#).unwrap();
        let dir = temp_registry("dts_none");
        assert_eq!(find_dts_path(&manifest, &dir), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
