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
/// returns. `bundled_file_count` is how many of the package's own files
/// (`main` plus everything it reaches via same-package relative
/// `require`s) got folded into `bundle.js` -- 1 for a package that's
/// just a single file.
#[derive(Debug, PartialEq)]
pub struct AddedPackage {
    pub dts_relative_path: String,
    pub js_relative_path: String,
    pub bundled_file_count: usize,
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
    npm_install(scratch, package)?;

    let package_dir = scratch.join("node_modules").join(package);
    let manifest = read_manifest(&package_dir)?;

    let main_field = manifest.get("main").and_then(|v| v.as_str()).unwrap_or("index.js");
    let (js_source, js_relative_path, bundled_file_count) =
        bundle_commonjs_package(&package_dir, main_field)?;

    let (dts_relative_path, dts_source) = match find_own_dts(&manifest, &package_dir) {
        Some((rel, abs)) => {
            let source = fs::read_to_string(&abs)
                .map_err(|e| format!("failed to read `{rel}`: {e}"))?;
            (rel, source)
        }
        None => fetch_types_package_dts(scratch, package)?,
    };

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
        bundled_file_count,
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
    match package.strip_prefix('@').and_then(|rest| rest.split_once('/')) {
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
        format!("{trimmed}/index.js"),
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

/// One file pulled into a `bundle_commonjs_package` bundle. `key` is the
/// path `resolve_module_path` resolved it to (relative to the package
/// root) -- used both as the emitted module map's key and, resolved
/// against `package_dir`, to read the file. `requires` is every
/// same-package relative `require` spec this file's source contains,
/// each already resolved to its own target `key`.
struct BundledModule {
    key: String,
    source: String,
    requires: Vec<(String, String)>,
}

/// Bundles a package's own internal CommonJS module graph -- starting
/// from `main_relative` (the `main` field, or a default) -- into a
/// single self-contained JS string with a small embedded module-system
/// emulation, so `thaw-bridge`'s `wrap_as_commonjs_module` (which only
/// ever sees one JS string per package) can still run it correctly.
///
/// Found necessary by running a real npm package (`qs`) through `add`:
/// its `main` (`lib/index.js`) does `require('./stringify')`,
/// `require('./parse')`, `require('./formats')` -- same-package relative
/// requires, nothing to do with an external dependency. The previous
/// single-file `bundle.js` (thaw-bridge's global `require` stub throws
/// unconditionally) failed at load time for *any* package split across
/// more than one file, which is the common case for anything beyond a
/// trivial one-file utility.
///
/// A `require` call to a *bare* specifier (another npm package, e.g.
/// `require('is-number')`) is left completely alone in the emitted
/// code -- at runtime it still resolves to whatever `require` is in
/// scope, i.e. thaw-bridge's global stub that throws a clear "not
/// supported" error. Real inter-package dependency resolution stays out
/// of scope (docs/design/registry.md); only a package's *own* internal
/// file layout is handled here.
///
/// Returns the bundle text, the resolved key of `main_relative` (the
/// bundle's entry point, for `AddedPackage` reporting), and the total
/// number of files folded in.
fn bundle_commonjs_package(
    package_dir: &Path,
    main_relative: &str,
) -> Result<(String, String, usize), String> {
    let (main_key, main_abs) = resolve_module_path(package_dir, main_relative)?;

    let mut modules: Vec<BundledModule> = Vec::new();
    let mut visited: Vec<String> = vec![main_key.clone()];
    let mut worklist: Vec<(String, PathBuf)> = vec![(main_key.clone(), main_abs)];

    while let Some((key, abs_path)) = worklist.pop() {
        let source = fs::read_to_string(&abs_path)
            .map_err(|e| format!("failed to read `{key}` while bundling: {e}"))?;

        let requiring_dir = Path::new(&key).parent().unwrap_or(Path::new(""));
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
            if let Ok((resolved_key, resolved_abs)) = resolve_module_path(package_dir, &normalized) {
                requires.push((spec, resolved_key.clone()));
                if !visited.contains(&resolved_key) {
                    visited.push(resolved_key.clone());
                    worklist.push((resolved_key, resolved_abs));
                }
            }
        }

        modules.push(BundledModule { key, source, requires });
    }

    let file_count = modules.len();
    Ok((render_bundle(&main_key, &modules), main_key, file_count))
}

/// Finds `require('./x')`/`require("../y")` call specs in raw JS text --
/// deliberately just a text scan, not a real parser (this project has no
/// general JS/CommonJS parser, only thaw-parser's TS-oriented one for
/// `.d.ts`/`.ts`). Only *relative* specs (starting with `./` or `../`)
/// are returned; a bare specifier (another npm package) is intentionally
/// left alone -- see `bundle_commonjs_package`'s doc comment. A dynamic
/// `require(someVariable)` or a template-literal spec simply won't be
/// found, which just means that one call falls through to whatever
/// runtime `require` is in scope -- no worse than every `require` call
/// failing outright, which is what happened before this existed.
fn find_relative_require_specs(source: &str) -> Vec<String> {
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

        let spec = &source[content_start..j];
        if spec.starts_with("./") || spec.starts_with("../") {
            specs.push(spec.to_string());
        }
    }
    specs
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
fn render_bundle(main_key: &str, modules: &[BundledModule]) -> String {
    let mut out = String::new();

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
        "module.exports = __thaw_bundle_require({});\n",
        js_string_literal(main_key)
    ));

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
    /// than in `cargo test` -- see docs/design/registry.md). `find_own_dts`/
    /// `types_package_name` are the pieces of `add` with real decision
    /// logic and no network dependency, so they get full offline coverage
    /// here.
    #[test]
    fn finds_dts_from_types_field() {
        let manifest: serde_json::Value = serde_json::from_str(r#"{"types": "dist/index.d.ts"}"#).unwrap();
        let dir = temp_registry("dts_types_field");
        let (rel, abs) = find_own_dts(&manifest, &dir).unwrap();
        assert_eq!(rel, "dist/index.d.ts");
        assert_eq!(abs, dir.join("dist/index.d.ts"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_dts_from_typings_field_when_types_is_absent() {
        let manifest: serde_json::Value = serde_json::from_str(r#"{"typings": "index.d.ts"}"#).unwrap();
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
        let specs = find_relative_require_specs(
            r#"var a = require('./a'); var b = require("../lib/b");"#,
        );
        assert_eq!(specs, vec!["./a".to_string(), "../lib/b".to_string()]);
    }

    #[test]
    fn ignores_bare_specifier_requires() {
        let specs = find_relative_require_specs(r#"var x = require('is-number');"#);
        assert!(specs.is_empty());
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
        fs::write(dir.join("lib/parse.js"), "module.exports = function parse(s) { return s; };").unwrap();
        fs::write(
            dir.join("lib/stringify.js"),
            "module.exports = function stringify(s) { return s; };",
        )
        .unwrap();

        let (bundle, main_key, file_count) = bundle_commonjs_package(&dir, "lib/index.js").unwrap();
        assert_eq!(main_key, "lib/index.js");
        assert_eq!(file_count, 3, "main + parse.js + stringify.js");
        assert!(bundle.contains("lib/parse.js"));
        assert!(bundle.contains("lib/stringify.js"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bundle_of_a_single_file_package_still_has_one_module() {
        let dir = temp_registry("bundle_single_file");
        fs::write(dir.join("index.js"), "module.exports = function f() { return 1; };").unwrap();

        let (_, main_key, file_count) = bundle_commonjs_package(&dir, "index.js").unwrap();
        assert_eq!(main_key, "index.js");
        assert_eq!(file_count, 1);

        let _ = fs::remove_dir_all(&dir);
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

        let (_, _, file_count) = bundle_commonjs_package(&dir, "index.js").unwrap();
        assert_eq!(file_count, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    /// The bundle isn't just plausible-looking text -- it must actually
    /// run correctly through the real QuickJS-NG engine, including a
    /// same-package relative require resolving to the right sibling
    /// module and an external bare-specifier require still reaching
    /// whatever `require` thaw-bridge's `wrap_as_commonjs_module` set up
    /// (simulated here directly, without depending on thaw-bridge).
    #[test]
    fn bundle_actually_runs_through_quickjs() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("bundle_runs_through_quickjs");
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(
            dir.join("lib/index.js"),
            // The external require is deliberately inside a function
            // that's never called: a real, eager, top-level external
            // require (like `is-odd`'s `require('is-number')`) throws
            // immediately when the module loads -- already covered by
            // this session's `is-odd` end-to-end verification. This test
            // is specifically about the internal `./double` require
            // resolving correctly, so the external one must not fire.
            "var double = require('./double');\n\
             function unused() { return require('an-external-package'); }\n\
             module.exports = function run(n) { return double(n); };",
        )
        .unwrap();
        fs::write(
            dir.join("lib/double.js"),
            "module.exports = function (n) { return n * 2; };",
        )
        .unwrap();

        let (bundle, _, file_count) = bundle_commonjs_package(&dir, "lib/index.js").unwrap();
        assert_eq!(file_count, 2);

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
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1, "bundle failed to load");

        let func = CString::new("run").unwrap();
        let args = CString::new("[21]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned();
        assert_eq!(result, "42", "same-package relative require didn't resolve correctly");

        let _ = fs::remove_dir_all(&dir);
    }
}
