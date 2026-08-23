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

    let node_modules_dir = scratch.join("node_modules");
    let package_dir = node_modules_dir.join(package);
    let manifest = read_manifest(&package_dir)?;

    let main_field = manifest.get("main").and_then(|v| v.as_str()).unwrap_or("index.js");
    let (js_source, js_relative_path, bundled_file_count) =
        bundle_commonjs_package(&node_modules_dir, package, &package_dir, main_field)?;

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
/// bundle's entry point, for `AddedPackage` reporting), and the total
/// number of files folded in (across every package the bundle reaches).
fn bundle_commonjs_package(
    node_modules_dir: &Path,
    root_package: &str,
    root_package_dir: &Path,
    main_relative: &str,
) -> Result<(String, String, usize), String> {
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
            if let Ok((resolved_relative, resolved_abs)) = resolve_module_path(&pkg_dir, &normalized) {
                let resolved_key = format!("{pkg_name}/{resolved_relative}");
                requires.push((spec, resolved_key.clone()));
                if !visited.contains(&resolved_key) {
                    visited.push(resolved_key.clone());
                    worklist.push((resolved_key, resolved_abs, pkg_name.clone(), pkg_dir.clone()));
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

        modules.push(BundledModule { key, source, requires });
    }

    let file_count = modules.len();
    Ok((render_bundle(&main_key, &modules), main_key, file_count))
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
                prologue.push_str(&format!("var {var_name} = require({});\n", js_string_literal(&spec)));
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
                    prologue.push_str(&format!("var {var_name} = require({});\n", js_string_literal(&spec)));
                    for spec in &named.specifiers {
                        if let ExportSpecifier::Named(n) = spec {
                            let orig = export_name(&n.orig);
                            let exported = n.exported.as_ref().map(&export_name).unwrap_or_else(|| orig.clone());
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
                            let exported = n.exported.as_ref().map(&export_name).unwrap_or_else(|| orig.clone());
                            rest.push_str(&format!("exports.{exported} = {orig};\n"));
                        }
                    }
                }
            },
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export_all)) => {
                let var_name = format!("__thaw_esm_reexport_all_{synthetic_count}");
                synthetic_count += 1;
                let spec = export_all.src.value.to_string_lossy();
                prologue.push_str(&format!("var {var_name} = require({});\n", js_string_literal(&spec)));
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
            "var __thaw_process = { argv: [], env: {}, platform: 'linux', version: '', versions: {}, nextTick: function(fn) { fn(); } };\n\
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
            let main = manifest.get("main").and_then(|v| v.as_str()).unwrap_or("index.js");
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
    fn finds_bare_specifiers_including_scoped_packages() {
        let specs = find_bare_require_specs(
            r#"var a = require('side-channel'); var b = require('@babel/core'); var c = require('./local');"#,
        );
        assert_eq!(specs, vec!["side-channel".to_string(), "@babel/core".to_string()]);
    }

    #[test]
    fn splits_bare_specs_into_package_and_subpath() {
        assert_eq!(split_bare_spec("lodash"), ("lodash", None));
        assert_eq!(split_bare_spec("lodash/fp"), ("lodash", Some("fp")));
        assert_eq!(split_bare_spec("es-errors/type"), ("es-errors", Some("type")));
        assert_eq!(split_bare_spec("@babel/core"), ("@babel/core", None));
        assert_eq!(
            split_bare_spec("@babel/core/lib/index"),
            ("@babel/core", Some("lib/index"))
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
        fs::write(node_modules.join("es-errors/index.js"), "module.exports = {};").unwrap();
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
        fs::write(dir.join("lib/parse.js"), "module.exports = function parse(s) { return s; };").unwrap();
        fs::write(
            dir.join("lib/stringify.js"),
            "module.exports = function stringify(s) { return s; };",
        )
        .unwrap();

        let empty_node_modules = temp_registry("bundle_multi_file_node_modules");
        let (bundle, main_key, file_count) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "lib/index.js").unwrap();
        assert_eq!(main_key, "pkg/lib/index.js");
        assert_eq!(file_count, 3, "main + parse.js + stringify.js");
        assert!(bundle.contains("pkg/lib/parse.js"));
        assert!(bundle.contains("pkg/lib/stringify.js"));

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn bundle_of_a_single_file_package_still_has_one_module() {
        let dir = temp_registry("bundle_single_file");
        fs::write(dir.join("index.js"), "module.exports = function f() { return 1; };").unwrap();

        let empty_node_modules = temp_registry("bundle_single_file_node_modules");
        let (_, main_key, file_count) =
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
        let (_, _, file_count) =
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

        let (bundle, _, file_count) =
            bundle_commonjs_package(&node_modules_dir, "pkg", &dir, "lib/index.js").unwrap();
        assert_eq!(file_count, 3, "pkg's index.js + double.js + triple-dep's index.js");

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
        let args = CString::new("[7]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned();
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
        let (bundle_a, _, _) =
            bundle_commonjs_package(&node_modules_dir, "pkg-a", &dir_a, "index.js").unwrap();

        let dir_b = temp_registry("multi_pkg_b");
        fs::write(dir_b.join("index.js"), "module.exports = function () { return 'b'; };").unwrap();
        let (bundle_b, _, _) =
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
        assert_eq!(thaw_quickjs::thaw_js_load(source_a.as_ptr()), 1, "package A failed to load");

        // Loaded into the same shared global context *after* A.
        let source_b = CString::new(wrap(&bundle_b, "pkgB")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source_b.as_ptr()), 1, "package B failed to load");

        // Call A's lazily-requiring function *after* B has loaded.
        let func = CString::new("getLazy").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned();
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

        let (bundle, _, file_count) =
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

        let (bundle, _, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.getPlatform = module.exports.default;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1, "failed to load");

        let func = CString::new("getPlatform").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned();
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

        let (bundle, _, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.checkInspectCustom = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1, "bundle failed to load");

        let func = CString::new("checkInspectCustom").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned();
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

        let (bundle, _, file_count) =
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
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1, "bundle failed to load");

        let func = CString::new("locate").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned();
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
        let rewritten = rewrite_esm_to_commonjs("export default function greet() { return 'hi'; }").unwrap();
        assert!(rewritten.contains("module.exports.default = function greet() { return 'hi'; }"));
        assert!(rewritten.contains("module.exports.__esModule = true;"));
    }

    #[test]
    fn rewrites_named_export_and_binds_it_too() {
        let rewritten = rewrite_esm_to_commonjs("export function add(a, b) { return a + b; }").unwrap();
        assert!(rewritten.contains("function add(a, b) { return a + b; }"));
        assert!(rewritten.contains("exports.add = add;"));
    }

    #[test]
    fn rewrites_named_import_to_a_require_call() {
        let rewritten = rewrite_esm_to_commonjs("import { add } from './math';\nconsole.log(add(1, 2));").unwrap();
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
        let (bundle, _, file_count) =
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
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1, "ESM bundle failed to load");

        let func = CString::new("run").unwrap();
        let args = CString::new("[21]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned();
        assert_eq!(result, "42");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }
}
