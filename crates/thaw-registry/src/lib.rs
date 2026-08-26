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
//! been fetched/built by some other means. `add` selects already-bundled
//! `.node` prebuilds and `prebuild-install`-style GitHub Release assets
//! described by the package manifest; it never runs package install scripts
//! or `node-gyp`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

include!("registry/resolution.rs");

include!("registry/install.rs");

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
    static_esm_specs: Vec<String>,
    has_esm: bool,
    has_top_level_await: bool,
    async_module: bool,
}

fn declared_runtime_dependencies(package_dir: &Path) -> Vec<String> {
    let Ok(source) = fs::read_to_string(package_dir.join("package.json")) else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&source) else {
        return Vec::new();
    };
    let mut dependencies = Vec::new();
    for field in ["dependencies", "optionalDependencies", "peerDependencies"] {
        let Some(entries) = manifest.get(field).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for name in entries.keys() {
            if !dependencies.contains(name) {
                dependencies.push(name.clone());
            }
        }
    }
    dependencies
}

fn split_module_suffix(specifier: &str) -> (&str, &str) {
    specifier
        .char_indices()
        .find_map(|(index, character)| {
            (character == '?' || (character == '#' && index > 0)).then_some(index)
        })
        .map(|index| specifier.split_at(index))
        .unwrap_or((specifier, ""))
}

fn runtime_export_specifiers(
    package_name: &str,
    package_dir: &Path,
) -> Result<Vec<String>, String> {
    let Ok(source) = fs::read_to_string(package_dir.join("package.json")) else {
        return Ok(Vec::new());
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&source) else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    let mut specifiers = Vec::new();
    let Some(exports_value) = manifest.get("exports") else {
        collect_relative_files(package_dir, package_dir, &mut files)?;
        for file in files.iter().filter(|file| {
            !file.split('/').any(|component| component == "node_modules")
                && matches!(
                    Path::new(file)
                        .extension()
                        .and_then(|extension| extension.to_str()),
                    Some("js" | "cjs" | "mjs" | "json")
                )
        }) {
            specifiers.push(format!("{package_name}/{file}"));
            if let Some(extensionless) = file
                .strip_suffix(".js")
                .or_else(|| file.strip_suffix(".cjs"))
                .or_else(|| file.strip_suffix(".mjs"))
            {
                specifiers.push(format!("{package_name}/{extensionless}"));
            }
            if let Some(directory) = file
                .strip_suffix("/index.js")
                .or_else(|| file.strip_suffix("/index.cjs"))
                .or_else(|| file.strip_suffix("/index.mjs"))
                .or_else(|| file.strip_suffix("/index.json"))
            {
                specifiers.push(format!("{package_name}/{directory}"));
            }
        }
        specifiers.sort();
        specifiers.dedup();
        return Ok(specifiers);
    };
    let Some(exports) = exports_value.as_object() else {
        return Ok(Vec::new());
    };
    for (key, target) in exports {
        let Some(subpath) = key.strip_prefix("./") else {
            continue;
        };
        let Some(runtime) = select_export_condition(target, &["require", "import", "default"])
        else {
            continue;
        };
        if subpath.contains('*') {
            if files.is_empty() {
                collect_relative_files(package_dir, package_dir, &mut files)?;
            }
            for file in &files {
                if let Some(capture) = wildcard_capture(runtime, file) {
                    let expanded = subpath.replacen('*', capture, 1);
                    if validate_export_subpath(&expanded).is_ok() {
                        specifiers.push(format!("{package_name}/{expanded}"));
                    }
                }
            }
        } else if validate_export_subpath(subpath).is_ok() {
            specifiers.push(format!("{package_name}/{subpath}"));
        }
    }
    specifiers.sort();
    specifiers.dedup();
    Ok(specifiers)
}

fn rewrite_static_worker_urls(
    source: &str,
    module_path: &Path,
    package_name: &str,
    package_dir: &Path,
) -> Result<String, String> {
    use std::collections::{BTreeMap, BTreeSet};
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        BindingIdent, Callee, Decl, Expr, ExprOrSpread, ImportDecl, ImportSpecifier, Lit,
        MemberProp, ModuleItem, NewExpr, Pat, Prop, PropName, PropOrSpread, Stmt, VarDeclKind,
        VarDeclarator,
    };
    use thaw_parser::common::Spanned;

    #[derive(Default)]
    struct WorkerBindings {
        constructors: BTreeSet<String>,
        namespaces: BTreeSet<String>,
        path_namespaces: BTreeSet<String>,
    }
    impl Visit for WorkerBindings {
        fn visit_import_decl(&mut self, declaration: &ImportDecl) {
            if !matches!(
                declaration.src.value.as_str(),
                Some("worker_threads" | "node:worker_threads")
            ) {
                return;
            }
            for specifier in &declaration.specifiers {
                match specifier {
                    ImportSpecifier::Named(named)
                        if named
                            .imported
                            .as_ref()
                            .map(|name| match name {
                                thaw_parser::ast::ModuleExportName::Ident(name) => {
                                    name.sym.as_ref()
                                }
                                thaw_parser::ast::ModuleExportName::Str(name) => {
                                    name.value.as_str().unwrap_or("")
                                }
                            })
                            .unwrap_or(named.local.sym.as_ref())
                            == "Worker" =>
                    {
                        self.constructors.insert(named.local.sym.to_string());
                    }
                    ImportSpecifier::Namespace(namespace) => {
                        self.namespaces.insert(namespace.local.sym.to_string());
                    }
                    _ => {}
                }
            }
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            let Some(initializer) = declaration.init.as_deref() else {
                return;
            };
            let (call, property) = match initializer {
                Expr::Call(call) => (call, None),
                Expr::Member(member) => {
                    let Expr::Call(call) = member.obj.as_ref() else {
                        return;
                    };
                    let property = match &member.prop {
                        MemberProp::Ident(property) => Some(property.sym.as_ref()),
                        _ => None,
                    };
                    (call, property)
                }
                _ => return,
            };
            let required_module = match call.args.first().map(|argument| argument.expr.as_ref()) {
                Some(Expr::Lit(Lit::Str(value))) => value.value.as_str(),
                _ => None,
            };
            let is_require = matches!(
                &call.callee,
                Callee::Expr(callee) if matches!(callee.as_ref(), Expr::Ident(name) if name.sym == "require")
            );
            if is_require
                && matches!(required_module, Some("path" | "node:path"))
                && property.is_none()
            {
                if let Pat::Ident(binding) = &declaration.name {
                    self.path_namespaces.insert(binding.id.sym.to_string());
                }
                return;
            }
            let is_worker_threads = is_require
                && matches!(
                    required_module,
                    Some("worker_threads" | "node:worker_threads")
                );
            if !is_worker_threads {
                return;
            }
            let Pat::Ident(binding) = &declaration.name else {
                return;
            };
            if property == Some("Worker") {
                self.constructors.insert(binding.id.sym.to_string());
            } else if property.is_none() {
                self.namespaces.insert(binding.id.sym.to_string());
            }
        }
    }

    struct WorkerUrlSpan {
        lo: u32,
        hi: u32,
        import_meta_base: Option<(u32, u32)>,
        relative: String,
    }

    fn static_file_options(arguments: &[ExprOrSpread]) -> bool {
        let Some(options) = arguments.get(1) else {
            return true;
        };
        if matches!(options.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == "undefined")
        {
            return true;
        }
        let Expr::Object(options) = options.expr.as_ref() else {
            return false;
        };
        for property in &options.props {
            let PropOrSpread::Prop(property) = property else {
                return false;
            };
            let Prop::KeyValue(property) = property.as_ref() else {
                continue;
            };
            let is_eval = matches!(&property.key, PropName::Ident(name) if name.sym == "eval")
                || matches!(&property.key, PropName::Str(name) if name.value.as_str() == Some("eval"));
            if is_eval {
                return matches!(property.value.as_ref(), Expr::Lit(Lit::Bool(value)) if !value.value);
            }
        }
        true
    }

    fn static_worker_path(
        expression: &Expr,
        constants: &BTreeMap<String, String>,
        path_namespaces: &BTreeSet<String>,
    ) -> Option<String> {
        match expression {
            Expr::Lit(Lit::Str(path)) => Some(path.value.to_string_lossy().into_owned()),
            Expr::Ident(identifier) => constants.get(identifier.sym.as_ref()).cloned(),
            Expr::Tpl(template) if template.quasis.len() == template.exprs.len() + 1 => {
                let mut path = String::new();
                for (index, quasi) in template.quasis.iter().enumerate() {
                    path.push_str(
                        &quasi
                            .cooked
                            .as_ref()
                            .map(|value| value.to_string_lossy().into_owned())
                            .unwrap_or_else(|| quasi.raw.to_string()),
                    );
                    if let Some(expression) = template.exprs.get(index) {
                        path.push_str(&static_worker_path(expression, constants, path_namespaces)?);
                    }
                }
                Some(path)
            }
            Expr::Paren(parenthesized) => {
                static_worker_path(&parenthesized.expr, constants, path_namespaces)
            }
            Expr::Bin(binary) if binary.op == thaw_parser::ast::BinaryOp::Add => Some(format!(
                "{}{}",
                static_worker_path(&binary.left, constants, path_namespaces)?,
                static_worker_path(&binary.right, constants, path_namespaces)?
            )),
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else {
                    return None;
                };
                let Expr::Member(member) = callee.as_ref() else {
                    return None;
                };
                let Expr::Ident(namespace) = member.obj.as_ref() else {
                    return None;
                };
                let MemberProp::Ident(operation) = &member.prop else {
                    return None;
                };
                if !path_namespaces.contains(namespace.sym.as_ref())
                    || !matches!(operation.sym.as_ref(), "join" | "resolve")
                {
                    return None;
                }
                let mut parts = Vec::new();
                for (index, argument) in call.args.iter().enumerate() {
                    if index == 0
                        && matches!(argument.expr.as_ref(), Expr::Ident(name) if name.sym == "__dirname")
                    {
                        continue;
                    }
                    parts.push(static_worker_path(
                        &argument.expr,
                        constants,
                        path_namespaces,
                    )?);
                }
                let normalized = normalize_path_string(&parts.join("/"));
                (!normalized.is_empty()).then(|| format!("./{normalized}"))
            }
            _ => None,
        }
    }

    fn top_level_worker_path_constants(
        module: &thaw_parser::ast::Module,
        binding_counts: &BTreeMap<String, usize>,
        path_namespaces: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        let mut constants = BTreeMap::new();
        for item in &module.body {
            let ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration))) = item else {
                continue;
            };
            if declaration.kind != VarDeclKind::Const {
                continue;
            }
            for declarator in &declaration.decls {
                let (Pat::Ident(binding), Some(initializer)) =
                    (&declarator.name, declarator.init.as_deref())
                else {
                    continue;
                };
                if binding_counts.get(binding.id.sym.as_ref()) != Some(&1) {
                    continue;
                }
                if let Some(path) = static_worker_path(initializer, &constants, path_namespaces) {
                    constants.insert(binding.id.sym.to_string(), path);
                }
            }
        }
        constants
    }

    #[derive(Default)]
    struct BindingCounts(BTreeMap<String, usize>);
    impl Visit for BindingCounts {
        fn visit_binding_ident(&mut self, binding: &BindingIdent) {
            *self.0.entry(binding.id.sym.to_string()).or_default() += 1;
        }
    }

    struct WorkerUrls {
        constructors: BTreeSet<String>,
        namespaces: BTreeSet<String>,
        path_constants: BTreeMap<String, String>,
        path_namespaces: BTreeSet<String>,
        spans: Vec<WorkerUrlSpan>,
    }
    impl Visit for WorkerUrls {
        fn visit_new_expr(&mut self, expression: &NewExpr) {
            let is_worker = match expression.callee.as_ref() {
                Expr::Ident(callee) => self.constructors.contains(callee.sym.as_ref()),
                Expr::Member(member) => {
                    matches!(member.obj.as_ref(), Expr::Ident(namespace) if self.namespaces.contains(namespace.sym.as_ref()))
                        && matches!(&member.prop, MemberProp::Ident(property) if property.sym == "Worker")
                }
                _ => false,
            };
            if !is_worker {
                expression.visit_children_with(self);
                return;
            }
            let Some(arguments) = expression.args.as_ref() else {
                return;
            };
            let Some(first) = arguments.first() else {
                return;
            };
            if let Some(path) =
                static_worker_path(&first.expr, &self.path_constants, &self.path_namespaces)
            {
                if !static_file_options(arguments) {
                    expression.visit_children_with(self);
                    return;
                }
                let span = first.expr.span();
                self.spans.push(WorkerUrlSpan {
                    lo: span.lo.0,
                    hi: span.hi.0,
                    import_meta_base: None,
                    relative: path,
                });
                expression.visit_children_with(self);
                return;
            }
            let Expr::New(url) = first.expr.as_ref() else {
                expression.visit_children_with(self);
                return;
            };
            let Expr::Ident(url_callee) = url.callee.as_ref() else {
                expression.visit_children_with(self);
                return;
            };
            let Some(url_arguments) = url.args.as_ref() else {
                return;
            };
            if url_callee.sym != "URL" || url_arguments.len() != 2 {
                expression.visit_children_with(self);
                return;
            }
            let Expr::Lit(Lit::Str(path)) = url_arguments[0].expr.as_ref() else {
                expression.visit_children_with(self);
                return;
            };
            let span = first.expr.span();
            let base_span = url_arguments[1].expr.span();
            self.spans.push(WorkerUrlSpan {
                lo: span.lo.0,
                hi: span.hi.0,
                import_meta_base: Some((base_span.lo.0, base_span.hi.0)),
                relative: path.value.to_string_lossy().into_owned(),
            });
            expression.visit_children_with(self);
        }
    }

    let Ok((module, source_map)) = thaw_parser::parse_javascript_with_source_map(source) else {
        return Ok(source.to_string());
    };
    let mut bindings = WorkerBindings::default();
    module.visit_with(&mut bindings);
    let mut binding_counts = BindingCounts::default();
    module.visit_with(&mut binding_counts);
    let path_constants =
        top_level_worker_path_constants(&module, &binding_counts.0, &bindings.path_namespaces);
    let mut workers = WorkerUrls {
        constructors: bindings.constructors,
        namespaces: bindings.namespaces,
        path_constants,
        path_namespaces: bindings.path_namespaces,
        spans: Vec::new(),
    };
    module.visit_with(&mut workers);
    if workers.spans.is_empty() {
        return Ok(source.to_string());
    }
    workers.spans.sort_by_key(|span| span.lo);
    let directory = module_path.parent().unwrap_or(Path::new(""));
    let mut output = String::with_capacity(source.len());
    let mut worker_requires = Vec::new();
    let mut cursor = 0usize;
    for worker in workers.spans {
        let WorkerUrlSpan {
            lo,
            hi,
            import_meta_base,
            relative,
        } = worker;
        if !(relative.starts_with("./") || relative.starts_with("../")) {
            continue;
        }
        if let Some((base_lo, base_hi)) = import_meta_base {
            let base_lo = source_map
                .lookup_byte_offset(thaw_parser::common::BytePos(base_lo))
                .pos
                .0 as usize;
            let base_hi = source_map
                .lookup_byte_offset(thaw_parser::common::BytePos(base_hi))
                .pos
                .0 as usize;
            if source[base_lo..base_hi].trim() != "import.meta.url" {
                continue;
            }
        }
        let worker_path = directory.join(&relative);
        let worker_source = fs::read_to_string(&worker_path).map_err(|error| {
            format!(
                "failed to read Worker source `{}` referenced by `{}`: {error}",
                worker_path.display(),
                module_path.display()
            )
        })?;
        let worker_relative = worker_path.strip_prefix(package_dir).map_err(|_| {
            format!(
                "Worker source `{}` is outside package `{}`",
                worker_path.display(),
                package_dir.display()
            )
        })?;
        let worker_relative = normalize_path_string(&worker_relative.to_string_lossy());
        let worker_key = format!("{package_name}/{worker_relative}");
        let worker_bootstrap = format!(
            "var __thaw_worker_require = globalThis.__thaw_bundle_create_require({});\nvar require = function(name) {{ return name === 'worker_threads' || name === 'node:worker_threads' ? globalThis.__thaw_worker_module : __thaw_worker_require(name); }};\n",
            js_string_literal(&worker_key)
        );
        let worker_source =
            rewrite_esm_to_commonjs_mode(&worker_source, false).unwrap_or(worker_source);
        let encoded = worker_bootstrap
            .bytes()
            .chain(worker_source.bytes())
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>();
        let replacement = serde_json::to_string(&format!("data:text/javascript,{encoded}"))
            .expect("Worker data URL is serializable");
        let lo = source_map
            .lookup_byte_offset(thaw_parser::common::BytePos(lo))
            .pos
            .0 as usize;
        let hi = source_map
            .lookup_byte_offset(thaw_parser::common::BytePos(hi))
            .pos
            .0 as usize;
        if lo < cursor {
            continue;
        }
        output.push_str(&source[cursor..lo]);
        output.push_str(&replacement);
        cursor = hi;
        if !worker_requires.contains(&relative) {
            worker_requires.push(relative);
        }
    }
    output.push_str(&source[cursor..]);
    for relative in worker_requires {
        output.push_str(&format!(
            "\nif (false) require({});",
            js_string_literal(&relative)
        ));
    }
    Ok(output)
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
    let mut uses_global_fetch = false;
    record_package_version(&mut dependency_versions, root_package, root_package_dir);

    while let Some((key, abs_path, pkg_name, pkg_dir)) = worklist.pop() {
        let source = fs::read_to_string(&abs_path)
            .map_err(|e| format!("failed to read `{key}` while bundling: {e}"))?;
        let source = if abs_path.extension().is_some_and(|ext| ext == "json") {
            let value: serde_json::Value = serde_json::from_str(&source)
                .map_err(|error| format!("invalid JSON module `{key}`: {error}"))?;
            format!("module.exports = {};", value)
        } else {
            source
        };
        let source = rewrite_static_worker_urls(&source, &abs_path, &pkg_name, &pkg_dir)?;
        let analysis = analyze_module(&source);
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

        if analysis.has_nonliteral_dynamic_import {
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
            .filter(|spec| spec.starts_with("./") || spec.starts_with("../"))
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
            .filter(|spec| !(spec.starts_with("./") || spec.starts_with("../")))
            .cloned()
        {
            let (resolution_spec, suffix) = split_module_suffix(&spec);
            if resolution_spec.starts_with('#') {
                if let Some((resolved_relative, resolved_abs)) =
                    resolve_package_import(&pkg_dir, resolution_spec)
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

/// Parses a JavaScript module and collects its statically knowable dependency
/// edges. Direct `require`/`import` arguments are constant-folded when they
/// consist solely of string literals, expression-free templates, parentheses,
/// and string concatenation; runtime expressions remain dynamic. Shadowed or
/// member calls, comments, and strings are not mistaken for edges.
/// ESM declarations are collected directly from the module AST before they
/// are lowered to CommonJS.
#[derive(Default)]
struct ModuleAnalysis {
    specs: Vec<String>,
    static_esm_specs: Vec<String>,
    has_esm: bool,
    has_top_level_await: bool,
    attribute_error: Option<String>,
    has_nonliteral_dynamic_import: bool,
    uses_global_fetch: bool,
    _commonjs_exports: Vec<String>,
}

fn analyze_module(source: &str) -> ModuleAnalysis {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        ArrowExpr, AssignExpr, AssignTarget, AwaitExpr, CallExpr, Callee, Expr, Function, Ident,
        ImportSpecifier, Lit, MemberExpr, MemberProp, ModuleDecl, ModuleExportName, ModuleItem,
        ObjectLit, Pat, Prop, PropName, PropOrSpread, SimpleAssignTarget, VarDeclarator,
    };

    fn validate_attributes(source: &str, attributes: Option<&ObjectLit>) -> Result<(), String> {
        let Some(attributes) = attributes else {
            return Ok(());
        };
        let mut json = false;
        for property in &attributes.props {
            let PropOrSpread::Prop(property) = property else {
                return Err("spread import attributes are not supported".to_string());
            };
            let Prop::KeyValue(property) = property.as_ref() else {
                return Err("only key/value import attributes are supported".to_string());
            };
            let key = match &property.key {
                PropName::Ident(identifier) => identifier.sym.to_string(),
                PropName::Str(value) => value.value.to_string_lossy().into_owned(),
                _ => String::new(),
            };
            let Expr::Lit(Lit::Str(value)) = property.value.as_ref() else {
                return Err("import attribute values must be strings".to_string());
            };
            if key == "type" && value.value.to_string_lossy() == "json" {
                json = true;
            } else {
                return Err(format!(
                    "unsupported import attribute `{key}` for `{source}`"
                ));
            }
        }
        let source_path = source.split(['?', '#']).next().unwrap_or(source);
        if !json || !source_path.ends_with(".json") {
            return Err(format!(
                "only JSON modules accept `type: json` import attributes (`{source}`)"
            ));
        }
        Ok(())
    }

    struct Calls {
        specs: Vec<String>,
        commonjs_exports: Vec<String>,
        has_nonliteral_dynamic_import: bool,
        attribute_error: Option<String>,
        require_functions: Vec<String>,
        create_require_functions: Vec<String>,
        module_namespaces: Vec<String>,
        uses_global_fetch: bool,
    }

    struct TopLevelAwait {
        found: bool,
    }

    const MAX_STATIC_SPECIFIER_CANDIDATES: usize = 64;

    fn combine_specifier_parts(left: Vec<String>, right: Vec<String>) -> Option<Vec<String>> {
        if left.len().saturating_mul(right.len()) > MAX_STATIC_SPECIFIER_CANDIDATES {
            return None;
        }
        let mut combined = Vec::new();
        for left in left {
            for right in &right {
                let value = format!("{left}{right}");
                if !combined.contains(&value) {
                    combined.push(value);
                }
            }
        }
        Some(combined)
    }

    fn static_module_specifiers(expr: &Expr) -> Option<Vec<String>> {
        match expr {
            Expr::Lit(Lit::Str(specifier)) => {
                Some(vec![specifier.value.to_string_lossy().into_owned()])
            }
            Expr::Tpl(template) if template.exprs.is_empty() && template.quasis.len() == 1 => {
                template.quasis[0]
                    .cooked
                    .as_ref()
                    .map(|value| value.to_string_lossy().into_owned())
                    .or_else(|| Some(template.quasis[0].raw.to_string()))
                    .map(|value| vec![value])
            }
            Expr::Tpl(template) if template.quasis.len() == template.exprs.len() + 1 => {
                let mut values = vec![String::new()];
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let text = quasi
                        .cooked
                        .as_ref()
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_else(|| quasi.raw.to_string());
                    values = combine_specifier_parts(values, vec![text])?;
                    if let Some(expr) = template.exprs.get(index) {
                        values = combine_specifier_parts(values, static_module_specifiers(expr)?)?;
                    }
                }
                Some(values)
            }
            Expr::Paren(parenthesized) => static_module_specifiers(&parenthesized.expr),
            Expr::Bin(binary) if binary.op == thaw_parser::ast::BinaryOp::Add => {
                combine_specifier_parts(
                    static_module_specifiers(&binary.left)?,
                    static_module_specifiers(&binary.right)?,
                )
            }
            Expr::Cond(conditional) => {
                let mut values = static_module_specifiers(&conditional.cons)?;
                for value in static_module_specifiers(&conditional.alt)? {
                    if !values.contains(&value) {
                        values.push(value);
                    }
                }
                (values.len() <= MAX_STATIC_SPECIFIER_CANDIDATES).then_some(values)
            }
            _ => None,
        }
    }

    fn dynamic_import_attributes(call: &CallExpr) -> Result<Option<&ObjectLit>, String> {
        if call.args.len() == 1 {
            return Ok(None);
        }
        if call.args.len() != 2 || call.args[1].spread.is_some() {
            return Err("dynamic import accepts one options object".to_string());
        }
        let Expr::Object(options) = call.args[1].expr.as_ref() else {
            return Err("dynamic import options must be an object literal".to_string());
        };
        let mut attributes = None;
        for property in &options.props {
            let PropOrSpread::Prop(property) = property else {
                return Err("spread dynamic import options are not supported".to_string());
            };
            let Prop::KeyValue(property) = property.as_ref() else {
                return Err("dynamic import options must be key/value properties".to_string());
            };
            let key = match &property.key {
                PropName::Ident(identifier) => identifier.sym.as_ref(),
                PropName::Str(value) => value.value.as_str().unwrap_or(""),
                _ => "",
            };
            if !matches!(key, "with" | "assert") {
                return Err(format!("unsupported dynamic import option `{key}`"));
            }
            if attributes.is_some() {
                return Err("dynamic import has duplicate attribute options".to_string());
            }
            let Expr::Object(object) = property.value.as_ref() else {
                return Err("dynamic import attributes must be an object literal".to_string());
            };
            attributes = Some(object);
        }
        Ok(attributes)
    }
    impl Visit for TopLevelAwait {
        fn visit_await_expr(&mut self, _: &AwaitExpr) {
            self.found = true;
        }
        fn visit_function(&mut self, _: &Function) {}
        fn visit_arrow_expr(&mut self, _: &ArrowExpr) {}
    }
    impl Visit for Calls {
        fn visit_ident(&mut self, identifier: &Ident) {
            if identifier.sym == "fetch" {
                self.uses_global_fetch = true;
            }
        }

        fn visit_call_expr(&mut self, call: &CallExpr) {
            if matches!(
                &call.callee,
                Callee::Expr(callee)
                    if matches!(callee.as_ref(), Expr::Ident(ident) if ident.sym == "fetch")
            ) {
                self.uses_global_fetch = true;
            }
            let is_require = matches!(
                &call.callee,
                Callee::Expr(callee)
                    if matches!(callee.as_ref(), Expr::Ident(ident) if self.require_functions.iter().any(|name| name == ident.sym.as_ref()))
            );
            let is_import = matches!(&call.callee, Callee::Import(_));
            if ((is_require && call.args.len() == 1) || (is_import && !call.args.is_empty()))
                && call.args[0].spread.is_none()
            {
                let specifiers = static_module_specifiers(&call.args[0].expr);
                if is_import && self.attribute_error.is_none() {
                    self.attribute_error = match dynamic_import_attributes(call) {
                        Ok(Some(attributes)) => match &specifiers {
                            Some(specifiers) => specifiers.iter().find_map(|specifier| {
                                validate_attributes(specifier, Some(attributes)).err()
                            }),
                            None => Some(
                                "attributed dynamic imports require a finite static specifier set"
                                    .to_string(),
                            ),
                        },
                        Ok(None) => None,
                        Err(error) => Some(error),
                    };
                }
                if let Some(specifiers) = specifiers {
                    self.specs.extend(specifiers);
                } else if is_import {
                    self.has_nonliteral_dynamic_import = true;
                }
            }
            call.visit_children_with(self);
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            let Pat::Ident(binding) = &declaration.name else {
                declaration.visit_children_with(self);
                return;
            };
            let Some(Expr::Call(call)) = declaration.init.as_deref() else {
                declaration.visit_children_with(self);
                return;
            };
            let creates_require = match &call.callee {
                Callee::Expr(callee) => match callee.as_ref() {
                    Expr::Ident(identifier) => self
                        .create_require_functions
                        .iter()
                        .any(|name| name == identifier.sym.as_ref()),
                    Expr::Member(member) => {
                        matches!(member.obj.as_ref(), Expr::Ident(identifier)
                            if self.module_namespaces.iter().any(|name| name == identifier.sym.as_ref()))
                            && property_name(&member.prop).as_deref() == Some("createRequire")
                    }
                    _ => false,
                },
                _ => false,
            };
            if creates_require {
                let name = binding.id.sym.to_string();
                if !self.require_functions.contains(&name) {
                    self.require_functions.push(name);
                }
            }
            declaration.visit_children_with(self);
        }

        fn visit_assign_expr(&mut self, assignment: &AssignExpr) {
            let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assignment.left else {
                assignment.visit_children_with(self);
                return;
            };
            if let Some(name) = commonjs_export_name(member) {
                self.commonjs_exports.push(name);
            }
            assignment.visit_children_with(self);
        }
    }

    fn property_name(property: &MemberProp) -> Option<String> {
        match property {
            MemberProp::Ident(ident) => Some(ident.sym.to_string()),
            MemberProp::Computed(computed) => match computed.expr.as_ref() {
                Expr::Lit(Lit::Str(value)) => Some(value.value.to_string_lossy().into_owned()),
                _ => None,
            },
            MemberProp::PrivateName(_) => None,
        }
    }

    fn is_module_exports(member: &MemberExpr) -> bool {
        matches!(member.obj.as_ref(), Expr::Ident(module) if module.sym == "module")
            && property_name(&member.prop).as_deref() == Some("exports")
    }

    fn commonjs_export_name(member: &MemberExpr) -> Option<String> {
        if matches!(member.obj.as_ref(), Expr::Ident(exports) if exports.sym == "exports") {
            return property_name(&member.prop);
        }
        if is_module_exports(member) {
            return Some("default".to_string());
        }
        if let Expr::Member(object) = member.obj.as_ref() {
            if is_module_exports(object) {
                return property_name(&member.prop);
            }
        }
        None
    }

    let Ok(module) = thaw_parser::parse_javascript(source) else {
        return ModuleAnalysis::default();
    };
    let mut create_require_functions = Vec::new();
    let mut module_namespaces = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
            continue;
        };
        if !matches!(import.src.value.as_str(), Some("module" | "node:module")) {
            continue;
        }
        for specifier in &import.specifiers {
            match specifier {
                ImportSpecifier::Named(named) => {
                    let imported = named.imported.as_ref().map_or_else(
                        || named.local.sym.to_string(),
                        |name| match name {
                            ModuleExportName::Ident(identifier) => identifier.sym.to_string(),
                            ModuleExportName::Str(value) => {
                                value.value.to_string_lossy().into_owned()
                            }
                        },
                    );
                    if imported == "createRequire" {
                        create_require_functions.push(named.local.sym.to_string());
                    }
                }
                ImportSpecifier::Namespace(namespace) => {
                    module_namespaces.push(namespace.local.sym.to_string());
                }
                ImportSpecifier::Default(default) => {
                    module_namespaces.push(default.local.sym.to_string());
                }
            }
        }
    }
    let mut calls = Calls {
        specs: Vec::new(),
        commonjs_exports: Vec::new(),
        has_nonliteral_dynamic_import: false,
        attribute_error: None,
        require_functions: vec!["require".to_string()],
        create_require_functions,
        module_namespaces,
        uses_global_fetch: false,
    };
    module.visit_with(&mut calls);
    let mut top_level_await = TopLevelAwait { found: false };
    module.visit_with(&mut top_level_await);
    let mut static_esm_specs = Vec::new();
    let mut attribute_error = calls.attribute_error.take();
    for item in &module.body {
        let (source, attributes) = match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(decl)) => {
                (Some(&decl.src), decl.with.as_deref())
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(decl)) => {
                (decl.src.as_ref(), decl.with.as_deref())
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(decl)) => {
                (Some(&decl.src), decl.with.as_deref())
            }
            _ => (None, None),
        };
        if let Some(source) = source {
            let spec = source.value.to_string_lossy().into_owned();
            if attribute_error.is_none() {
                attribute_error = validate_attributes(&spec, attributes).err();
            }
            calls.specs.push(spec.clone());
            static_esm_specs.push(spec);
        }
    }
    let mut unique = Vec::new();
    for spec in calls.specs {
        if !unique.contains(&spec) {
            unique.push(spec);
        }
    }
    calls.commonjs_exports.sort();
    calls.commonjs_exports.dedup();
    ModuleAnalysis {
        specs: unique,
        static_esm_specs,
        has_esm: module
            .body
            .iter()
            .any(|item| matches!(item, ModuleItem::ModuleDecl(_))),
        has_top_level_await: top_level_await.found,
        attribute_error,
        has_nonliteral_dynamic_import: calls.has_nonliteral_dynamic_import,
        uses_global_fetch: calls.uses_global_fetch,
        _commonjs_exports: calls.commonjs_exports,
    }
}

#[cfg(test)]
fn find_module_specs(source: &str) -> Vec<String> {
    analyze_module(source).specs
}

/// Converts literal dynamic imports to an asynchronous call through the
/// bundle's per-module `require` map. The `then` boundary ensures a missing or
/// throwing module rejects the returned Promise instead of throwing before a
/// Promise is returned.
fn rewrite_dynamic_imports(source: &str) -> Option<String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{CallExpr, Callee};
    use thaw_parser::common::Spanned;

    struct Imports {
        spans: Vec<(u32, u32, u32, u32)>,
    }
    impl Visit for Imports {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            if matches!(&call.callee, Callee::Import(_))
                && !call.args.is_empty()
                && call.args[0].spread.is_none()
            {
                let span = call.span();
                let argument = call.args[0].expr.span();
                self.spans
                    .push((span.lo.0, span.hi.0, argument.lo.0, argument.hi.0));
            }
            call.visit_children_with(self);
        }
    }

    let (module, cm) = thaw_parser::parse_javascript_with_source_map(source).ok()?;
    let mut imports = Imports { spans: Vec::new() };
    module.visit_with(&mut imports);
    if imports.spans.is_empty() {
        return None;
    }
    imports.spans.sort_by_key(|(lo, _, _, _)| *lo);
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (lo, hi, argument_lo, argument_hi) in imports.spans {
        let lo = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(lo))
            .pos
            .0 as usize;
        let hi = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(hi))
            .pos
            .0 as usize;
        let argument_lo = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(argument_lo))
            .pos
            .0 as usize;
        let argument_hi = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(argument_hi))
            .pos
            .0 as usize;
        output.push_str(&source[cursor..lo]);
        output.push_str("requireAsync(String(");
        output.push_str(&source[argument_lo..argument_hi]);
        output.push_str("))");
        cursor = hi;
    }
    output.push_str(&source[cursor..]);
    Some(output)
}

fn rewrite_live_import_references(source: &str) -> Option<String> {
    use std::collections::{BTreeMap, BTreeSet};
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        ArrowExpr, BlockStmt, CatchClause, Decl, Expr, Function, ImportSpecifier, ModuleDecl,
        ModuleExportName, ModuleItem, Pat, Prop, Stmt,
    };
    use thaw_parser::common::Spanned;

    fn pattern_names(pattern: &Pat, names: &mut BTreeSet<String>) {
        match pattern {
            Pat::Ident(binding) => {
                names.insert(binding.id.sym.to_string());
            }
            Pat::Array(array) => {
                for element in array.elems.iter().flatten() {
                    pattern_names(element, names);
                }
            }
            Pat::Object(object) => {
                for property in &object.props {
                    match property {
                        thaw_parser::ast::ObjectPatProp::KeyValue(property) => {
                            pattern_names(&property.value, names);
                        }
                        thaw_parser::ast::ObjectPatProp::Assign(property) => {
                            names.insert(property.key.sym.to_string());
                        }
                        thaw_parser::ast::ObjectPatProp::Rest(property) => {
                            pattern_names(&property.arg, names);
                        }
                    }
                }
            }
            Pat::Assign(assign) => pattern_names(&assign.left, names),
            Pat::Rest(rest) => pattern_names(&rest.arg, names),
            Pat::Expr(_) | Pat::Invalid(_) => {}
        }
    }

    fn direct_block_bindings(block: &BlockStmt) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for statement in &block.stmts {
            if let Stmt::Decl(declaration) = statement {
                match declaration {
                    Decl::Var(variable) => {
                        for declarator in &variable.decls {
                            pattern_names(&declarator.name, &mut names);
                        }
                    }
                    Decl::Fn(function) => {
                        names.insert(function.ident.sym.to_string());
                    }
                    Decl::Class(class) => {
                        names.insert(class.ident.sym.to_string());
                    }
                    _ => {}
                }
            }
        }
        names
    }

    let (module, cm) = thaw_parser::parse_javascript_with_source_map(source).ok()?;
    let export_name = |name: &ModuleExportName| match name {
        ModuleExportName::Ident(identifier) => identifier.sym.to_string(),
        ModuleExportName::Str(value) => value.value.to_string_lossy().into_owned(),
    };
    let mut bindings = BTreeMap::<String, String>::new();
    let mut synthetic_count = 0usize;
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                let module_name = format!("__thaw_esm_import_{synthetic_count}");
                synthetic_count += 1;
                for specifier in &import.specifiers {
                    match specifier {
                        ImportSpecifier::Named(named) => {
                            let local = named.local.sym.to_string();
                            let imported = named
                                .imported
                                .as_ref()
                                .map(&export_name)
                                .unwrap_or_else(|| local.clone());
                            bindings.insert(
                                local,
                                format!("{module_name}[{}]", js_string_literal(&imported)),
                            );
                        }
                        ImportSpecifier::Default(default) => {
                            bindings.insert(
                                default.local.sym.to_string(),
                                format!(
                                    "(({module_name} && {module_name}.__esModule) ? {module_name}.default : {module_name})"
                                ),
                            );
                        }
                        ImportSpecifier::Namespace(_) => {}
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) if export.src.is_some() => {
                synthetic_count += 1;
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(_)) => synthetic_count += 1,
            _ => {}
        }
    }
    if bindings.is_empty() {
        return None;
    }

    struct References<'a> {
        bindings: &'a BTreeMap<String, String>,
        shadowed: Vec<BTreeSet<String>>,
        replacements: Vec<(u32, u32, String)>,
    }
    impl References<'_> {
        fn is_shadowed(&self, name: &str) -> bool {
            self.shadowed.iter().rev().any(|scope| scope.contains(name))
        }
        fn push_function_scope(&mut self, function: &Function) {
            let mut names = BTreeSet::new();
            for parameter in &function.params {
                pattern_names(&parameter.pat, &mut names);
            }
            self.shadowed.push(names);
            function.decorators.visit_with(self);
            function.body.visit_with(self);
            self.shadowed.pop();
        }
    }
    impl Visit for References<'_> {
        fn visit_function(&mut self, function: &Function) {
            self.push_function_scope(function);
        }

        fn visit_arrow_expr(&mut self, arrow: &ArrowExpr) {
            let mut names = BTreeSet::new();
            for parameter in &arrow.params {
                pattern_names(parameter, &mut names);
            }
            self.shadowed.push(names);
            arrow.body.visit_with(self);
            self.shadowed.pop();
        }

        fn visit_block_stmt(&mut self, block: &BlockStmt) {
            self.shadowed.push(direct_block_bindings(block));
            block.stmts.visit_with(self);
            self.shadowed.pop();
        }

        fn visit_catch_clause(&mut self, clause: &CatchClause) {
            let mut names = BTreeSet::new();
            if let Some(parameter) = &clause.param {
                pattern_names(parameter, &mut names);
            }
            self.shadowed.push(names);
            clause.body.visit_with(self);
            self.shadowed.pop();
        }

        fn visit_expr(&mut self, expression: &Expr) {
            if let Expr::Ident(identifier) = expression {
                let name = identifier.sym.as_str();
                if !self.is_shadowed(name) {
                    if let Some(replacement) = self.bindings.get(name) {
                        let span = identifier.span();
                        self.replacements
                            .push((span.lo.0, span.hi.0, replacement.clone()));
                        return;
                    }
                }
            }
            expression.visit_children_with(self);
        }

        fn visit_prop(&mut self, property: &Prop) {
            if let Prop::Shorthand(identifier) = property {
                let name = identifier.sym.as_str();
                if !self.is_shadowed(name) {
                    if let Some(replacement) = self.bindings.get(name) {
                        let span = identifier.span();
                        self.replacements.push((
                            span.lo.0,
                            span.hi.0,
                            format!("{name}: {replacement}"),
                        ));
                        return;
                    }
                }
            }
            property.visit_children_with(self);
        }
    }

    let mut references = References {
        bindings: &bindings,
        shadowed: vec![BTreeSet::new()],
        replacements: Vec::new(),
    };
    for item in &module.body {
        if !matches!(item, ModuleItem::ModuleDecl(ModuleDecl::Import(_))) {
            item.visit_with(&mut references);
        }
    }
    if references.replacements.is_empty() {
        return None;
    }
    references.replacements.sort_by_key(|(lo, _, _)| *lo);
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (lo, hi, replacement) in references.replacements {
        let lo = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(lo))
            .pos
            .0 as usize;
        let hi = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(hi))
            .pos
            .0 as usize;
        if lo < cursor {
            continue;
        }
        output.push_str(&source[cursor..lo]);
        output.push_str(&replacement);
        cursor = hi;
    }
    output.push_str(&source[cursor..]);
    Some(output)
}

/// Rewrites ESM (`import`/`export`) syntax to the CommonJS shape the rest
/// of this bundler's require-graph resolution already understands:
/// the parser-backed dependency walk recognizes the synthesized
/// `require(...)` calls alongside original CommonJS calls, so deep imports,
/// builtins, and cross-package resolution share one graph.
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
#[cfg(test)]
fn rewrite_esm_to_commonjs(source: &str) -> Option<String> {
    rewrite_esm_to_commonjs_mode(source, false)
}

fn rewrite_esm_to_commonjs_mode(source: &str, await_imports: bool) -> Option<String> {
    use thaw_parser::ast::{
        Decl, DefaultDecl, ExportSpecifier, ImportSpecifier, ModuleDecl, ModuleExportName,
        ModuleItem, Pat,
    };
    use thaw_parser::common::{SourceMapper, Spanned};

    let dynamic_source = rewrite_dynamic_imports(source);
    let source = dynamic_source.as_deref().unwrap_or(source);
    let live_source = rewrite_live_import_references(source);
    let source = live_source.as_deref().unwrap_or(source);
    let (module, cm) = thaw_parser::parse_javascript_with_source_map(source).ok()?;
    let has_esm_syntax = module
        .body
        .iter()
        .any(|item| matches!(item, ModuleItem::ModuleDecl(_)));
    if !has_esm_syntax {
        return live_source.or(dynamic_source);
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

    let mut prologue = String::new();
    let mut local_export_prologue = String::new();
    let mut rest = String::new();
    let mut synthetic_count = 0usize;
    let mut imported_bindings = BTreeMap::<String, String>::new();
    let mut binding_counter = 0usize;
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                let module_name = format!("__thaw_esm_import_{binding_counter}");
                binding_counter += 1;
                for specifier in &import.specifiers {
                    match specifier {
                        ImportSpecifier::Named(named) => {
                            let local = named.local.sym.to_string();
                            let imported = named
                                .imported
                                .as_ref()
                                .map(&export_name)
                                .unwrap_or_else(|| local.clone());
                            imported_bindings.insert(
                                local,
                                format!("{module_name}[{}]", js_string_literal(&imported)),
                            );
                        }
                        ImportSpecifier::Default(default) => {
                            imported_bindings.insert(
                                default.local.sym.to_string(),
                                format!(
                                    "(({module_name} && {module_name}.__esModule) ? {module_name}.default : {module_name})"
                                ),
                            );
                        }
                        ImportSpecifier::Namespace(_) => {}
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) if export.src.is_some() => {
                binding_counter += 1;
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(_)) => binding_counter += 1,
            _ => {}
        }
    }

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
                let loader = if await_imports {
                    "await requireAsync"
                } else {
                    "require"
                };
                prologue.push_str(&format!(
                    "var {var_name} = {loader}({});\n",
                    js_string_literal(&spec)
                ));
                for specifier in &import.specifiers {
                    match specifier {
                        ImportSpecifier::Default(d) => {
                            let _ = d;
                        }
                        ImportSpecifier::Namespace(n) => {
                            prologue.push_str(&format!("var {} = {var_name};\n", n.local.sym));
                        }
                        ImportSpecifier::Named(n) => {
                            let _ = n;
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
                    local_export_prologue.push_str(&format!(
                        "Object.defineProperty(exports, {}, {{ enumerable: true, get: function() {{ return {name}; }} }});\n",
                        js_string_literal(&name)
                    ));
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
                    let loader = if await_imports {
                        "await requireAsync"
                    } else {
                        "require"
                    };
                    prologue.push_str(&format!(
                        "var {var_name} = {loader}({});\n",
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
                            rest.push_str(&format!(
                                "Object.defineProperty(exports, {}, {{ enumerable: true, get: function() {{ return {var_name}[{}]; }} }});\n",
                                js_string_literal(&exported),
                                js_string_literal(&orig)
                            ));
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
                            let value = imported_bindings
                                .get(&orig)
                                .map(String::as_str)
                                .unwrap_or(orig.as_str());
                            local_export_prologue.push_str(&format!(
                                "Object.defineProperty(exports, {}, {{ enumerable: true, get: function() {{ return {value}; }} }});\n",
                                js_string_literal(&exported)
                            ));
                        }
                    }
                }
            },
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export_all)) => {
                let var_name = format!("__thaw_esm_reexport_all_{synthetic_count}");
                synthetic_count += 1;
                let spec = export_all.src.value.to_string_lossy();
                let loader = if await_imports {
                    "await requireAsync"
                } else {
                    "require"
                };
                prologue.push_str(&format!(
                    "var {var_name} = {loader}({});\n",
                    js_string_literal(&spec)
                ));
                rest.push_str(&format!(
                    "for (let __thaw_esm_key in {var_name}) {{ if (__thaw_esm_key !== 'default' && __thaw_esm_key !== '__esModule') Object.defineProperty(exports, __thaw_esm_key, {{ enumerable: true, get: function() {{ return {var_name}[__thaw_esm_key]; }} }}); }}\n"
                ));
            }
            // `import foo = require(...)`/`export = foo`/`export as
            // namespace`: TS-only forms that shouldn't appear in real
            // runtime `.js` files; skip gracefully rather than crashing
            // if one somehow does.
            ModuleItem::ModuleDecl(_) => {}
        }
    }

    Some(format!(
        "module.exports.__esModule = true;\n{local_export_prologue}{prologue}{rest}"
    ))
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

include!("registry/builtins.rs");

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
fn resolve_bare_require(
    node_modules_dir: &Path,
    spec: &str,
) -> Option<(String, String, PathBuf, PathBuf)> {
    let (dep_name, subpath) = split_bare_spec(spec);
    let dep_dir = node_modules_dir.join(dep_name);
    let target = match subpath {
        Some(sub) => read_manifest(&dep_dir)
            .ok()
            .and_then(|manifest| package_subpath_runtime_target(&manifest, sub))
            .unwrap_or_else(|| sub.to_string()),
        None => {
            let manifest = read_manifest(&dep_dir).ok()?;
            package_export_target(&manifest, None, &["require", "import", "default"])
                .or_else(|| manifest.get("main").and_then(|v| v.as_str()))
                .unwrap_or("index.js")
                .to_string()
        }
    };
    let (dep_relative, dep_abs) = resolve_module_path(&dep_dir, &target).ok()?;
    Some((dep_name.to_string(), dep_relative, dep_abs, dep_dir))
}

fn package_subpath_runtime_target(manifest: &serde_json::Value, subpath: &str) -> Option<String> {
    if let Some(target) = package_export_target(
        manifest,
        Some(subpath),
        &["require", "import", "node", "default"],
    ) {
        return Some(target.to_string());
    }
    let exports = manifest.get("exports")?.as_object()?;
    for (key, value) in exports {
        let Some(pattern) = key.strip_prefix("./") else {
            continue;
        };
        let Some(capture) = wildcard_capture(pattern, subpath) else {
            continue;
        };
        if let Some(target) =
            select_export_condition(value, &["require", "import", "node", "default"])
        {
            return Some(target.replace('*', capture));
        }
    }
    None
}

fn resolve_package_import(package_dir: &Path, spec: &str) -> Option<(String, PathBuf)> {
    let manifest = read_manifest(package_dir).ok()?;
    let imports = manifest.get("imports")?.as_object()?;
    if let Some(value) = imports.get(spec) {
        let target = select_export_condition(value, &["require", "import", "node", "default"])?;
        return resolve_module_path(package_dir, target).ok();
    }
    for (pattern, value) in imports {
        let Some(capture) = wildcard_capture(pattern, spec) else {
            continue;
        };
        if let Some(target) =
            select_export_condition(value, &["require", "import", "node", "default"])
        {
            return resolve_module_path(package_dir, &target.replace('*', capture)).ok();
        }
    }
    None
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

fn relative_module_specifier(from_dir: &Path, target: &Path) -> String {
    let from: Vec<_> = from_dir
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let to: Vec<_> = target
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = vec!["..".to_string(); from.len() - common];
    parts.extend(to[common..].iter().cloned());
    let path = parts.join("/");
    if path.starts_with("../") {
        path
    } else {
        format!("./{path}")
    }
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
fn prepare_async_modules(modules: &mut [BundledModule]) -> Result<(), String> {
    use std::collections::BTreeSet;

    for module in modules.iter_mut() {
        module.async_module = module.has_top_level_await;
    }
    loop {
        let async_keys: BTreeSet<_> = modules
            .iter()
            .filter(|module| module.async_module)
            .map(|module| module.key.clone())
            .collect();
        let mut changed = false;
        for module in modules.iter_mut().filter(|module| module.has_esm) {
            if module.async_module {
                continue;
            }
            if module.static_esm_specs.iter().any(|specifier| {
                module
                    .requires
                    .iter()
                    .any(|(source, target)| source == specifier && async_keys.contains(target))
            }) {
                module.async_module = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    fn visit(
        key: &str,
        modules: &[BundledModule],
        visiting: &mut Vec<String>,
        visited: &mut BTreeSet<String>,
    ) -> Result<(), String> {
        if let Some(index) = visiting.iter().position(|item| item == key) {
            let mut cycle = visiting[index..].to_vec();
            cycle.push(key.to_string());
            return Err(format!(
                "top-level await module cycle is not supported: {}",
                cycle.join(" -> ")
            ));
        }
        if !visited.insert(key.to_string()) {
            return Ok(());
        }
        let Some(module) = modules.iter().find(|module| module.key == key) else {
            return Ok(());
        };
        visiting.push(key.to_string());
        for (specifier, target) in &module.requires {
            if module.static_esm_specs.contains(specifier)
                && modules
                    .iter()
                    .any(|candidate| candidate.key == *target && candidate.async_module)
            {
                visit(target, modules, visiting, visited)?;
            }
        }
        visiting.pop();
        Ok(())
    }

    let mut visited = BTreeSet::new();
    for module in modules.iter().filter(|module| module.async_module) {
        visit(&module.key, modules, &mut Vec::new(), &mut visited)?;
    }

    for module in modules.iter_mut() {
        let source = rewrite_esm_to_commonjs_mode(&module.source, module.async_module)
            .unwrap_or_else(|| module.source.clone());
        module.source = source;
    }
    Ok(())
}

fn render_bundle(main_key: &str, modules: &[BundledModule]) -> String {
    let mut out = String::from("module.exports = (function() {\n");

    out.push_str("var __thaw_bundle_cache = {};\n");
    out.push_str("var __thaw_bundle_factories = {\n");
    for module in modules {
        let asynchronous = if module.async_module { "async " } else { "" };
        out.push_str(&format!(
            "{}: {asynchronous}function(module, exports, require, requireAsync, __filename, __dirname) {{\n{}\n}},\n",
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
        "function __thaw_bundle_target(map, spec) {\n\
         \x20\x20if (Object.prototype.hasOwnProperty.call(map, spec)) return { key: map[spec], factory: map[spec] };\n\
         \x20\x20var query = spec.indexOf('?');\n\
         \x20\x20var fragment = spec.indexOf('#', 1);\n\
         \x20\x20var suffixAt = query < 0 ? fragment : (fragment < 0 ? query : Math.min(query, fragment));\n\
         \x20\x20if (suffixAt < 0) return null;\n\
         \x20\x20var base = spec.slice(0, suffixAt);\n\
         \x20\x20if (!Object.prototype.hasOwnProperty.call(map, base)) return null;\n\
         \x20\x20var factory = map[base];\n\
         \x20\x20return { key: factory + spec.slice(suffixAt), factory: factory };\n\
         }\n\
         function __thaw_bundle_require(key, factoryKey) {\n\
         \x20\x20if (!(key in __thaw_bundle_cache)) {\n\
         \x20\x20\x20\x20factoryKey = factoryKey || key;\n\
         \x20\x20\x20\x20var mod = { exports: {} };\n\
         \x20\x20\x20\x20__thaw_bundle_cache[key] = mod;\n\
         \x20\x20\x20\x20var map = __thaw_bundle_require_maps[factoryKey] || {};\n\
         \x20\x20\x20\x20var localRequire = function(spec) {\n\
         \x20\x20\x20\x20\x20\x20var target = __thaw_bundle_target(map, spec);\n\
         \x20\x20\x20\x20\x20\x20if (target) return __thaw_bundle_require(target.key, target.factory);\n\
         \x20\x20\x20\x20\x20\x20return require(spec);\n\
         \x20\x20\x20\x20};\n\
         \x20\x20\x20\x20var localRequireAsync = function(spec) {\n\
         \x20\x20\x20\x20\x20\x20var target = __thaw_bundle_target(map, spec);\n\
         \x20\x20\x20\x20\x20\x20if (!target) return Promise.resolve().then(function() { return require(spec); });\n\
         \x20\x20\x20\x20\x20\x20var value = __thaw_bundle_require(target.key, target.factory);\n\
         \x20\x20\x20\x20\x20\x20return __thaw_bundle_cache[target.key].ready.then(function() { return value; });\n\
         \x20\x20\x20\x20};\n\
         \x20\x20\x20\x20var filename = '/thaw_modules/' + factoryKey, slash = filename.lastIndexOf('/'), dirname = slash < 0 ? '.' : filename.slice(0, slash);\n\
         \x20\x20\x20\x20var initialized = __thaw_bundle_factories[factoryKey](mod, mod.exports, localRequire, localRequireAsync, filename, dirname);\n\
         \x20\x20\x20\x20mod.ready = Promise.resolve(initialized).then(function() { return mod.exports; });\n\
         \x20\x20}\n\
         \x20\x20return __thaw_bundle_cache[key].exports;\n\
         }\n\
         function __thaw_bundle_create_require(base) {\n\
         \x20\x20var text = String(base || '');\n\
         \x20\x20var keys = Object.keys(__thaw_bundle_require_maps);\n\
         \x20\x20var factoryKey = keys.indexOf(text) >= 0 ? text : keys.find(function(key) { return text.endsWith('/' + key) || text.endsWith(key); });\n\
         \x20\x20var map = __thaw_bundle_require_maps[factoryKey] || {};\n\
         \x20\x20var created = function(spec) { var target = __thaw_bundle_target(map, String(spec)); if (target) return __thaw_bundle_require(target.key, target.factory); return require(String(spec)); };\n\
         \x20\x20created.resolve = function(spec) { var target = __thaw_bundle_target(map, String(spec)); return target ? target.key : String(spec); };\n\
         \x20\x20created.cache = __thaw_bundle_cache; return created;\n\
         }\n\
         globalThis.__thaw_bundle_create_require = __thaw_bundle_create_require;\n\
         globalThis.__thaw_worker_bundle_source =\n\
         \x20\x20'var __thaw_bundle_cache = {};\\nvar __thaw_bundle_factories = {' +\n\
         \x20\x20Object.keys(__thaw_bundle_factories).map(function(key) { return JSON.stringify(key) + ': ' + __thaw_bundle_factories[key].toString(); }).join(',\\n') +\n\
         \x20\x20'};\\nvar __thaw_bundle_require_maps = ' + JSON.stringify(__thaw_bundle_require_maps) + ';\\n' +\n\
         \x20\x20__thaw_bundle_target.toString() + '\\n' + __thaw_bundle_require.toString() + '\\n' + __thaw_bundle_create_require.toString() + '\\n' +\n\
         \x20\x20'globalThis.__thaw_bundle_create_require = __thaw_bundle_create_require;\\n';\n",
    );

    if modules.iter().any(|module| module.key == "node:http") {
        out.push_str(
            r#"if (typeof globalThis.fetch !== 'function') {
  globalThis.fetch = function(input, init) {
    var request;
    try { request = new Request(input, init); } catch (error) { return Promise.reject(error); }
    return Promise.resolve().then(async function() {
      var body = request.body ? await request.bytes() : new Uint8Array(), redirects = 0;
      function aborted() {
        var reason = request.signal && request.signal.reason;
        if (reason !== undefined) return reason;
        var error = new Error('This operation was aborted'); error.name = 'AbortError'; return error;
      }
      if (request.signal && request.signal.aborted) throw aborted();
      return new Promise(function(resolve, reject) {
        var active = null, settled = false;
        function finishReject(error) { if (settled) return; settled = true; cleanup(); reject(error); }
        function cleanup() { if (request.signal) request.signal.removeEventListener('abort', onAbort); }
        function onAbort() { if (active && typeof active.destroy === 'function') active.destroy(); finishReject(aborted()); }
        if (request.signal) request.signal.addEventListener('abort', onAbort, { once: true });
        var initialHeaders = {}; request.headers.forEach(function(value, name) { initialHeaders[name] = value; }); if (initialHeaders['accept-encoding'] === undefined) initialHeaders['accept-encoding'] = 'gzip, deflate, br';
        function dispatch(url, method, bytes, redirected, headers) {
          var parsed;
          try { parsed = new URL(url); } catch (error) { finishReject(error); return; }
          if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') { finishReject(new TypeError('fetch only supports http: and https: URLs')); return; }
          if (parsed.username || parsed.password) { finishReject(new TypeError('Request URL cannot contain credentials')); return; }
          parsed.hash = '';
          var transport = __thaw_bundle_require(parsed.protocol === 'https:' ? 'node:https' : 'node:http');
          if ((method === 'GET' || method === 'HEAD') && bytes.length) { bytes = new Uint8Array(); delete headers['content-length']; }
          var options = { method: method, headers: headers, signal: request.signal };
          try {
            active = transport.request(parsed.href, options, function(incoming) {
              var status = Number(incoming.statusCode), location = incoming.headers && incoming.headers.location;
              if (location && [301, 302, 303, 307, 308].indexOf(status) >= 0) {
                if (request.redirect === 'error') { finishReject(new TypeError('fetch redirect mode is set to error')); return; }
                if (request.redirect === 'follow') {
                  if (++redirects > 20) { finishReject(new TypeError('fetch redirect count exceeded')); return; }
                  var nextUrl = new URL(String(location), parsed), nextMethod = method, nextBody = bytes, nextHeaders = Object.assign({}, headers);
                  if (status === 303 && method !== 'HEAD' || (status === 301 || status === 302) && method === 'POST') { nextMethod = 'GET'; nextBody = new Uint8Array(); Object.keys(nextHeaders).forEach(function(name) { if (name.indexOf('content-') === 0) delete nextHeaders[name]; }); }
                  if (nextUrl.origin !== parsed.origin) ['authorization', 'proxy-authorization', 'cookie', 'cookie2'].forEach(function(name) { delete nextHeaders[name]; });
                  dispatch(nextUrl.href, nextMethod, nextBody, true, nextHeaders); return;
                }
              }
              var responseHeaders = new Headers();
              if (Array.isArray(incoming.rawHeaders)) for (var index = 0; index + 1 < incoming.rawHeaders.length; index += 2) responseHeaders.append(incoming.rawHeaders[index], incoming.rawHeaders[index + 1]);
              else if (incoming.headers) Object.keys(incoming.headers).forEach(function(name) { var value = incoming.headers[name]; if (Array.isArray(value)) value.forEach(function(item) { responseHeaders.append(name, item); }); else if (value !== undefined) responseHeaders.append(name, value); });
              var noBody = method === 'HEAD' || status === 101 || status === 204 || status === 205 || status === 304, controller, bodyAbort;
              function cleanupBody() { if (request.signal && bodyAbort) request.signal.removeEventListener('abort', bodyAbort); }
              var stream = noBody ? null : new ReadableStream({ start: function(value) { controller = value; }, cancel: function(reason) { cleanupBody(); if (incoming.destroy) incoming.destroy(reason); } });
              if (stream) {
                incoming.on('data', function(chunk) { if (controller) controller.enqueue(chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk)); });
                incoming.on('end', function() { cleanupBody(); if (controller) controller.close(); });
                incoming.on('error', function(error) { cleanupBody(); if (controller) controller.error(error); });
                incoming.on('aborted', function() { cleanupBody(); if (controller) controller.error(new TypeError('terminated')); });
                bodyAbort = function() { cleanupBody(); if (controller) controller.error(aborted()); if (incoming.destroy) incoming.destroy(); };
                if (request.signal) request.signal.addEventListener('abort', bodyAbort, { once: true });
                var contentEncoding = String(responseHeaders.get('content-encoding') || '').trim().toLowerCase();
                if (contentEncoding === 'gzip' || contentEncoding === 'deflate' || contentEncoding === 'br') stream = stream.pipeThrough(new DecompressionStream(contentEncoding));
              }
              var response;
              try { response = new Response(stream, { status: status, statusText: incoming.statusMessage || '', headers: responseHeaders }); }
              catch (error) { finishReject(error); return; }
              if (globalThis.__thaw_set_response_metadata) globalThis.__thaw_set_response_metadata(response, parsed.href, redirected);
              if (!settled) { settled = true; cleanup(); resolve(response); }
            });
            active.once('error', function(error) { var failure = new TypeError('fetch failed'); failure.cause = error; finishReject(failure); });
            if (bytes.length) active.write(bytes);
            active.end();
          } catch (error) { finishReject(error); }
        }
        dispatch(request.url, request.method, body, false, initialHeaders);
      });
    });
  };
}
"#,
        );
    }

    out.push_str(&format!(
        "var __thaw_bundle_entry_key = {};\n\
         var __thaw_bundle_entry = __thaw_bundle_require(__thaw_bundle_entry_key);\n\
         globalThis.__thaw_module_ready = __thaw_bundle_cache[__thaw_bundle_entry_key].ready;\n\
         return __thaw_bundle_entry;\n",
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
mod tests;
