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
            "var inspectCustom = Symbol.for('nodejs.util.inspect.custom');\n\
             function inspect(value, options) {\n\
             \x20\x20if (value && typeof value[inspectCustom] === 'function') return String(value[inspectCustom](2, options || {}, inspect));\n\
             \x20\x20if (typeof value === 'string') return \"'\" + value.replace(/\\\\/g, '\\\\\\\\').replace(/'/g, \"\\\\'\") + \"'\";\n\
             \x20\x20if (typeof value === 'function') return '[Function' + (value.name ? ': ' + value.name : '') + ']';\n\
             \x20\x20if (typeof value === 'symbol' || typeof value === 'bigint') return String(value);\n\
             \x20\x20if (value instanceof Error) return value.stack || value.name + ': ' + value.message;\n\
             \x20\x20var seen = new Set();\n\
             \x20\x20function render(input) {\n\
             \x20\x20\x20\x20if (input === null || typeof input !== 'object') return typeof input === 'string' ? \"'\" + input + \"'\" : String(input);\n\
             \x20\x20\x20\x20if (seen.has(input)) return '[Circular]'; seen.add(input);\n\
             \x20\x20\x20\x20var result;\n\
             \x20\x20\x20\x20if (Array.isArray(input)) result = '[ ' + input.map(render).join(', ') + ' ]';\n\
             \x20\x20\x20\x20else if (input instanceof Date) result = isNaN(input.getTime()) ? 'Invalid Date' : input.toISOString();\n\
             \x20\x20\x20\x20else if (input instanceof RegExp) result = String(input);\n\
             \x20\x20\x20\x20else if (input instanceof Map) result = 'Map(' + input.size + ') { ' + Array.from(input).map(function(entry) { return render(entry[0]) + ' => ' + render(entry[1]); }).join(', ') + ' }';\n\
             \x20\x20\x20\x20else if (input instanceof Set) result = 'Set(' + input.size + ') { ' + Array.from(input).map(render).join(', ') + ' }';\n\
             \x20\x20\x20\x20else result = '{ ' + Object.keys(input).map(function(key) { return key + ': ' + render(input[key]); }).join(', ') + ' }';\n\
             \x20\x20\x20\x20seen.delete(input); return result;\n\
             \x20\x20}\n\
             \x20\x20return render(value);\n\
             }\n\
             inspect.custom = inspectCustom; inspect.defaultOptions = {};\n\
             function format() {\n\
             \x20\x20var args = Array.prototype.slice.call(arguments); if (args.length === 0) return '';\n\
             \x20\x20if (typeof args[0] !== 'string') return args.map(inspect).join(' ');\n\
             \x20\x20var index = 1; var output = args[0].replace(/%[sdifjoOc%]/g, function(token) {\n\
             \x20\x20\x20\x20if (token === '%%') return '%'; if (index >= args.length) return token; var value = args[index++];\n\
             \x20\x20\x20\x20if (token === '%s') return String(value); if (token === '%d') return String(Number(value));\n\
             \x20\x20\x20\x20if (token === '%i') return String(parseInt(value, 10)); if (token === '%f') return String(parseFloat(value));\n\
             \x20\x20\x20\x20if (token === '%j') { try { return JSON.stringify(value); } catch (_) { return '[Circular]'; } }\n\
             \x20\x20\x20\x20if (token === '%c') return ''; return inspect(value);\n\
             \x20\x20});\n\
             \x20\x20while (index < args.length) { var extra = args[index++]; output += ' ' + (typeof extra === 'string' ? extra : inspect(extra)); } return output;\n\
             }\n\
             function formatWithOptions(options) { return format.apply(null, Array.prototype.slice.call(arguments, 1)); }\n\
             function inherits(constructor, superConstructor) { if (constructor === undefined || superConstructor === undefined) throw new TypeError('constructors are required'); constructor.super_ = superConstructor; Object.setPrototypeOf(constructor.prototype, superConstructor.prototype); }\n\
             var promisifyCustom = Symbol.for('nodejs.util.promisify.custom');\n\
             function promisify(original) {\n\
             \x20\x20if (typeof original !== 'function') throw new TypeError('original must be a function'); if (original[promisifyCustom]) return original[promisifyCustom];\n\
             \x20\x20function wrapped() { var self = this; var args = Array.prototype.slice.call(arguments); return new Promise(function(resolve, reject) { args.push(function(error) { if (error) reject(error); else { var values = Array.prototype.slice.call(arguments, 1); resolve(values.length > 1 ? values : values[0]); } }); original.apply(self, args); }); }\n\
             \x20\x20Object.setPrototypeOf(wrapped, Object.getPrototypeOf(original)); return wrapped;\n\
             }\n\
             promisify.custom = promisifyCustom;\n\
             function callbackify(original) {\n\
             \x20\x20if (typeof original !== 'function') throw new TypeError('original must be a function');\n\
             \x20\x20return function() { var args = Array.prototype.slice.call(arguments); var callback = args.pop(); if (typeof callback !== 'function') throw new TypeError('callback must be a function'); Promise.resolve(original.apply(this, args)).then(function(value) { queueMicrotask(function() { callback(null, value); }); }, function(error) { queueMicrotask(function() { callback(error || new Error('Promise was rejected with a falsy value')); }); }); };\n\
             }\n\
             function deprecate(fn) { return function() { return fn.apply(this, arguments); }; }\n\
             function stripVTControlCharacters(value) { return String(value).replace(/[\\u001B\\u009B][[\\]()#;?]*(?:(?:[a-zA-Z\\d]*(?:;[-a-zA-Z\\d\\/#&.:=?%@~_]+)*)?\\u0007|(?:(?:\\d{1,4}(?:;\\d{0,4})*)?[\\dA-PR-TZcf-nq-uy=><~]))/g, ''); }\n\
             function toUSVString(value) { return String(value).replace(/[\\uD800-\\uDBFF](?![\\uDC00-\\uDFFF])|(^|[^\\uD800-\\uDBFF])[\\uDC00-\\uDFFF]/g, function(match, prefix) { return (prefix || '') + '\\uFFFD'; }); }\n\
             function parseArgs(config) { config = config || {}; var args = Array.from(config.args === undefined ? (process.argv || []).slice(2) : config.args, String), definitions = config.options || {}, values = Object.create(null), positionals = [], tokens = [], short = Object.create(null); Object.keys(definitions).forEach(function(name) { var definition = definitions[name] || {}; if (definition.short) short[String(definition.short)] = name; if (definition.default !== undefined) values[name] = definition.multiple ? Array.from(definition.default) : definition.default; else if (definition.multiple) values[name] = []; }); function assign(name, value, index, inline) { var definition = definitions[name]; if (!definition) { if (config.strict !== false) { var error = new TypeError('Unknown option --' + name); error.code = 'ERR_PARSE_ARGS_UNKNOWN_OPTION'; throw error; } definition = { type: 'boolean' }; } if (definition.type === 'string') { if (value === undefined) { var error = new TypeError('Option --' + name + ' argument is missing'); error.code = 'ERR_PARSE_ARGS_INVALID_OPTION_VALUE'; throw error; } value = String(value); } else value = value === undefined ? true : Boolean(value); if (definition.multiple) (values[name] || (values[name] = [])).push(value); else values[name] = value; tokens.push({ kind: 'option', index: index, name: name, rawName: inline || '--' + name, value: definition.type === 'string' ? value : undefined, inlineValue: Boolean(inline && inline.indexOf('=') >= 0) }); } for (var index = 0; index < args.length; index++) { var argument = args[index]; if (argument === '--') { tokens.push({ kind: 'option-terminator', index: index }); for (index++; index < args.length; index++) { positionals.push(args[index]); tokens.push({ kind: 'positional', index: index, value: args[index] }); } break; } if (argument.slice(0, 2) === '--') { var equal = argument.indexOf('='), name = argument.slice(2, equal < 0 ? undefined : equal), value = equal < 0 ? undefined : argument.slice(equal + 1); if (name.slice(0, 3) === 'no-' && config.allowNegative && definitions[name.slice(3)] && definitions[name.slice(3)].type === 'boolean') { assign(name.slice(3), false, index, argument); continue; } var definition = definitions[name]; if (equal < 0 && definition && definition.type === 'string') value = args[++index]; assign(name, value, equal < 0 && definition && definition.type === 'string' ? index - 1 : index, argument); } else if (argument[0] === '-' && argument.length > 1) { var letters = argument.slice(1); for (var letterIndex = 0; letterIndex < letters.length; letterIndex++) { var name = short[letters[letterIndex]] || letters[letterIndex], definition = definitions[name], value; if (definition && definition.type === 'string') { value = letters.slice(letterIndex + 1) || args[++index]; letterIndex = letters.length; } assign(name, value, index, '-' + letters[letterIndex]); } } else { if (!config.allowPositionals && config.strict !== false) { var error = new TypeError('Unexpected argument ' + argument); error.code = 'ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL'; throw error; } positionals.push(argument); tokens.push({ kind: 'positional', index: index, value: argument }); } } var result = { values: values, positionals: positionals }; if (config.tokens) result.tokens = tokens; return result; }\n\
             var types = globalThis.__thaw_util_types || (globalThis.__thaw_util_types = { isDate: function(value) { return value instanceof Date; }, isRegExp: function(value) { return value instanceof RegExp; }, isMap: function(value) { return value instanceof Map; }, isSet: function(value) { return value instanceof Set; }, isPromise: function(value) { return value instanceof Promise; }, isArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isAnyArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isTypedArray: function(value) { return ArrayBuffer.isView(value) && !(value instanceof DataView); }, isNativeError: function(value) { return value instanceof Error; }, isArgumentsObject: function(value) { return Object.prototype.toString.call(value) === '[object Arguments]'; } });\n\
             module.exports = { inspect: inspect, format: format, formatWithOptions: formatWithOptions, inherits: inherits, promisify: promisify, callbackify: callbackify, deprecate: deprecate, stripVTControlCharacters: stripVTControlCharacters, toUSVString: toUSVString, parseArgs: parseArgs, TextEncoder: globalThis.TextEncoder, TextDecoder: globalThis.TextDecoder, types: types };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "util/types" => Some(
            "var tag = function(value) { return Object.prototype.toString.call(value); }; var types = { isDate: function(value) { return value instanceof Date; }, isRegExp: function(value) { return value instanceof RegExp; }, isMap: function(value) { return value instanceof Map; }, isSet: function(value) { return value instanceof Set; }, isWeakMap: function(value) { return value instanceof WeakMap; }, isWeakSet: function(value) { return value instanceof WeakSet; }, isPromise: function(value) { return value instanceof Promise; }, isArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isAnyArrayBuffer: function(value) { return value instanceof ArrayBuffer || (typeof SharedArrayBuffer === 'function' && value instanceof SharedArrayBuffer); }, isArrayBufferView: function(value) { return ArrayBuffer.isView(value); }, isDataView: function(value) { return value instanceof DataView; }, isTypedArray: function(value) { return ArrayBuffer.isView(value) && !(value instanceof DataView); }, isUint8Array: function(value) { return value instanceof Uint8Array; }, isUint8ClampedArray: function(value) { return value instanceof Uint8ClampedArray; }, isUint16Array: function(value) { return value instanceof Uint16Array; }, isUint32Array: function(value) { return value instanceof Uint32Array; }, isInt8Array: function(value) { return value instanceof Int8Array; }, isInt16Array: function(value) { return value instanceof Int16Array; }, isInt32Array: function(value) { return value instanceof Int32Array; }, isFloat32Array: function(value) { return value instanceof Float32Array; }, isFloat64Array: function(value) { return value instanceof Float64Array; }, isBigInt64Array: function(value) { return typeof BigInt64Array === 'function' && value instanceof BigInt64Array; }, isBigUint64Array: function(value) { return typeof BigUint64Array === 'function' && value instanceof BigUint64Array; }, isNativeError: function(value) { return value instanceof Error; }, isArgumentsObject: function(value) { return tag(value) === '[object Arguments]'; }, isNumberObject: function(value) { return tag(value) === '[object Number]'; }, isStringObject: function(value) { return tag(value) === '[object String]'; }, isBooleanObject: function(value) { return tag(value) === '[object Boolean]'; }, isBigIntObject: function(value) { return tag(value) === '[object BigInt]'; }, isSymbolObject: function(value) { return tag(value) === '[object Symbol]'; }, isBoxedPrimitive: function(value) { return /\\[object (Number|String|Boolean|BigInt|Symbol)\\]/.test(tag(value)); }, isAsyncFunction: function(value) { return tag(value) === '[object AsyncFunction]'; }, isGeneratorFunction: function(value) { return tag(value) === '[object GeneratorFunction]'; }, isGeneratorObject: function(value) { return tag(value) === '[object Generator]'; }, isExternal: function() { return false; }, isProxy: function() { return false; }, isModuleNamespaceObject: function() { return false; }, isKeyObject: function() { return false; }, isCryptoKey: function() { return false; } }; module.exports = types; module.exports.default = types; module.exports.__esModule = true;\n",
        ),
        // Found necessary by a real ESM package (`has-flag`): `import
        // process from 'process'` -- Node exposes `process` as both a
        // global and a core module; this is the module half. `.default`
        // is set too so the ESM-interop convention `rewrite_esm_to_commonjs`
        // generates for a default import (`.__esModule ? .default : ...`)
        // finds the same object either way. Only the couple of fields a
        // real package has actually been found to read.
        "process" => Some(
            "var __thaw_process = globalThis.process || { argv: [], env: {}, platform: 'linux', version: '', versions: {}, cwd: function() { return '/'; }, nextTick: function(fn) { var args = Array.prototype.slice.call(arguments, 1); Promise.resolve().then(function() { fn.apply(undefined, args); }); } };\n\
             if (!__thaw_process.cwd) __thaw_process.cwd = function() { return '/'; };\n\
             module.exports = __thaw_process;\n\
             module.exports.default = __thaw_process;\n\
             module.exports.__esModule = true;\n",
        ),
        "child_process" => Some(
            r#"var EventEmitter = require('node:events'), streams = require('node:stream');
             function normalize(command, args, options) { if (!Array.isArray(args)) { options = args || {}; args = []; } else options = options || {}; command = String(command); args = args.map(String); if (options.shell) { var shell = typeof options.shell === 'string' ? options.shell : '/bin/sh', joined = [command].concat(args).map(function(value) { return "'" + value.replace(/'/g, "'\\''") + "'"; }).join(' '); command = shell; args = ['-c', joined]; } var environment; if (options.env) { environment = {}; Object.keys(options.env).forEach(function(name) { if (options.env[name] !== undefined) environment[name] = String(options.env[name]); }); } var input = options.input === undefined ? undefined : Buffer.from(options.input, options.encoding).toString('hex'); return { command: command, args: args, options: { cwd: options.cwd === undefined ? undefined : String(options.cwd), env: environment, input: input, detached: Boolean(options.detached) }, original: options }; }
             function processError(record, normalized) { var error = new Error(record.error || ('Command failed: ' + normalized.command)); error.code = record.code || null; error.errno = record.errno === undefined ? null : record.errno; error.path = record.path || normalized.command; error.spawnargs = normalized.args.slice(); error.status = record.status === undefined ? null : record.status; error.signal = record.signal === undefined ? null : record.signal; return error; }
             function decode(record, options) { var encoding = options.encoding === undefined ? null : options.encoding, stdout = Buffer.from(record.stdout || '', 'hex'), stderr = Buffer.from(record.stderr || '', 'hex'); if (encoding && encoding !== 'buffer') { stdout = stdout.toString(encoding); stderr = stderr.toString(encoding); } return { pid: record.pid || 0, output: [null, stdout, stderr], stdout: stdout, stderr: stderr, status: record.status === undefined ? null : record.status, signal: record.signal === undefined ? null : record.signal, error: record.error ? processError(record, { command: record.path || '', args: [] }) : undefined }; }
             function spawnSync(command, args, options) { var normalized = normalize(command, args, options), record = JSON.parse(__thaw_child_process_sync(normalized.command, JSON.stringify(normalized.args), JSON.stringify(normalized.options))), result = decode(record, normalized.original), maxBuffer = normalized.original.maxBuffer === undefined ? 1048576 : Number(normalized.original.maxBuffer); if (!result.error && result.stdout.length + result.stderr.length > maxBuffer) { var error = new Error('spawnSync ' + normalized.command + ' ENOBUFS'); error.code = 'ENOBUFS'; result.error = error; result.status = null; } return result; }
             function execFileSync(file, args, options) { var normalized = normalize(file, args, options), result = spawnSync(file, args, options); if (result.error || result.status !== 0) { var error = result.error || processError({ status: result.status, signal: result.signal }, normalized); error.stdout = result.stdout; error.stderr = result.stderr; error.output = result.output; error.pid = result.pid; throw error; } return result.stdout; }
             function execSync(command, options) { options = options || {}; return execFileSync(options.shell || '/bin/sh', ['-c', String(command)], Object.assign({}, options, { shell: false })); }
             var children = new Map(), signals = { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGKILL: 9, SIGUSR1: 10, SIGUSR2: 12, SIGPIPE: 13, SIGALRM: 14, SIGTERM: 15 }; function signalName(signal) { if (signal === null || signal === undefined) return null; var names = Object.keys(signals); for (var index = 0; index < names.length; index++) if (signals[names[index]] === signal) return names[index]; return 'SIG' + signal; } function signalNumber(signal) { if (signal === undefined || signal === null) return signals.SIGTERM; if (typeof signal === 'string') { signal = signal.toUpperCase(); if (signals[signal] !== undefined) return signals[signal]; } else if (Number.isInteger(signal) && signal > 0) return signal; var error = new TypeError('Unknown signal: ' + signal); error.code = 'ERR_UNKNOWN_SIGNAL'; throw error; }
             function ChildProcess(normalized) { EventEmitter.call(this); this.pid = 0; this.spawnfile = normalized.command; this.spawnargs = [normalized.command].concat(normalized.args); this.killed = false; this.connected = false; this.exitCode = null; this.signalCode = null; this.detached = Boolean(normalized.original.detached); this._closed = false; this._spawnError = false; this._timedOut = false; var child = this; this._stdin = new streams.Writable({ write: function(chunk, encoding, callback) { if (!child._closed) __thaw_child_process_stdin(child._handle, Buffer.from(chunk).toString('hex'), false); callback(); }, final: function(callback) { if (!child._closed) __thaw_child_process_stdin(child._handle, '', true); callback(); } }); this._stdout = new streams.PassThrough(); this._stderr = new streams.PassThrough(); var stdio = normalized.original.stdio === undefined ? 'pipe' : normalized.original.stdio, modes = Array.isArray(stdio) ? stdio.slice(0, 3) : [stdio, stdio, stdio]; while (modes.length < 3) modes.push('pipe'); modes = modes.map(function(mode) { mode = mode === null || mode === undefined ? 'pipe' : mode; if (mode !== 'pipe' && mode !== 'ignore' && mode !== 'inherit') { var error = new TypeError('The argument stdio is invalid. Received ' + mode); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; } return mode; }); this._stdioModes = modes; this.stdin = modes[0] === 'pipe' ? this._stdin : null; this.stdout = modes[1] === 'pipe' ? this._stdout : null; this.stderr = modes[2] === 'pipe' ? this._stderr : null; this.stdio = [this.stdin, this.stdout, this.stderr]; this.channel = null; this._handle = __thaw_child_process_spawn(normalized.command, JSON.stringify(normalized.args), JSON.stringify(normalized.options)); children.set(this._handle, this); if (modes[0] !== 'pipe') this._stdin.end(); var timeout = Number(normalized.original.timeout || 0); if (timeout > 0 && Number.isFinite(timeout)) this._timeout = setTimeout(function() { if (!child._closed) { child._timedOut = true; child.kill(normalized.original.killSignal); } }, timeout); var signal = normalized.original.signal; if (signal && typeof signal.addEventListener === 'function') { this._abortSignal = signal; this._abortListener = function() { if (child._closed || child._aborted) return; child._aborted = true; var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = signal.reason; child.emit('error', error); child.kill(normalized.original.killSignal); }; signal.addEventListener('abort', this._abortListener, { once: true }); if (signal.aborted) queueMicrotask(this._abortListener); } }
             ChildProcess.prototype = Object.create(EventEmitter.prototype); ChildProcess.prototype.constructor = ChildProcess; ChildProcess.prototype.kill = function(signal) { if (this._closed) return false; this.killed = __thaw_child_process_kill(this._handle, signalNumber(signal)); return this.killed; }; ChildProcess.prototype.ref = function() { return this; }; ChildProcess.prototype.unref = function() { return this; }; ChildProcess.prototype.disconnect = function() { if (!this.connected) { var error = new Error('IPC channel is already disconnected'); error.code = 'ERR_IPC_DISCONNECTED'; throw error; } this.connected = false; this.channel = null; if (this.stdin && !this.stdin.writableEnded) this.stdin.end(); this.emit('disconnect'); return this; }; ChildProcess.prototype.send = function(message, sendHandle, options, callback) { if (typeof sendHandle === 'function') callback = sendHandle; else if (typeof options === 'function') callback = options; if (!this.connected || !this._ipc) { var error = new Error('Channel closed'); error.code = 'ERR_IPC_CHANNEL_CLOSED'; if (callback) { queueMicrotask(function() { callback(error); }); return false; } throw error; } var serialized; try { serialized = JSON.stringify(message); } catch (error) { if (callback) queueMicrotask(function() { callback(error); }); else throw error; return false; } this.stdin.write('__THAW_IPC__' + serialized + '\n', function(error) { if (callback) callback(error || null); }); return true; };
             function spawn(command, args, options) { return new ChildProcess(normalize(command, args, options)); }
             function fork(modulePath, args, options) { if (!Array.isArray(args)) { options = args || {}; args = []; } else options = options || {}; if (typeof modulePath !== 'string' && !(modulePath instanceof String)) { var pathError = new TypeError('The modulePath argument must be of type string'); pathError.code = 'ERR_INVALID_ARG_TYPE'; throw pathError; } var wrapper = "var prefix='__THAW_IPC__',buffer='';process.send=function(value,handle,options,callback){if(typeof handle==='function')callback=handle;else if(typeof options==='function')callback=options;try{process.stdout.write(prefix+JSON.stringify(value)+'\\n',function(){if(typeof callback==='function')callback(null);});return true;}catch(error){if(typeof callback==='function')callback(error);else throw error;return false;}};process.connected=true;process.disconnect=function(){if(!process.connected)return;process.connected=false;process.emit('disconnect');process.stdin.destroy();setTimeout(function(){process.exit(0);},0);};process.stdin.setEncoding('utf8');process.stdin.on('data',function(chunk){buffer+=chunk;for(;;){var end=buffer.indexOf('\\n');if(end<0)break;var line=buffer.slice(0,end);buffer=buffer.slice(end+1);if(line.indexOf(prefix)===0)process.emit('message',JSON.parse(line.slice(prefix.length)));}});process.stdin.on('end',process.disconnect);require(require('path').resolve(process.argv[1]));"; var childOptions = Object.assign({}, options, { stdio: 'pipe' }), execArgv = options.execArgv === undefined ? [] : Array.from(options.execArgv, String), child = spawn(options.execPath || process.execPath || '/usr/bin/node', execArgv.concat(['-e', wrapper, String(modulePath)]).concat(args.map(String)), childOptions); child.connected = true; child.channel = {}; child._ipc = true; child._ipcBuffer = ''; child._ipcOutputMode = options.silent ? 'pipe' : 'inherit'; if (!options.silent) { child.stdout = null; child.stderr = null; child.stdio = [child.stdin, null, null, child.channel]; } else child.stdio.push(child.channel); return child; }
             function execFile(file, args, options, callback) { if (typeof args === 'function') { callback = args; args = []; options = {}; } else if (!Array.isArray(args)) { callback = options; options = args || {}; args = []; } else if (typeof options === 'function') { callback = options; options = {}; } options = options || {}; var child = spawn(file, args, options), stdout = [], stderr = [], encoding = options.encoding || 'utf8', maxBuffer = options.maxBuffer === undefined ? 1048576 : Number(options.maxBuffer); child.stdout.on('data', function(value) { stdout.push(Buffer.from(value)); }); child.stderr.on('data', function(value) { stderr.push(Buffer.from(value)); }); child.once('close', function(code, signal) { var out = Buffer.concat(stdout), err = Buffer.concat(stderr), decodedOut = encoding === 'buffer' ? out : out.toString(encoding), decodedErr = encoding === 'buffer' ? err : err.toString(encoding), error = null; if (out.length + err.length > maxBuffer) { error = new Error('stdout maxBuffer length exceeded'); error.code = 'ERR_CHILD_PROCESS_STDIO_MAXBUFFER'; } else if (code !== 0) { error = new Error('Command failed: ' + file + '\n' + decodedErr); error.code = code; error.killed = child.killed; error.signal = signal; error.cmd = file; } if (error) { error.stdout = decodedOut; error.stderr = decodedErr; } if (typeof callback === 'function') callback(error, decodedOut, decodedErr); }); child.stdin.end(); return child; }
             function exec(command, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } options = options || {}; return execFile(options.shell || '/bin/sh', ['-c', String(command)], Object.assign({}, options, { shell: false }), callback); }
             if (!globalThis.__thaw_child_process_poll_installed) { globalThis.__thaw_child_process_poll_installed = true; var previousPlatformPoll = globalThis.__thaw_poll_platform_events; globalThis.__thaw_poll_platform_events = function() { if (previousPlatformPoll) previousPlatformPoll(); JSON.parse(__thaw_child_process_poll()).forEach(function(event) { var child = children.get(event.handle); if (!child) return; if (event.type === 'spawn') { child.pid = Number(event.pid); child.emit('spawn'); } else if (event.type === 'stdout') { var stdout = Buffer.from(event.value, 'hex'); if (child._ipc) { child._ipcBuffer += stdout.toString(); for (;;) { var end = child._ipcBuffer.indexOf('\n'); if (end < 0) break; var line = child._ipcBuffer.slice(0, end); child._ipcBuffer = child._ipcBuffer.slice(end + 1); if (line.indexOf('__THAW_IPC__') === 0) { try { child.emit('message', JSON.parse(line.slice(12))); } catch (error) { child.emit('error', error); } } else if (child._ipcOutputMode === 'inherit' && process.stdout && process.stdout.write) process.stdout.write(Buffer.from(line + '\n')); else child._stdout.write(Buffer.from(line + '\n')); } } else if (child._stdioModes[1] === 'pipe') child._stdout.write(stdout); else if (child._stdioModes[1] === 'inherit' && process.stdout && process.stdout.write) process.stdout.write(stdout); } else if (event.type === 'stderr') { var stderr = Buffer.from(event.value, 'hex'); if (child._ipc && child._ipcOutputMode === 'inherit' && process.stderr && process.stderr.write) process.stderr.write(stderr); else if (child._stdioModes[2] === 'pipe') child._stderr.write(stderr); else if (child._stdioModes[2] === 'inherit' && process.stderr && process.stderr.write) process.stderr.write(stderr); } else if (event.type === 'error') { child._spawnError = true; var error = new Error(event.message); error.code = event.code; error.path = child.spawnfile; error.spawnargs = child.spawnargs.slice(1); child.emit('error', error); } else if (event.type === 'exit') { child._closed = true; child.exitCode = event.code === null ? null : Number(event.code); child.signalCode = signalName(event.signal); if (child._ipcBuffer) { if (child._ipcOutputMode === 'inherit' && process.stdout && process.stdout.write) process.stdout.write(Buffer.from(child._ipcBuffer)); else child._stdout.write(Buffer.from(child._ipcBuffer)); } child._stdout.end(); child._stderr.end(); if (!child._stdin.writableEnded) child._stdin.end(); if (child.connected) { child.connected = false; child.channel = null; child.emit('disconnect'); } if (child._timeout) clearTimeout(child._timeout); if (child._abortSignal && child._abortListener) child._abortSignal.removeEventListener('abort', child._abortListener); if (!child._spawnError) child.emit('exit', child.exitCode, child.signalCode); child.emit('close', child.exitCode, child.signalCode); children.delete(event.handle); } }); }; }
             module.exports = { ChildProcess: ChildProcess, spawn: spawn, fork: fork, exec: exec, execFile: execFile, spawnSync: spawnSync, execFileSync: execFileSync, execSync: execSync }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "punycode" => Some(
            "var base = 36, tMin = 1, tMax = 26, skew = 38, damp = 700, initialBias = 72, initialN = 128, delimiter = '-'; function adapt(delta, points, first) { delta = first ? Math.floor(delta / damp) : delta >> 1; delta += Math.floor(delta / points); var k = 0; while (delta > Math.floor(((base - tMin) * tMax) / 2)) { delta = Math.floor(delta / (base - tMin)); k += base; } return k + Math.floor(((base - tMin + 1) * delta) / (delta + skew)); } function encodeDigit(value) { return String.fromCharCode(value + 22 + 75 * (value < 26)); } function decodeDigit(code) { if (code >= 48 && code <= 57) return code - 22; if (code >= 65 && code <= 90) return code - 65; if (code >= 97 && code <= 122) return code - 97; return base; } function codePoints(value) { return Array.from(String(value)).map(function(character) { return character.codePointAt(0); }); }\n\
             function encode(value) { var input = codePoints(value), output = [], n = initialN, delta = 0, bias = initialBias; input.forEach(function(point) { if (point < 128) output.push(String.fromCharCode(point)); }); var basic = output.length, handled = basic; if (basic) output.push(delimiter); while (handled < input.length) { var next = Infinity; input.forEach(function(point) { if (point >= n && point < next) next = point; }); delta += (next - n) * (handled + 1); n = next; input.forEach(function(point) { if (point < n) delta++; if (point === n) { var q = delta; for (var k = base;; k += base) { var threshold = k <= bias ? tMin : k >= bias + tMax ? tMax : k - bias; if (q < threshold) break; output.push(encodeDigit(threshold + ((q - threshold) % (base - threshold)))); q = Math.floor((q - threshold) / (base - threshold)); } output.push(encodeDigit(q)); bias = adapt(delta, handled + 1, handled === basic); delta = 0; handled++; } }); delta++; n++; } return output.join(''); }\n\
             function decode(value) { var input = String(value), output = [], n = initialN, index = 0, bias = initialBias, i = 0, delimiterIndex = input.lastIndexOf(delimiter); if (delimiterIndex >= 0) { for (var basicIndex = 0; basicIndex < delimiterIndex; basicIndex++) { var basicCode = input.charCodeAt(basicIndex); if (basicCode >= 128) throw new RangeError('Illegal input'); output.push(basicCode); } index = delimiterIndex + 1; } while (index < input.length) { var oldI = i, weight = 1; for (var k = base;; k += base) { if (index >= input.length) throw new RangeError('Invalid input'); var digit = decodeDigit(input.charCodeAt(index++)); if (digit >= base) throw new RangeError('Invalid input'); i += digit * weight; var threshold = k <= bias ? tMin : k >= bias + tMax ? tMax : k - bias; if (digit < threshold) break; weight *= base - threshold; } var length = output.length + 1; bias = adapt(i - oldI, length, oldI === 0); n += Math.floor(i / length); i %= length; output.splice(i, 0, n); i++; } return String.fromCodePoint.apply(String, output); }\n\
             function mapDomain(value, callback) { var input = String(value), parts = input.split('@'), local = ''; if (parts.length > 1) local = parts.shift() + '@'; return local + parts.join('@').replace(/[\\u3002\\uFF0E\\uFF61]/g, '.').split('.').map(callback).join('.'); } function toASCII(value) { return mapDomain(value, function(label) { return /[^\\x00-\\x7F]/.test(label) ? 'xn--' + encode(label) : label; }); } function toUnicode(value) { return mapDomain(value, function(label) { return /^xn--/i.test(label) ? decode(label.slice(4).toLowerCase()) : label; }); } var ucs2 = { decode: codePoints, encode: function(points) { return String.fromCodePoint.apply(String, points); } }; module.exports = { version: '2.1.0', ucs2: ucs2, decode: decode, encode: encode, toASCII: toASCII, toUnicode: toUnicode }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
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
             \x20\x20p = String(p); if (p === '') return '.'; var parts = p.split('/'); var out = []; var trailing = p.length > 1 && p.charAt(p.length - 1) === '/';\n\
             \x20\x20var abs = p.charAt(0) === '/';\n\
             \x20\x20for (var i = 0; i < parts.length; i++) {\n\
             \x20\x20\x20\x20var part = parts[i];\n\
             \x20\x20\x20\x20if (part === '' || part === '.') continue;\n\
             \x20\x20\x20\x20if (part === '..') { if (out.length && out[out.length - 1] !== '..') out.pop(); else if (!abs) out.push('..'); } else { out.push(part); }\n\
             \x20\x20}\n\
             \x20\x20var result = (abs ? '/' : '') + out.join('/'); if (result === '') result = abs ? '/' : '.'; if (trailing && result !== '/') result += '/'; return result;\n\
             }\n\
             function resolve() {\n\
             \x20\x20var p = '';\n\
             \x20\x20for (var i = 0; i < arguments.length; i++) {\n\
             \x20\x20\x20\x20var seg = String(arguments[i]);\n\
             \x20\x20\x20\x20if (seg.charAt(0) === '/') { p = seg; } else { p = p ? p + '/' + seg : seg; }\n\
             \x20\x20}\n\
             \x20\x20if (p.charAt(0) !== '/') p = (globalThis.process && process.cwd ? process.cwd() : '/') + '/' + p;\n\
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
             function basename(p, suffix) {\n\
             \x20\x20var n = __thaw_path_normalize(p);\n\
             \x20\x20var idx = n.lastIndexOf('/');\n\
             \x20\x20var base = idx === -1 ? n : n.substring(idx + 1); if (suffix && base.endsWith(String(suffix))) base = base.substring(0, base.length - String(suffix).length); return base;\n\
             }\n\
             function extname(p) { var base = basename(p); var index = base.lastIndexOf('.'); return index <= 0 ? '' : base.substring(index); }\n\
             function isAbsolute(p) { return String(p).charAt(0) === '/'; }\n\
             function relative(from, to) { var left = resolve(from).split('/').filter(Boolean); var right = resolve(to).split('/').filter(Boolean); var shared = 0; while (shared < left.length && shared < right.length && left[shared] === right[shared]) shared++; return left.slice(shared).map(function() { return '..'; }).concat(right.slice(shared)).join('/') || ''; }\n\
             function parse(p) { var root = isAbsolute(p) ? '/' : ''; var dir = dirname(p); var base = basename(p); var ext = extname(base); return { root: root, dir: dir, base: base, ext: ext, name: ext ? base.substring(0, base.length - ext.length) : base }; }\n\
             function format(value) { var dir = value.dir || value.root || ''; var base = value.base || String(value.name || '') + String(value.ext || ''); return dir ? (dir === '/' ? '/' : dir + '/') + base : base; }\n\
             function toNamespacedPath(p) { return p; }\n\
             var posix = { resolve: resolve, join: join, dirname: dirname, basename: basename, extname: extname, normalize: __thaw_path_normalize, relative: relative, isAbsolute: isAbsolute, parse: parse, format: format, toNamespacedPath: toNamespacedPath, sep: '/', delimiter: ':' };\n\
             function winInput(p) { return String(p).replace(/\\\\/g, '/').replace(/^([A-Za-z]):/, '/$1:'); }\n\
             function winOutput(p) { return String(p).replace(/^\\/([A-Za-z]:)/, '$1').replace(/\\//g, '\\\\'); }\n\
             var win32 = { resolve: function() { return winOutput(resolve.apply(null, Array.from(arguments, winInput))); }, join: function() { return winOutput(join.apply(null, Array.from(arguments, winInput))); }, dirname: function(p) { return winOutput(dirname(winInput(p))); }, basename: function(p, suffix) { return basename(winInput(p), suffix); }, extname: function(p) { return extname(winInput(p)); }, normalize: function(p) { return winOutput(__thaw_path_normalize(winInput(p))); }, relative: function(from, to) { return winOutput(relative(winInput(from), winInput(to))); }, isAbsolute: function(p) { return /^[A-Za-z]:[\\\\/]|^[\\\\/]{2}/.test(String(p)); }, parse: function(p) { var result = parse(winInput(p)); result.root = /^[A-Za-z]:/.test(String(p)) ? String(p).substring(0, 3) : result.root; result.dir = winOutput(result.dir); return result; }, format: function(value) { return winOutput(format(value)); }, toNamespacedPath: toNamespacedPath, sep: '\\\\', delimiter: ';' };\n\
             module.exports = posix; module.exports.posix = posix; module.exports.win32 = win32;\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "path/posix" => Some(
            "module.exports = require('node:path').posix; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "path/win32" => Some(
            "module.exports = require('node:path').win32; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "url" => Some(
            "function pathToFileURL(path) {\n\
             \x20\x20var value = String(path);\n\
             \x20\x20if (value.charAt(0) !== '/') value = '/' + value;\n\
             \x20\x20var encoded = value.split('/').map(function(part) { return encodeURIComponent(part); }).join('/');\n\
             \x20\x20return new globalThis.URL('file://' + encoded);\n\
             }\n\
             function fileURLToPath(input) {\n\
             \x20\x20var url = input instanceof globalThis.URL ? input : new globalThis.URL(input);\n\
             \x20\x20if (url.protocol !== 'file:') throw new TypeError('URL must use the file: protocol');\n\
             \x20\x20if (url.hostname !== '' && url.hostname !== 'localhost') throw new TypeError('file URL host must be empty or localhost');\n\
             \x20\x20if (/%2f|%5c/i.test(url.pathname)) throw new TypeError('file URL path must not include encoded separators');\n\
             \x20\x20return decodeURIComponent(url.pathname);\n\
             }\n\
             function urlToHttpOptions(input) {\n\
             \x20\x20var url = input instanceof globalThis.URL ? input : new globalThis.URL(input);\n\
             \x20\x20var options = { protocol: url.protocol, hostname: url.hostname, hash: url.hash, search: url.search, pathname: url.pathname, path: url.pathname + url.search, href: url.href };\n\
             \x20\x20if (url.port !== '') options.port = Number(url.port);\n\
             \x20\x20if (url.username !== '' || url.password !== '') options.auth = decodeURIComponent(url.username) + ':' + decodeURIComponent(url.password);\n\
             \x20\x20return options;\n\
             }\n\
             module.exports = { URL: globalThis.URL, URLSearchParams: globalThis.URLSearchParams, pathToFileURL: pathToFileURL, fileURLToPath: fileURLToPath, urlToHttpOptions: urlToHttpOptions };\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "querystring" => Some(
            "function escape(value) { return encodeURIComponent(String(value)); }\n\
             function unescape(value) {\n\
             \x20\x20try { return decodeURIComponent(String(value).replace(/\\+/g, ' ')); } catch (_) { return String(value); }\n\
             }\n\
             function primitive(value) {\n\
             \x20\x20return value === null || value === undefined ? '' : (typeof value === 'string' || typeof value === 'number' || typeof value === 'bigint' || typeof value === 'boolean' ? String(value) : '');\n\
             }\n\
             function stringify(object, separator, assignment, options) {\n\
             \x20\x20separator = separator === undefined ? '&' : String(separator);\n\
             \x20\x20assignment = assignment === undefined ? '=' : String(assignment);\n\
             \x20\x20var encoder = options && typeof options.encodeURIComponent === 'function' ? options.encodeURIComponent : escape;\n\
             \x20\x20if (object === null || typeof object !== 'object') return '';\n\
             \x20\x20var fields = [];\n\
             \x20\x20Object.keys(object).forEach(function(key) {\n\
             \x20\x20\x20\x20var values = Array.isArray(object[key]) ? object[key] : [object[key]];\n\
             \x20\x20\x20\x20if (values.length === 0) return;\n\
             \x20\x20\x20\x20values.forEach(function(value) { fields.push(encoder(key) + assignment + encoder(primitive(value))); });\n\
             \x20\x20});\n\
             \x20\x20return fields.join(separator);\n\
             }\n\
             function parse(text, separator, assignment, options) {\n\
             \x20\x20var result = Object.create(null);\n\
             \x20\x20var source = String(text);\n\
             \x20\x20separator = separator === undefined ? '&' : String(separator);\n\
             \x20\x20assignment = assignment === undefined ? '=' : String(assignment);\n\
             \x20\x20var decoder = options && typeof options.decodeURIComponent === 'function' ? options.decodeURIComponent : unescape;\n\
             \x20\x20var maxKeys = options && options.maxKeys !== undefined ? Number(options.maxKeys) : 1000;\n\
             \x20\x20var fields = source === '' ? [] : source.split(separator);\n\
             \x20\x20if (maxKeys > 0) fields = fields.slice(0, maxKeys);\n\
             \x20\x20fields.forEach(function(field) {\n\
             \x20\x20\x20\x20var index = field.indexOf(assignment);\n\
             \x20\x20\x20\x20var key = decoder(index < 0 ? field : field.substring(0, index));\n\
             \x20\x20\x20\x20var value = decoder(index < 0 ? '' : field.substring(index + assignment.length));\n\
             \x20\x20\x20\x20if (!Object.prototype.hasOwnProperty.call(result, key)) result[key] = value;\n\
             \x20\x20\x20\x20else if (Array.isArray(result[key])) result[key].push(value);\n\
             \x20\x20\x20\x20else result[key] = [result[key], value];\n\
             \x20\x20});\n\
             \x20\x20return result;\n\
             }\n\
             module.exports = { stringify: stringify, encode: stringify, parse: parse, decode: parse, escape: escape, unescape: unescape };\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "sys" => Some(
            "module.exports = require('node:util');\n",
        ),
        "domain" => Some(
            r#"var EventEmitter = require('node:events'), stack = [];
             function Domain() { if (!(this instanceof Domain)) return new Domain(); EventEmitter.call(this); this.members = []; }
             Domain.prototype = Object.create(EventEmitter.prototype); Domain.prototype.constructor = Domain;
             Domain.prototype.enter = function() { stack.push(module.exports.active); module.exports.active = this; if (globalThis.process) process.domain = this; return this; };
             Domain.prototype.exit = function() { if (module.exports.active !== this) return this; module.exports.active = stack.length ? stack.pop() : null; if (globalThis.process) process.domain = module.exports.active || null; return this; };
             Domain.prototype.add = function(emitter) { if (!emitter || (typeof emitter !== 'object' && typeof emitter !== 'function')) throw new TypeError('emitter must be an object'); if (emitter.domain && emitter.domain !== this && emitter.domain.remove) emitter.domain.remove(emitter); if (this.members.indexOf(emitter) < 0) this.members.push(emitter); emitter.domain = this; return this; };
             Domain.prototype.remove = function(emitter) { var index = this.members.indexOf(emitter); if (index >= 0) this.members.splice(index, 1); if (emitter && emitter.domain === this) emitter.domain = null; return this; };
             Domain.prototype._handle = function(error) { if (!(error instanceof Error)) error = new Error(String(error)); error.domain = this; error.domainThrown = true; if (this.listenerCount('error')) { this.emit('error', error); return; } throw error; };
             Domain.prototype.run = function(fn) { var args = Array.prototype.slice.call(arguments, 1); this.enter(); try { return fn.apply(undefined, args); } catch (error) { return this._handle(error); } finally { this.exit(); } };
             Domain.prototype.bind = function(fn) { if (typeof fn !== 'function') throw new TypeError('callback must be a function'); var domain = this; function bound() { var args = arguments, receiver = this; return domain.run(function() { return fn.apply(receiver, args); }); } bound.domain = domain; return bound; };
             Domain.prototype.intercept = function(fn) { if (typeof fn !== 'function') throw new TypeError('callback must be a function'); var domain = this; function intercepted(error) { if (error) return domain._handle(error); var args = Array.prototype.slice.call(arguments, 1), receiver = this; return domain.run(function() { return fn.apply(receiver, args); }); } intercepted.domain = domain; return intercepted; };
             Domain.prototype.dispose = function() { this.exit(); this.members.slice().forEach(this.remove, this); this.removeAllListeners(); return this; };
             function create() { return new Domain(); }
             module.exports = { Domain: Domain, create: create, createDomain: create, active: null }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "events" => Some(
            "function EventEmitter() {\n\
             \x20\x20if (!(this instanceof EventEmitter)) return new EventEmitter();\n\
             \x20\x20this._events = Object.create(null); this._maxListeners = undefined;\n\
             }\n\
             EventEmitter.prototype._add = function(event, listener, prepend, once) {\n\
             \x20\x20if (typeof listener !== 'function') throw new TypeError('listener must be a function');\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name] || (this._events[name] = []);\n\
             \x20\x20var entry = { listener: listener, once: Boolean(once) };\n\
             \x20\x20if (prepend) list.unshift(entry); else list.push(entry);\n\
             \x20\x20return this;\n\
             };\n\
             EventEmitter.prototype.addListener = EventEmitter.prototype.on = function(event, listener) { return this._add(event, listener, false, false); };\n\
             EventEmitter.prototype.once = function(event, listener) { return this._add(event, listener, false, true); };\n\
             EventEmitter.prototype.prependListener = function(event, listener) { return this._add(event, listener, true, false); };\n\
             EventEmitter.prototype.prependOnceListener = function(event, listener) { return this._add(event, listener, true, true); };\n\
             EventEmitter.prototype.emit = function(event) {\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name];\n\
             \x20\x20if (!list || list.length === 0) {\n\
             \x20\x20\x20\x20if (name === 'error') { var error = arguments[1]; throw error instanceof Error ? error : new Error('Unhandled error event'); }\n\
             \x20\x20\x20\x20return false;\n\
             \x20\x20}\n\
             \x20\x20var args = Array.prototype.slice.call(arguments, 1);\n\
             \x20\x20list.slice().forEach(function(entry) {\n\
             \x20\x20\x20\x20if (entry.once) this.removeListener(name, entry.listener);\n\
             \x20\x20\x20\x20entry.listener.apply(this, args);\n\
             \x20\x20}, this);\n\
             \x20\x20return true;\n\
             };\n\
             EventEmitter.prototype.removeListener = EventEmitter.prototype.off = function(event, listener) {\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name]; if (!list) return this;\n\
             \x20\x20for (var index = list.length - 1; index >= 0; index--) if (list[index].listener === listener || list[index].listener.listener === listener) { list.splice(index, 1); break; }\n\
             \x20\x20if (list.length === 0) delete this._events[name]; return this;\n\
             };\n\
             EventEmitter.prototype.removeAllListeners = function(event) { if (event === undefined) this._events = Object.create(null); else delete this._events[typeof event === 'symbol' ? event : String(event)]; return this; };\n\
             EventEmitter.prototype.listeners = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)] || []; return list.map(function(entry) { return entry.listener.listener || entry.listener; }); };\n\
             EventEmitter.prototype.rawListeners = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)] || []; return list.map(function(entry) { return entry.listener; }); };\n\
             EventEmitter.prototype.listenerCount = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)]; return list ? list.length : 0; };\n\
             EventEmitter.prototype.eventNames = function() { return Reflect.ownKeys(this._events); };\n\
             EventEmitter.prototype.setMaxListeners = function(value) { value = Number(value); if (!Number.isFinite(value) || value < 0) throw new RangeError('The value of n is out of range'); this._maxListeners = value; return this; }; EventEmitter.prototype.getMaxListeners = function() { return this._maxListeners === undefined ? EventEmitter.defaultMaxListeners : this._maxListeners; }; EventEmitter.defaultMaxListeners = 10;\n\
             EventEmitter.listenerCount = function(emitter, event) { return emitter.listenerCount(event); };\n\
             function once(emitter, event, options) {\n\
             \x20\x20return new Promise(function(resolve, reject) {\n\
             \x20\x20\x20\x20function cleanup() { emitter.removeListener(event, done); emitter.removeListener('error', failed); if (options && options.signal) options.signal.removeEventListener('abort', aborted); }\n\
             \x20\x20\x20\x20function done() { var values = Array.prototype.slice.call(arguments); cleanup(); resolve(values); }\n\
             \x20\x20\x20\x20function failed(error) { cleanup(); reject(error); }\n\
             \x20\x20\x20\x20function aborted() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; cleanup(); reject(error); }\n\
             \x20\x20\x20\x20if (options && options.signal && options.signal.aborted) { aborted(); return; } emitter.once(event, done); if (event !== 'error') emitter.once('error', failed); if (options && options.signal) options.signal.addEventListener('abort', aborted, { once: true });\n\
             \x20\x20});\n\
             }\n\
             function on(emitter, event, options) { options = options || {}; var values = [], waiters = [], ended = false, failure; function received() { var value = Array.prototype.slice.call(arguments); if (waiters.length) waiters.shift().resolve({ value: value, done: false }); else values.push(value); } function fail(error) { failure = error; finish(); } function finish() { if (ended) return; ended = true; emitter.removeListener(event, received); if (event !== 'error') emitter.removeListener('error', fail); while (waiters.length) { var waiter = waiters.shift(); if (failure) waiter.reject(failure); else waiter.resolve({ value: undefined, done: true }); } } emitter.on(event, received); if (event !== 'error') emitter.on('error', fail); if (options.signal) { var abort = function() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; failure = error; finish(); }; if (options.signal.aborted) abort(); else options.signal.addEventListener('abort', abort, { once: true }); } return { next: function() { if (values.length) return Promise.resolve({ value: values.shift(), done: false }); if (failure) return Promise.reject(failure); if (ended) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { waiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { finish(); return Promise.resolve({ value: undefined, done: true }); }, throw: function(error) { failure = error; finish(); return Promise.reject(error); }, [Symbol.asyncIterator]: function() { return this; } }; }\n\
             function getEventListeners(emitter, event) { return typeof emitter.listeners === 'function' ? emitter.listeners(event) : []; } function getMaxListeners(emitter) { return typeof emitter.getMaxListeners === 'function' ? emitter.getMaxListeners() : EventEmitter.defaultMaxListeners; } function setMaxListeners(value) { var emitters = Array.prototype.slice.call(arguments, 1); value = Number(value); if (!Number.isFinite(value) || value < 0) throw new RangeError('The value of n is out of range'); if (!emitters.length) EventEmitter.defaultMaxListeners = value; else emitters.forEach(function(emitter) { emitter.setMaxListeners(value); }); }\n\
             module.exports = EventEmitter;\n\
             module.exports.EventEmitter = EventEmitter;\n\
             module.exports.once = once;\n\
             module.exports.on = on; module.exports.getEventListeners = getEventListeners; module.exports.getMaxListeners = getMaxListeners; module.exports.setMaxListeners = setMaxListeners; module.exports.errorMonitor = Symbol.for('events.errorMonitor'); module.exports.captureRejectionSymbol = Symbol.for('nodejs.rejection');\n\
             module.exports.default = EventEmitter;\n\
             module.exports.__esModule = true;\n",
        ),
        "assert" | "assert/strict" => Some(
            "function AssertionError(options) {\n\
             \x20\x20options = options || {}; this.name = 'AssertionError'; this.code = 'ERR_ASSERTION';\n\
             \x20\x20this.actual = options.actual; this.expected = options.expected; this.operator = options.operator;\n\
             \x20\x20this.generatedMessage = options.message === undefined;\n\
             \x20\x20this.message = options.message === undefined ? 'Expected values to satisfy ' + (options.operator || 'assertion') : String(options.message);\n\
             \x20\x20if (Error.captureStackTrace) Error.captureStackTrace(this, options.stackStartFn || AssertionError);\n\
             }\n\
             AssertionError.prototype = Object.create(Error.prototype); AssertionError.prototype.constructor = AssertionError;\n\
             function failure(actual, expected, message, operator, start) { throw new AssertionError({ actual: actual, expected: expected, message: message, operator: operator, stackStartFn: start }); }\n\
             function deep(actual, expected, seen) {\n\
             \x20\x20if (Object.is(actual, expected)) return true;\n\
             \x20\x20if (actual === null || expected === null || typeof actual !== 'object' || typeof expected !== 'object') return false;\n\
             \x20\x20if (Object.getPrototypeOf(actual) !== Object.getPrototypeOf(expected)) return false;\n\
             \x20\x20seen = seen || new Map(); if (seen.get(actual) === expected) return true; seen.set(actual, expected);\n\
             \x20\x20if (actual instanceof Date) return expected instanceof Date && actual.getTime() === expected.getTime();\n\
             \x20\x20if (actual instanceof RegExp) return expected instanceof RegExp && actual.source === expected.source && actual.flags === expected.flags;\n\
             \x20\x20if (ArrayBuffer.isView(actual)) { if (!ArrayBuffer.isView(expected) || actual.length !== expected.length) return false; for (var i = 0; i < actual.length; i++) if (!Object.is(actual[i], expected[i])) return false; return true; }\n\
             \x20\x20var left = Object.keys(actual); var right = Object.keys(expected); if (left.length !== right.length) return false;\n\
             \x20\x20for (var index = 0; index < left.length; index++) { var key = left[index]; if (!Object.prototype.hasOwnProperty.call(expected, key) || !deep(actual[key], expected[key], seen)) return false; }\n\
             \x20\x20return true;\n\
             }\n\
             function ok(value, message) { if (!value) failure(value, true, message, '==', ok); }\n\
             function equal(actual, expected, message) { if (actual != expected) failure(actual, expected, message, '==', equal); }\n\
             function notEqual(actual, expected, message) { if (actual == expected) failure(actual, expected, message, '!=', notEqual); }\n\
             function strictEqual(actual, expected, message) { if (!Object.is(actual, expected)) failure(actual, expected, message, 'strictEqual', strictEqual); }\n\
             function notStrictEqual(actual, expected, message) { if (Object.is(actual, expected)) failure(actual, expected, message, 'notStrictEqual', notStrictEqual); }\n\
             function deepStrictEqual(actual, expected, message) { if (!deep(actual, expected)) failure(actual, expected, message, 'deepStrictEqual', deepStrictEqual); }\n\
             function notDeepStrictEqual(actual, expected, message) { if (deep(actual, expected)) failure(actual, expected, message, 'notDeepStrictEqual', notDeepStrictEqual); }\n\
             function fail(message) { failure(undefined, undefined, message || 'Failed', 'fail', fail); }\n\
             function matches(error, expected) { if (expected === undefined) return true; if (expected instanceof RegExp) return expected.test(String(error && error.message || error)); if (typeof expected === 'function') return error instanceof expected || expected(error) === true; return true; }\n\
             function throws(block, expected, message) { var caught; try { block(); } catch (error) { caught = error; } if (caught === undefined || !matches(caught, expected)) failure(caught, expected, message, 'throws', throws); return caught; }\n\
             function doesNotThrow(block, expected, message) { try { block(); } catch (error) { if (matches(error, expected)) failure(error, undefined, message, 'doesNotThrow', doesNotThrow); throw error; } }\n\
             ok.AssertionError = AssertionError; ok.ok = ok; ok.equal = equal; ok.notEqual = notEqual; ok.strictEqual = strictEqual; ok.notStrictEqual = notStrictEqual;\n\
             ok.deepEqual = deepStrictEqual; ok.notDeepEqual = notDeepStrictEqual; ok.deepStrictEqual = deepStrictEqual; ok.notDeepStrictEqual = notDeepStrictEqual;\n\
             ok.fail = fail; ok.throws = throws; ok.doesNotThrow = doesNotThrow; ok.strict = ok; ok.default = ok; ok.__esModule = true;\n\
             module.exports = ok;\n",
        ),
        // Same real dependency chain as `path` above (`node-gyp-build.js`
        // reads `os.arch()`/`os.platform()` to build its target string).
        "os" => Some(
            "var info = JSON.parse(__thaw_os_info()); var __thaw_os = { arch: function() { return info.arch; }, platform: function() { return info.platform; }, type: function() { return info.type; }, tmpdir: function() { return info.tmpdir; }, homedir: function() { return info.homedir; }, hostname: function() { return info.hostname; }, release: function() { return info.release; }, version: function() { return info.version; }, machine: function() { return info.machine; }, endianness: function() { return info.endianness; }, cpus: function() { return structuredClone(info.cpus); }, totalmem: function() { return info.totalmem; }, freemem: function() { return info.freemem; }, uptime: function() { return info.uptime; }, loadavg: function() { return info.loadavg.slice(); }, userInfo: function() { return structuredClone(info.userInfo); }, networkInterfaces: function() { return { lo: [{ address: '127.0.0.1', netmask: '255.0.0.0', family: 'IPv4', mac: '00:00:00:00:00:00', internal: true, cidr: '127.0.0.1/8' }, { address: '::1', netmask: 'ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff', family: 'IPv6', mac: '00:00:00:00:00:00', internal: true, cidr: '::1/128', scopeid: 0 }] }; }, getPriority: function() { return 0; }, setPriority: function() {}, EOL: info.platform === 'win32' ? '\\r\\n' : '\\n', devNull: info.platform === 'win32' ? '\\\\\\\\.\\\\nul' : '/dev/null', constants: { signals: { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGILL: 4, SIGTRAP: 5, SIGABRT: 6, SIGBUS: 7, SIGFPE: 8, SIGKILL: 9, SIGUSR1: 10, SIGSEGV: 11, SIGUSR2: 12, SIGPIPE: 13, SIGALRM: 14, SIGTERM: 15 }, errno: { EACCES: 13, EADDRINUSE: 98, ECONNREFUSED: 111, EEXIST: 17, EINVAL: 22, ENOENT: 2, ENOMEM: 12, ENOTDIR: 20, ETIMEDOUT: 110 } } };\n\
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
            "var stream = require('node:stream'); function pathValue(path) { return path instanceof URL ? decodeURIComponent(path.pathname) : String(path); } function invoke(operation, path, value, recursive) { path = pathValue(path); var record = JSON.parse(globalThis.__thaw_fs(operation, path, value || '', Boolean(recursive))); if (!record.ok) { var error = new Error(record.code + ': ' + record.message + ', ' + operation + ' ' + path); error.code = record.code; error.errno = -1; error.path = path; error.syscall = operation; throw error; } return record; } function encoding(options) { return typeof options === 'string' ? options : options && options.encoding; } function data(value, options) { return Buffer.isBuffer(value) || value instanceof Uint8Array ? Buffer.from(value) : Buffer.from(String(value), encoding(options)); } function exclusiveError(path, syscall) { var error = new Error('EEXIST: path already exists, ' + syscall + ' ' + pathValue(path)); error.code = 'EEXIST'; error.errno = -1; error.path = pathValue(path); error.syscall = syscall; return error; } function timeValue(value) { if (value instanceof Date) return value.getTime() / 1000; var number = Number(value); if (Number.isFinite(number)) return number; return new Date(value).getTime() / 1000; } function Stats(record, bigint) { var convert = bigint ? function(value) { return BigInt(Math.trunc(Number(value))); } : Number, numeric = ['dev','ino','mode','nlink','uid','gid','rdev','blksize','blocks']; this.size = convert(record.length); for (var index = 0; index < numeric.length; index++) this[numeric[index]] = convert(record[numeric[index]]); var atime = Number(record.atimeMs), mtime = Number(record.mtimeMs), ctime = Number(record.ctimeMs), birthtime = Number(record.birthtimeMs); this.atimeMs = convert(atime); this.mtimeMs = convert(mtime); this.ctimeMs = convert(ctime); this.birthtimeMs = convert(birthtime); if (bigint) { this.atimeNs = BigInt(Math.round(atime * 1000000)); this.mtimeNs = BigInt(Math.round(mtime * 1000000)); this.ctimeNs = BigInt(Math.round(ctime * 1000000)); this.birthtimeNs = BigInt(Math.round(birthtime * 1000000)); } this.atime = new Date(atime); this.mtime = new Date(mtime); this.ctime = new Date(ctime); this.birthtime = new Date(birthtime); this._file = record.file; this._directory = record.directory; this._symlink = Boolean(record.symlink); } Stats.prototype.isFile = function() { return this._file; }; Stats.prototype.isDirectory = function() { return this._directory; }; Stats.prototype.isSymbolicLink = function() { return this._symlink; }; Stats.prototype.isSocket = Stats.prototype.isFIFO = Stats.prototype.isCharacterDevice = Stats.prototype.isBlockDevice = function() { return false; };\n\
             var __thaw_fs = { existsSync: function(path) { return invoke('exists', path).exists; }, readFileSync: function(path, options) { var output = Buffer.from(invoke('read', path).data, 'hex'), target = encoding(options); return target ? output.toString(target) : output; }, writeFileSync: function(path, value, options) { var flag = options && typeof options === 'object' && options.flag || 'w'; if (String(flag).indexOf('x') >= 0 && __thaw_fs.existsSync(path)) throw exclusiveError(path, 'open'); invoke(String(flag)[0] === 'a' ? 'append' : 'write', path, data(value, options).toString('hex')); }, appendFileSync: function(path, value, options) { var flag = options && typeof options === 'object' && options.flag || 'a'; if (String(flag).indexOf('x') >= 0 && __thaw_fs.existsSync(path)) throw exclusiveError(path, 'open'); invoke('append', path, data(value, options).toString('hex')); }, mkdirSync: function(path, options) { invoke('mkdir', path, '', options && options.recursive); }, readdirSync: function(path, options) { options = options || {}; var root = pathValue(path), output = [], targetEncoding = encoding(options); function visit(directory, relative) { invoke('readdir', directory).entries.forEach(function(rawName) { var full = pathValue(directory).replace(/[/]$/, '') + '/' + rawName, name = relative ? relative + '/' + rawName : rawName, stat = invoke('lstat', full); if (options.withFileTypes) { var encodedName = targetEncoding === 'buffer' ? Buffer.from(rawName) : rawName; output.push({ name: encodedName, parentPath: pathValue(directory), path: pathValue(directory), isFile: function() { return stat.file; }, isDirectory: function() { return stat.directory; }, isSymbolicLink: function() { return Boolean(stat.symlink); } }); } else output.push(targetEncoding === 'buffer' ? Buffer.from(name) : name); if (options.recursive && stat.directory && !stat.symlink) visit(full, name); }); } visit(root, ''); return output; }, statSync: function(path, options) { return new Stats(invoke('stat', path), Boolean(options && options.bigint)); }, lstatSync: function(path, options) { return new Stats(invoke('lstat', path), Boolean(options && options.bigint)); }, unlinkSync: function(path) { invoke('unlink', path); }, rmSync: function(path, options) { var stat; try { stat = invoke('stat', path); } catch (error) { if (options && options.force && error.code === 'ENOENT') return; throw error; } invoke(stat.directory ? 'rmdir' : 'unlink', path, '', options && options.recursive); }, rmdirSync: function(path, options) { invoke('rmdir', path, '', options && options.recursive); }, renameSync: function(oldPath, newPath) { invoke('rename', oldPath, pathValue(newPath)); } };\n\
             __thaw_fs.copyFileSync = function(source, destination, mode) { destination = pathValue(destination); if ((Number(mode) & __thaw_fs.constants.COPYFILE_EXCL) && __thaw_fs.existsSync(destination)) { var error = new Error('EEXIST: destination already exists, copyfile ' + destination); error.code = 'EEXIST'; error.errno = -1; error.path = destination; error.syscall = 'copyfile'; throw error; } invoke('copy', source, destination); }; __thaw_fs.cpSync = function(source, destination, options) { options = options || {}; destination = pathValue(destination); if (__thaw_fs.existsSync(destination)) { if (options.force === false) { if (options.errorOnExist) { var error = new Error('EEXIST: destination already exists, cp ' + destination); error.code = 'EEXIST'; error.path = destination; error.syscall = 'cp'; throw error; } return; } __thaw_fs.rmSync(destination, { recursive: true, force: true }); } invoke('cp', source, destination, Boolean(options.recursive)); }; __thaw_fs.realpathSync = function(path, options) { var result = invoke('realpath', path).path; return encoding(options) === 'buffer' ? Buffer.from(result) : result; }; __thaw_fs.realpathSync.native = __thaw_fs.realpathSync; __thaw_fs.mkdtempSync = function(prefix, options) { var result = invoke('mkdtemp', prefix).path; return encoding(options) === 'buffer' ? Buffer.from(result) : result; }; __thaw_fs.mkdtempDisposableSync = function(prefix, options) { var path = __thaw_fs.mkdtempSync(prefix, options), removed = false, disposable = { path: path, remove: function() { if (!removed) { __thaw_fs.rmSync(path, { recursive: true, force: true }); removed = true; } } }; if (Symbol.dispose) disposable[Symbol.dispose] = disposable.remove; return disposable; }; function protectFileBlob(blob, verify) { var readArrayBuffer = blob.arrayBuffer.bind(blob), readBytes = blob.bytes.bind(blob), readText = blob.text.bind(blob), makeSlice = blob.slice.bind(blob), makeStream = blob.stream.bind(blob); blob.arrayBuffer = function() { return Promise.resolve().then(verify).then(readArrayBuffer); }; blob.bytes = function() { return Promise.resolve().then(verify).then(readBytes); }; blob.text = function() { return Promise.resolve().then(verify).then(readText); }; blob.slice = function(start, end, type) { return protectFileBlob(makeSlice(start, end, type), verify); }; blob.stream = function() { var stream = makeStream(); return { getReader: function() { var reader = stream.getReader(); return { read: function() { return Promise.resolve().then(verify).then(function() { return reader.read(); }); }, releaseLock: function() { return reader.releaseLock(); } }; }, [Symbol.asyncIterator]: function() { var iterator = stream[Symbol.asyncIterator](); return { next: function() { return Promise.resolve().then(verify).then(function() { return iterator.next(); }); }, [Symbol.asyncIterator]: function() { return this; } }; } }; }; return blob; } __thaw_fs.openAsBlob = function(path, options) { return Promise.resolve().then(function() { var initial = __thaw_fs.statSync(path), blob = new Blob([__thaw_fs.readFileSync(path)], { type: options && options.type }); function verify() { var current; try { current = __thaw_fs.statSync(path); } catch (error) { throw new DOMException('The blob could not be read', 'NotReadableError'); } if (current.size !== initial.size || current.mtimeMs !== initial.mtimeMs) throw new DOMException('The blob could not be read', 'NotReadableError'); } return protectFileBlob(blob, verify); }); };\n\
             __thaw_fs.linkSync = function(existingPath, newPath) { invoke('link', existingPath, pathValue(newPath)); }; __thaw_fs.symlinkSync = function(target, path) { invoke('symlink', target, pathValue(path)); }; __thaw_fs.readlinkSync = function(path, options) { var result = invoke('readlink', path).path; return encoding(options) === 'buffer' ? Buffer.from(result) : result; }; __thaw_fs.chmodSync = function(path, mode) { if (typeof mode === 'string') mode = parseInt(mode, 8); invoke('chmod', path, String(Number(mode))); }; __thaw_fs.utimesSync = function(path, atime, mtime) { invoke('utimes', path, String(timeValue(atime)) + ',' + String(timeValue(mtime))); }; __thaw_fs.lutimesSync = function(path, atime, mtime) { invoke('lutimes', path, String(timeValue(atime)) + ',' + String(timeValue(mtime))); }; __thaw_fs.chownSync = function(path, uid, gid) { invoke('chown', path, String(Number(uid)) + ',' + String(Number(gid))); }; __thaw_fs.lchownSync = function(path, uid, gid) { invoke('lchown', path, String(Number(uid)) + ',' + String(Number(gid))); }; __thaw_fs.truncateSync = function(path, length) { invoke('truncate', path, String(length === undefined ? 0 : Number(length))); }; __thaw_fs.accessSync = function(path, mode) { invoke('access', path, String(mode === undefined ? __thaw_fs.constants.F_OK : Number(mode))); }; function StatFs(record, bigint) { var convert = bigint ? function(value) { return BigInt(Math.trunc(Number(value))); } : Number; this.type = convert(record.type); this.bsize = convert(record.bsize); this.blocks = convert(record.blocks); this.bfree = convert(record.bfree); this.bavail = convert(record.bavail); this.files = convert(record.files); this.ffree = convert(record.ffree); } __thaw_fs.StatFs = StatFs; __thaw_fs.statfsSync = function(path, options) { return new StatFs(invoke('statfs', path), Boolean(options && options.bigint)); }; var statWatchers = new Map(); function StatWatcher(path, options) { this.path = pathValue(path); this._listeners = []; this._refed = !options || options.persistent !== false; this._previous = __thaw_fs.statSync(this.path); var self = this; this._timer = setInterval(function() { var current; try { current = __thaw_fs.statSync(self.path); } catch (error) { return; } var previous = self._previous; if (current.mtimeMs !== previous.mtimeMs || current.ctimeMs !== previous.ctimeMs || current.size !== previous.size) { self._previous = current; self._listeners.slice().forEach(function(listener) { listener(current, previous); }); } }, Math.max(1, Number(options && options.interval || 5007))); } StatWatcher.prototype.ref = function() { this._refed = true; return this; }; StatWatcher.prototype.unref = function() { this._refed = false; return this; }; StatWatcher.prototype.hasRef = function() { return this._refed; }; StatWatcher.prototype.stop = function() { if (this._timer !== null) { clearInterval(this._timer); this._timer = null; statWatchers.delete(this.path); } return this; }; __thaw_fs.StatWatcher = StatWatcher; __thaw_fs.watchFile = function(path, options, listener) { if (typeof options === 'function') { listener = options; options = {}; } if (typeof listener !== 'function') throw new TypeError('listener must be a function'); path = pathValue(path); var watcher = statWatchers.get(path); if (!watcher) { watcher = new StatWatcher(path, options || {}); statWatchers.set(path, watcher); } if (watcher._listeners.indexOf(listener) < 0) watcher._listeners.push(listener); return watcher; }; __thaw_fs.unwatchFile = function(path, listener) { var watcher = statWatchers.get(pathValue(path)); if (!watcher) return; if (typeof listener === 'function') watcher._listeners = watcher._listeners.filter(function(value) { return value !== listener; }); else watcher._listeners = []; if (!watcher._listeners.length) watcher.stop(); };\n\
             function Dir(path) { this.path = pathValue(path); this.closed = false; this._index = 0; this._entries = __thaw_fs.readdirSync(this.path, { withFileTypes: true }); } Dir.prototype._check = function() { if (this.closed) { var error = new Error('Directory handle was closed'); error.code = 'ERR_DIR_CLOSED'; throw error; } }; Dir.prototype.readSync = function() { this._check(); return this._index < this._entries.length ? this._entries[this._index++] : null; }; Dir.prototype.read = function(callback) { var self = this, operation = Promise.resolve().then(function() { return self.readSync(); }); if (typeof callback === 'function') { operation.then(function(value) { callback(null, value); }, callback); return; } return operation; }; Dir.prototype.closeSync = function() { this._check(); this.closed = true; }; Dir.prototype.close = function(callback) { var self = this, operation = Promise.resolve().then(function() { self.closeSync(); }); if (typeof callback === 'function') { operation.then(function() { callback(null); }, callback); return; } return operation; }; Dir.prototype[Symbol.asyncIterator] = function() { var self = this; return { next: function() { return self.read().then(function(value) { if (value === null) { if (!self.closed) self.closeSync(); return { value: undefined, done: true }; } return { value: value, done: false }; }); } }; }; __thaw_fs.Dir = Dir; __thaw_fs.opendirSync = function(path) { invoke('readdir', path); return new Dir(path); };\n\
             function ReadStream(path, options) { if (!(this instanceof ReadStream)) return new ReadStream(path, options); options = options || {}; stream.Readable.call(this, options); this.path = pathValue(path); this.pending = true; this.fd = null; this.bytesRead = 0; this.close = this.destroy.bind(this); var self = this; queueMicrotask(function() { try { var value = Buffer.from(invoke('read', self.path).data, 'hex'), start = options.start === undefined ? 0 : Number(options.start), end = options.end === undefined ? value.length - 1 : Number(options.end), selected = value.subarray(start, Math.min(value.length, end + 1)), size = Math.max(1, Number(options.highWaterMark || 65536)); self.fd = 1; self.pending = false; self.emit('open', self.fd); self.emit('ready'); for (var offset = 0; offset < selected.length; offset += size) { var chunk = selected.subarray(offset, offset + size); self.bytesRead += chunk.length; self.push(chunk); } self.push(null); if (options.autoClose !== false) queueMicrotask(function() { self.fd = null; self.emit('close'); }); } catch (error) { self.pending = false; self.destroy(error); } }); } ReadStream.prototype = Object.create(stream.Readable.prototype); ReadStream.prototype.constructor = ReadStream;\n\
             function WriteStream(path, options) { if (!(this instanceof WriteStream)) return new WriteStream(path, options); options = options || {}; this.path = pathValue(path); this.pending = true; this.fd = null; this.bytesWritten = 0; var chunks = [], self = this; stream.Writable.call(this, { write: function(chunk, encoding, callback) { chunks.push(Buffer.from(chunk)); self.bytesWritten += chunk.length; callback(); }, final: function(callback) { try { var value = Buffer.concat(chunks); invoke(options.flags === 'a' || options.flags === 'ax' ? 'append' : 'write', self.path, value.toString('hex')); callback(); if (options.autoClose !== false) queueMicrotask(function() { self.fd = null; self.emit('close'); }); } catch (error) { callback(error); } } }); queueMicrotask(function() { self.fd = 1; self.pending = false; self.emit('open', self.fd); self.emit('ready'); }); } WriteStream.prototype = Object.create(stream.Writable.prototype); WriteStream.prototype.constructor = WriteStream; WriteStream.prototype.close = function(callback) { if (!this.writableEnded) this.end(callback); else if (callback) queueMicrotask(callback); }; __thaw_fs.ReadStream = ReadStream; __thaw_fs.WriteStream = WriteStream; __thaw_fs.createReadStream = function(path, options) { return new ReadStream(path, options); }; __thaw_fs.createWriteStream = function(path, options) { return new WriteStream(path, options); };\n\
             var fileDescriptors = new Map(), nextFileHandle = 10; function FileHandle(path, flags) { this.path = pathValue(path); this.flags = String(flags || 'r'); this.fd = nextFileHandle++; this.closed = false; this._position = 0; if (this.flags.indexOf('x') >= 0 && __thaw_fs.existsSync(this.path)) throw exclusiveError(this.path, 'open'); if (this.flags[0] === 'w') invoke('write', this.path, ''); else if (this.flags[0] === 'a') invoke('append', this.path, ''); else invoke('stat', this.path); fileDescriptors.set(this.fd, this); } FileHandle.prototype._check = function() { if (this.closed) { var error = new Error('file closed'); error.code = 'EBADF'; throw error; } }; FileHandle.prototype.close = function() { if (!this.closed) { fileDescriptors.delete(this.fd); this.closed = true; this.fd = -1; } return Promise.resolve(); }; FileHandle.prototype.readFile = function(options) { var self = this; return Promise.resolve().then(function() { self._check(); return __thaw_fs.readFileSync(self.path, options); }); }; FileHandle.prototype.writeFile = function(value, options) { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.writeFileSync(self.path, value, options); }); }; FileHandle.prototype.appendFile = function(value, options) { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.appendFileSync(self.path, value, options); }); }; FileHandle.prototype.read = function(buffer, offset, length, position) { var self = this; if (buffer && !Buffer.isBuffer(buffer) && !(buffer instanceof Uint8Array)) { var options = buffer; buffer = options.buffer || Buffer.alloc(options.length || 16384); offset = options.offset; length = options.length; position = options.position; } buffer = buffer || Buffer.alloc(16384); offset = offset === undefined ? 0 : Number(offset); length = length === undefined ? buffer.length - offset : Number(length); return Promise.resolve().then(function() { self._check(); var source = __thaw_fs.readFileSync(self.path), start = position === null || position === undefined ? self._position : Number(position), count = Math.max(0, Math.min(length, source.length - start)); if (count) buffer.set(source.subarray(start, start + count), offset); if (position === null || position === undefined) self._position += count; return { bytesRead: count, buffer: buffer }; }); }; FileHandle.prototype.write = function(value, offset, length, position) { var self = this, stringValue = typeof value === 'string', resultValue = value; if (stringValue) { var encodingName = typeof length === 'string' ? length : 'utf8'; position = offset; value = Buffer.from(value, encodingName); offset = 0; length = value.length; } else { value = Buffer.from(value); resultValue = value; offset = offset === undefined ? 0 : Number(offset); length = length === undefined ? value.length - offset : Number(length); } return Promise.resolve().then(function() { self._check(); var original = __thaw_fs.readFileSync(self.path), chunk = value.subarray(offset, offset + length), start = position === null || position === undefined ? (self.flags[0] === 'a' ? original.length : self._position) : Number(position), size = Math.max(original.length, start + chunk.length), output = Buffer.alloc(size); output.set(original, 0); output.set(chunk, start); __thaw_fs.writeFileSync(self.path, output); if (position === null || position === undefined) self._position = start + chunk.length; return { bytesWritten: chunk.length, buffer: resultValue }; }); }; FileHandle.prototype.stat = function(options) { var self = this; return Promise.resolve().then(function() { self._check(); return __thaw_fs.statSync(self.path, options); }); }; FileHandle.prototype.truncate = function(length) { var self = this; return Promise.resolve().then(function() { self._check(); invoke('truncate', self.path, String(length === undefined ? 0 : length)); }); }; FileHandle.prototype.createReadStream = function(options) { this._check(); return new ReadStream(this.path, options); }; FileHandle.prototype.createWriteStream = function(options) { this._check(); return new WriteStream(this.path, Object.assign({ flags: this.flags }, options || {})); }; FileHandle.prototype[Symbol.asyncDispose] = FileHandle.prototype.close; __thaw_fs.FileHandle = FileHandle;\n\
             __thaw_fs.constants = globalThis.__thaw_fs_constants || (globalThis.__thaw_fs_constants = { F_OK: 0, X_OK: 1, W_OK: 2, R_OK: 4, O_RDONLY: 0, O_WRONLY: 1, O_RDWR: 2, O_CREAT: 64, O_EXCL: 128, O_NOCTTY: 256, O_TRUNC: 512, O_APPEND: 1024, O_DIRECTORY: 65536, O_NOFOLLOW: 131072, O_SYNC: 1052672, S_IFMT: 61440, S_IFREG: 32768, S_IFDIR: 16384, S_IFCHR: 8192, S_IFBLK: 24576, S_IFIFO: 4096, S_IFLNK: 40960, S_IFSOCK: 49152, COPYFILE_EXCL: 1, COPYFILE_FICLONE: 2, COPYFILE_FICLONE_FORCE: 4 });\n\
             function fdHandle(fd) { var handle = fileDescriptors.get(Number(fd)); if (!handle || handle.closed) { var error = new Error('EBADF: bad file descriptor'); error.code = 'EBADF'; error.errno = -1; error.syscall = 'fd'; throw error; } return handle; } __thaw_fs.openSync = function(path, flags) { return new FileHandle(path, flags).fd; }; __thaw_fs.closeSync = function(fd) { var handle = fdHandle(fd); fileDescriptors.delete(handle.fd); handle.closed = true; handle.fd = -1; }; __thaw_fs.readSync = function(fd, buffer, offset, length, position) { var handle = fdHandle(fd), source = __thaw_fs.readFileSync(handle.path), start = position === null || position === undefined ? handle._position : Number(position), count = Math.max(0, Math.min(Number(length), source.length - start)); if (count) buffer.set(source.subarray(start, start + count), Number(offset)); if (position === null || position === undefined) handle._position += count; return count; }; __thaw_fs.writeSync = function(fd, value, offset, length, position) { var handle = fdHandle(fd), stringValue = typeof value === 'string'; if (stringValue) { var encodingName = typeof length === 'string' ? length : 'utf8'; position = offset; value = Buffer.from(value, encodingName); offset = 0; length = value.length; } else { value = Buffer.from(value); offset = offset === undefined ? 0 : Number(offset); length = length === undefined ? value.length - offset : Number(length); } var original = __thaw_fs.readFileSync(handle.path), chunk = value.subarray(offset, offset + length), start = position === null || position === undefined ? (handle.flags[0] === 'a' ? original.length : handle._position) : Number(position), output = Buffer.alloc(Math.max(original.length, start + chunk.length)); output.set(original); output.set(chunk, start); __thaw_fs.writeFileSync(handle.path, output); if (position === null || position === undefined) handle._position = start + chunk.length; return chunk.length; }; __thaw_fs.fstatSync = function(fd, options) { return __thaw_fs.statSync(fdHandle(fd).path, options); }; __thaw_fs.ftruncateSync = function(fd, length) { invoke('truncate', fdHandle(fd).path, String(length === undefined ? 0 : length)); }; __thaw_fs.fchmodSync = function(fd, mode) { __thaw_fs.chmodSync(fdHandle(fd).path, mode); }; __thaw_fs.fchownSync = function(fd, uid, gid) { __thaw_fs.chownSync(fdHandle(fd).path, uid, gid); }; __thaw_fs.futimesSync = function(fd, atime, mtime) { __thaw_fs.utimesSync(fdHandle(fd).path, atime, mtime); }; __thaw_fs.fsyncSync = __thaw_fs.fdatasyncSync = function(fd) { fdHandle(fd); }; __thaw_fs.readvSync = function(fd, buffers, position) { var total = 0, cursor = position; buffers.forEach(function(buffer) { var count = __thaw_fs.readSync(fd, buffer, 0, buffer.length, cursor); total += count; if (cursor !== null && cursor !== undefined) cursor = Number(cursor) + count; }); return total; }; __thaw_fs.writevSync = function(fd, buffers, position) { var total = 0, cursor = position; buffers.forEach(function(buffer) { var count = __thaw_fs.writeSync(fd, buffer, 0, buffer.length, cursor); total += count; if (cursor !== null && cursor !== undefined) cursor = Number(cursor) + count; }); return total; };;\n\
             function promised(method) { return function() { var args = arguments; return new Promise(function(resolve, reject) { queueMicrotask(function() { try { resolve(method.apply(__thaw_fs, args)); } catch (error) { reject(error); } }); }); }; } __thaw_fs.promises = { access: promised(__thaw_fs.accessSync), open: function(path, flags) { return promised(function() { return new FileHandle(path, flags); })(); }, readFile: promised(__thaw_fs.readFileSync), readdir: promised(__thaw_fs.readdirSync), opendir: promised(__thaw_fs.opendirSync), stat: promised(__thaw_fs.statSync), lstat: promised(__thaw_fs.lstatSync), writeFile: promised(__thaw_fs.writeFileSync), appendFile: promised(__thaw_fs.appendFileSync), mkdir: promised(__thaw_fs.mkdirSync), unlink: promised(__thaw_fs.unlinkSync), rm: promised(__thaw_fs.rmSync), rmdir: promised(__thaw_fs.rmdirSync), rename: promised(__thaw_fs.renameSync), copyFile: promised(__thaw_fs.copyFileSync), cp: promised(__thaw_fs.cpSync), realpath: promised(__thaw_fs.realpathSync), mkdtemp: promised(__thaw_fs.mkdtempSync), statfs: promised(__thaw_fs.statfsSync), lutimes: promised(__thaw_fs.lutimesSync), truncate: promised(__thaw_fs.truncateSync) }; __thaw_fs.promises.mkdtempDisposable = function(prefix, options) { return __thaw_fs.promises.mkdtemp(prefix, options).then(function(path) { var removal, disposable = { path: path, remove: function() { if (!removal) removal = __thaw_fs.promises.rm(path, { recursive: true, force: true }); return removal; } }; if (Symbol.asyncDispose) disposable[Symbol.asyncDispose] = disposable.remove; return disposable; }); }; function callbackMethod(method, valueResult) { return function() { var args = Array.prototype.slice.call(arguments), callback = args.pop(); if (typeof callback !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { var value = method.apply(__thaw_fs, args); if (valueResult) callback(null, value); else callback(null); } catch (error) { callback(error); } }); }; } __thaw_fs.readFile = callbackMethod(__thaw_fs.readFileSync, true); __thaw_fs.readdir = callbackMethod(__thaw_fs.readdirSync, true); __thaw_fs.opendir = callbackMethod(__thaw_fs.opendirSync, true); __thaw_fs.stat = callbackMethod(__thaw_fs.statSync, true); __thaw_fs.lstat = callbackMethod(__thaw_fs.lstatSync, true); __thaw_fs.writeFile = callbackMethod(__thaw_fs.writeFileSync, false); __thaw_fs.appendFile = callbackMethod(__thaw_fs.appendFileSync, false); __thaw_fs.mkdir = callbackMethod(__thaw_fs.mkdirSync, false); __thaw_fs.unlink = callbackMethod(__thaw_fs.unlinkSync, false); __thaw_fs.rm = callbackMethod(__thaw_fs.rmSync, false); __thaw_fs.rmdir = callbackMethod(__thaw_fs.rmdirSync, false); __thaw_fs.rename = callbackMethod(__thaw_fs.renameSync, false); __thaw_fs.copyFile = callbackMethod(__thaw_fs.copyFileSync, false); __thaw_fs.cp = callbackMethod(__thaw_fs.cpSync, false); __thaw_fs.realpath = callbackMethod(__thaw_fs.realpathSync, true); __thaw_fs.realpath.native = __thaw_fs.realpath; __thaw_fs.mkdtemp = callbackMethod(__thaw_fs.mkdtempSync, true);\n\
             FileHandle.prototype.utimes = function(atime, mtime) { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.utimesSync(self.path, atime, mtime); }); }; FileHandle.prototype.chmod = function(mode) { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.chmodSync(self.path, mode); }); }; FileHandle.prototype.sync = function() { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.fsyncSync(self.fd); }); }; FileHandle.prototype.datasync = function() { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.fdatasyncSync(self.fd); }); }; FileHandle.prototype.chown = function(uid, gid) { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.chownSync(self.path, uid, gid); }); }; Object.assign(__thaw_fs.promises, { link: promised(__thaw_fs.linkSync), symlink: promised(__thaw_fs.symlinkSync), readlink: promised(__thaw_fs.readlinkSync), chmod: promised(__thaw_fs.chmodSync), utimes: promised(__thaw_fs.utimesSync), chown: promised(__thaw_fs.chownSync), lchown: promised(__thaw_fs.lchownSync) }); __thaw_fs.link = callbackMethod(__thaw_fs.linkSync, false); __thaw_fs.symlink = callbackMethod(__thaw_fs.symlinkSync, false); __thaw_fs.readlink = callbackMethod(__thaw_fs.readlinkSync, true); __thaw_fs.chmod = callbackMethod(__thaw_fs.chmodSync, false); __thaw_fs.utimes = callbackMethod(__thaw_fs.utimesSync, false); __thaw_fs.lutimes = callbackMethod(__thaw_fs.lutimesSync, false); __thaw_fs.chown = callbackMethod(__thaw_fs.chownSync, false); __thaw_fs.lchown = callbackMethod(__thaw_fs.lchownSync, false); __thaw_fs.access = callbackMethod(__thaw_fs.accessSync, false); __thaw_fs.statfs = callbackMethod(__thaw_fs.statfsSync, true); __thaw_fs.truncate = callbackMethod(__thaw_fs.truncateSync, false); __thaw_fs.open = callbackMethod(__thaw_fs.openSync, true); __thaw_fs.close = callbackMethod(__thaw_fs.closeSync, false); __thaw_fs.fstat = callbackMethod(__thaw_fs.fstatSync, true); __thaw_fs.ftruncate = callbackMethod(__thaw_fs.ftruncateSync, false); __thaw_fs.fchmod = callbackMethod(__thaw_fs.fchmodSync, false); __thaw_fs.fchown = callbackMethod(__thaw_fs.fchownSync, false); __thaw_fs.futimes = callbackMethod(__thaw_fs.futimesSync, false); __thaw_fs.fsync = callbackMethod(__thaw_fs.fsyncSync, false); __thaw_fs.fdatasync = callbackMethod(__thaw_fs.fdatasyncSync, false); __thaw_fs.read = function(fd, buffer, offset, length, position, callback) { if (typeof callback !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { callback(null, __thaw_fs.readSync(fd, buffer, offset, length, position), buffer); } catch (error) { callback(error); } }); }; __thaw_fs.write = function(fd, value, offset, length, position, callback) { var args = Array.prototype.slice.call(arguments), done = args.pop(); if (typeof done !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { done(null, __thaw_fs.writeSync.apply(__thaw_fs, args), value); } catch (error) { done(error); } }); }; FileHandle.prototype.statfs = function(options) { var self = this; return Promise.resolve().then(function() { self._check(); return __thaw_fs.statfsSync(self.path, options); }); }; FileHandle.prototype.readv = function(buffers, position) { var self = this; return Promise.resolve().then(function() { self._check(); return { bytesRead: __thaw_fs.readvSync(self.fd, buffers, position), buffers: buffers }; }); }; FileHandle.prototype.writev = function(buffers, position) { var self = this; return Promise.resolve().then(function() { self._check(); return { bytesWritten: __thaw_fs.writevSync(self.fd, buffers, position), buffers: buffers }; }); }; __thaw_fs.readv = function(fd, buffers, position, callback) { if (typeof position === 'function') { callback = position; position = null; } if (typeof callback !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { callback(null, __thaw_fs.readvSync(fd, buffers, position), buffers); } catch (error) { callback(error); } }); }; __thaw_fs.writev = function(fd, buffers, position, callback) { if (typeof position === 'function') { callback = position; position = null; } if (typeof callback !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { callback(null, __thaw_fs.writevSync(fd, buffers, position), buffers); } catch (error) { callback(error); } }); };;\n\
             function watchSnapshot(path, recursive, prefix, output) { output = output || {}; prefix = prefix || ''; var stats = __thaw_fs.statSync(path); if (!stats.isDirectory()) { output[prefix || pathValue(path).split('/').pop()] = String(stats.size) + ':' + String(stats.mtimeMs); return output; } __thaw_fs.readdirSync(path, { withFileTypes: true }).forEach(function(entry) { var name = prefix ? prefix + '/' + entry.name : entry.name, full = pathValue(path).replace(/\\/$/, '') + '/' + entry.name, value = __thaw_fs.statSync(full); output[name] = String(value.size) + ':' + String(value.mtimeMs); if (recursive && value.isDirectory()) watchSnapshot(full, true, name, output); }); return output; } function FSWatcher(path, options, listener) { this.path = pathValue(path); this.closed = false; this._refed = options.persistent !== false; this._listeners = { change: [], error: [], close: [] }; if (typeof listener === 'function') this.on('change', listener); this._options = options; this._snapshot = watchSnapshot(this.path, Boolean(options.recursive)); var self = this; this._timer = setInterval(function() { if (self.closed) return; var current; try { current = watchSnapshot(self.path, Boolean(options.recursive)); } catch (error) { self.emit('error', error); self.close(); return; } var previous = self._snapshot, names = Object.keys(Object.assign({}, previous, current)); names.forEach(function(name) { if (!(name in previous) || !(name in current)) self.emit('change', 'rename', options.encoding === 'buffer' ? Buffer.from(name) : name); else if (previous[name] !== current[name]) self.emit('change', 'change', options.encoding === 'buffer' ? Buffer.from(name) : name); }); self._snapshot = current; }, Math.max(1, Number(options.interval || 50))); if (options.signal) { if (options.signal.aborted) this.close(); else options.signal.addEventListener('abort', function() { self.close(); }, { once: true }); } } FSWatcher.prototype.on = FSWatcher.prototype.addListener = function(event, listener) { (this._listeners[event] || (this._listeners[event] = [])).push(listener); return this; }; FSWatcher.prototype.once = function(event, listener) { var self = this, wrapped = function() { self.removeListener(event, wrapped); return listener.apply(self, arguments); }; return this.on(event, wrapped); }; FSWatcher.prototype.removeListener = function(event, listener) { if (this._listeners[event]) this._listeners[event] = this._listeners[event].filter(function(value) { return value !== listener; }); return this; }; FSWatcher.prototype.emit = function(event) { var args = Array.prototype.slice.call(arguments, 1), self = this; (this._listeners[event] || []).slice().forEach(function(listener) { listener.apply(self, args); }); return Boolean((this._listeners[event] || []).length); }; FSWatcher.prototype.close = function() { if (!this.closed) { this.closed = true; clearInterval(this._timer); this._timer = null; this.emit('close'); } return this; }; FSWatcher.prototype.ref = function() { this._refed = true; return this; }; FSWatcher.prototype.unref = function() { this._refed = false; return this; }; FSWatcher.prototype.hasRef = function() { return this._refed; }; __thaw_fs.FSWatcher = FSWatcher; __thaw_fs.watch = function(path, options, listener) { if (typeof options === 'function') { listener = options; options = {}; } else if (typeof options === 'string') options = { encoding: options }; return new FSWatcher(path, options || {}, listener); };\n\
             function globMatch(pattern, value) { var memo = {}; function match(pi, vi) { var key = pi + ':' + vi; if (key in memo) return memo[key]; if (pi === pattern.length) return memo[key] = vi === value.length; if (pattern[pi] === '*') { var double = pattern[pi + 1] === '*'; if (double) { var next = pi + 2; while (pattern[next] === '*') next++; if (pattern[next] === '/' && match(next + 1, vi)) return memo[key] = true; if (match(next, vi)) return memo[key] = true; return memo[key] = vi < value.length && match(pi, vi + 1); } if (match(pi + 1, vi)) return memo[key] = true; return memo[key] = vi < value.length && value[vi] !== '/' && match(pi, vi + 1); } if (pattern[pi] === '?') return memo[key] = vi < value.length && value[vi] !== '/' && match(pi + 1, vi + 1); return memo[key] = vi < value.length && pattern[pi] === value[vi] && match(pi + 1, vi + 1); } return match(0, 0); } function globEntries(root, relative, output) { __thaw_fs.readdirSync(root, { withFileTypes: true }).forEach(function(entry) { var name = relative ? relative + '/' + entry.name : entry.name, full = pathValue(root).replace(/[/]$/, '') + '/' + entry.name; output.push({ name: name, entry: entry }); if (entry.isDirectory()) globEntries(full, name, output); }); return output; } function globExcluded(name, exclude) { if (!exclude) return false; if (typeof exclude === 'function') return Boolean(exclude(name)); return (Array.isArray(exclude) ? exclude : [exclude]).some(function(pattern) { return globMatch(String(pattern), name); }); } __thaw_fs.globSync = function(patterns, options) { options = options || {}; patterns = Array.isArray(patterns) ? patterns : [patterns]; var root = pathValue(options.cwd || process.cwd()), entries = globEntries(root, '', []), seen = {}; return entries.filter(function(record) { if (!patterns.some(function(pattern) { return globMatch(String(pattern), record.name); }) || globExcluded(record.name, options.exclude)) return false; if (seen[record.name]) return false; seen[record.name] = true; return true; }).map(function(record) { return options.withFileTypes ? record.entry : record.name; }); }; __thaw_fs.glob = function(patterns, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } if (typeof callback !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { callback(null, __thaw_fs.globSync(patterns, options)); } catch (error) { callback(error); } }); }; __thaw_fs.promises.glob = function(patterns, options) { var values, index = 0; return { [Symbol.asyncIterator]: function() { return this; }, next: function() { return Promise.resolve().then(function() { if (!values) values = __thaw_fs.globSync(patterns, options); return index < values.length ? { value: values[index++], done: false } : { value: undefined, done: true }; }); } }; };\n\
             __thaw_fs.promises.watch = function(path, options) { options = options || {}; var queue = [], waiters = [], closed = false, failure, watchOptions = Object.assign({}, options); delete watchOptions.signal; var watcher = __thaw_fs.watch(path, watchOptions, function(eventType, filename) { var value = { eventType: eventType, filename: filename }; if (waiters.length) waiters.shift().resolve({ value: value, done: false }); else queue.push(value); }); function finish(error) { if (closed) return; closed = true; failure = error; watcher.close(); while (waiters.length) { var waiter = waiters.shift(); error ? waiter.reject(error) : waiter.resolve({ value: undefined, done: true }); } } watcher.on('error', function(error) { finish(error); }); var signal = options.signal; if (signal) { var abort = function() { var error = new DOMException('The operation was aborted', 'AbortError'); error.cause = signal.reason; finish(error); }; if (signal.aborted) abort(); else signal.addEventListener('abort', abort, { once: true }); } return { [Symbol.asyncIterator]: function() { return this; }, next: function() { if (queue.length) return Promise.resolve({ value: queue.shift(), done: false }); if (failure) return Promise.reject(failure); if (closed) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { waiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { finish(); return Promise.resolve({ value: undefined, done: true }); }, throw: function(error) { finish(error); return Promise.reject(error); } }; };\n\
             function cpError(path) { var error = new Error('EEXIST: destination already exists, cp ' + path); error.code = 'EEXIST'; error.errno = -1; error.path = path; error.syscall = 'cp'; return error; } function cpDirectoryError(path) { var error = new Error('ERR_FS_EISDIR: recursive option is required, cp ' + path); error.code = 'ERR_FS_EISDIR'; error.path = path; error.syscall = 'cp'; return error; } function cpParent(path) { var slash = path.lastIndexOf('/'); return slash > 0 ? path.slice(0, slash) : '.'; } function cpNormalize(path) { var absolute = path.charAt(0) === '/', values = []; path.split('/').forEach(function(part) { if (!part || part === '.') return; if (part === '..') values.pop(); else values.push(part); }); return (absolute ? '/' : '') + values.join('/'); } function cpEnsureParent(path) { var parent = cpParent(path); if (parent && parent !== '.' && !__thaw_fs.existsSync(parent)) __thaw_fs.mkdirSync(parent, { recursive: true }); } function cpPrepare(destination, directory, options) { if (!__thaw_fs.existsSync(destination)) return true; var current = __thaw_fs.lstatSync(destination); if (directory && current.isDirectory()) return true; if (options.force === false) { if (options.errorOnExist) throw cpError(destination); return false; } __thaw_fs.rmSync(destination, { recursive: true, force: true }); return true; } function cpPreserve(sourceStats, destination, options) { if (options.preserveTimestamps) __thaw_fs.utimesSync(destination, sourceStats.atime, sourceStats.mtime); } function cpLinkTarget(source, options) { var target = __thaw_fs.readlinkSync(source); if (options.verbatimSymlinks || target.charAt(0) === '/') return target; return cpNormalize(cpParent(source) + '/' + target); } function cpNodeSync(source, destination, options) { source = pathValue(source); destination = pathValue(destination); if (typeof options.filter === 'function') { var allowed = options.filter(source, destination); if (allowed && typeof allowed.then === 'function') throw new TypeError('fs.cpSync filter must return a boolean'); if (!allowed) return; } var stats = options.dereference ? __thaw_fs.statSync(source) : __thaw_fs.lstatSync(source); if (stats.isSymbolicLink() && !options.dereference) { if (!cpPrepare(destination, false, options)) return; cpEnsureParent(destination); __thaw_fs.symlinkSync(cpLinkTarget(source, options), destination); return; } if (stats.isDirectory()) { if (!options.recursive) throw cpDirectoryError(source); if (!cpPrepare(destination, true, options)) return; if (!__thaw_fs.existsSync(destination)) __thaw_fs.mkdirSync(destination, { recursive: true }); __thaw_fs.readdirSync(source).forEach(function(name) { cpNodeSync(source.replace(/[/]$/, '') + '/' + name, destination.replace(/[/]$/, '') + '/' + name, options); }); cpPreserve(stats, destination, options); return; } if (!cpPrepare(destination, false, options)) return; cpEnsureParent(destination); __thaw_fs.copyFileSync(source, destination); cpPreserve(stats, destination, options); } __thaw_fs.cpSync = function(source, destination, options) { cpNodeSync(source, destination, options || {}); }; function cpNodeAsync(source, destination, options) { source = pathValue(source); destination = pathValue(destination); var filtered = typeof options.filter === 'function' ? Promise.resolve().then(function() { return options.filter(source, destination); }) : Promise.resolve(true); return filtered.then(function(allowed) { if (!allowed) return; var stats = options.dereference ? __thaw_fs.statSync(source) : __thaw_fs.lstatSync(source); if (stats.isSymbolicLink() && !options.dereference) { if (!cpPrepare(destination, false, options)) return; cpEnsureParent(destination); __thaw_fs.symlinkSync(cpLinkTarget(source, options), destination); return; } if (stats.isDirectory()) { if (!options.recursive) throw cpDirectoryError(source); if (!cpPrepare(destination, true, options)) return; if (!__thaw_fs.existsSync(destination)) __thaw_fs.mkdirSync(destination, { recursive: true }); return __thaw_fs.readdirSync(source).reduce(function(chain, name) { return chain.then(function() { return cpNodeAsync(source.replace(/[/]$/, '') + '/' + name, destination.replace(/[/]$/, '') + '/' + name, options); }); }, Promise.resolve()).then(function() { cpPreserve(stats, destination, options); }); } if (!cpPrepare(destination, false, options)) return; cpEnsureParent(destination); __thaw_fs.copyFileSync(source, destination); cpPreserve(stats, destination, options); }); } __thaw_fs.promises.cp = function(source, destination, options) { return cpNodeAsync(source, destination, options || {}); }; __thaw_fs.cp = function(source, destination, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } if (typeof callback !== 'function') throw new TypeError('callback must be a function'); cpNodeAsync(source, destination, options || {}).then(function() { callback(null); }, callback); };\n\
             ReadStream = function ReadStream(path, options) { if (!(this instanceof ReadStream)) return new ReadStream(path, options); options = options || {}; stream.Readable.call(this, options); this.path = pathValue(path); this.pending = true; this.fd = null; this.bytesRead = 0; this._position = options.start === undefined ? 0 : Number(options.start); this._end = options.end === undefined ? Infinity : Number(options.end); this._chunkSize = Math.max(1, Number(options.highWaterMark || 65536)); this.close = this.destroy.bind(this); if (options.encoding) this.setEncoding(options.encoding); var self = this; queueMicrotask(function() { try { invoke('stat', self.path); self.fd = 1; self.pending = false; self.emit('open', self.fd); self.emit('ready'); function finish() { if (self.readableEnded || self.destroyed) return; self.push(null); if (options.autoClose !== false) queueMicrotask(function() { if (self.fd !== null) { self.fd = null; if (options.emitClose !== false) self.emit('close'); } }); } function pump() { if (self.destroyed || self.readableEnded) return; if (self._position > self._end) { finish(); return; } var length = Math.min(self._chunkSize, self._end === Infinity ? self._chunkSize : self._end - self._position + 1), record = invoke('read_range', self.path, String(self._position) + ',' + String(length)), chunk = Buffer.from(record.data, 'hex'); if (!chunk.length) { finish(); return; } self._position += chunk.length; self.bytesRead += chunk.length; self.push(chunk); if (chunk.length < length || self._position > self._end) finish(); else queueMicrotask(pump); } pump(); } catch (error) { self.pending = false; self.destroy(error); } }); if (options.signal) stream.addAbortSignal(options.signal, this); }; ReadStream.prototype = Object.create(stream.Readable.prototype); ReadStream.prototype.constructor = ReadStream; WriteStream = function WriteStream(path, options) { if (!(this instanceof WriteStream)) return new WriteStream(path, options); options = options || {}; initializeWriteQueue(this, options.highWaterMark); this.path = pathValue(path); this.pending = true; this.fd = null; this.bytesWritten = 0; this._position = options.start === undefined ? 0 : Number(options.start); this._flags = String(options.flags || 'w'); var self = this; if (this._flags.indexOf('x') >= 0 && __thaw_fs.existsSync(this.path)) throw exclusiveError(this.path, 'open'); if (this._flags.charAt(0) === 'w') invoke('write_mode', this.path, String(fsMode(options, 438)) + ','); else if (this._flags.charAt(0) === 'a') invoke('append_mode', this.path, String(fsMode(options, 438)) + ','); else invoke('stat', this.path); stream.Writable.call(this, { highWaterMark: options.highWaterMark, write: function(chunk, encoding, callback) { try { chunk = Buffer.from(chunk); if (self._flags.charAt(0) === 'a') invoke('append', self.path, chunk.toString('hex')); else { invoke('write_range', self.path, String(self._position) + ':' + chunk.toString('hex')); self._position += chunk.length; } self.bytesWritten += chunk.length; queueMicrotask(callback); } catch (error) { queueMicrotask(function() { callback(error); }); } }, final: function(callback) { callback(); if (options.autoClose !== false) queueMicrotask(function() { if (self.fd !== null) { self.fd = null; if (options.emitClose !== false) self.emit('close'); } }); } }); queueMicrotask(function() { if (self.destroyed) return; self.fd = 1; self.pending = false; self.emit('open', self.fd); self.emit('ready'); }); if (options.signal) stream.addAbortSignal(options.signal, this); }; WriteStream.prototype = Object.create(stream.Writable.prototype); WriteStream.prototype.constructor = WriteStream; WriteStream.prototype.close = function(callback) { if (!this.writableEnded) this.end(callback); else if (callback) queueMicrotask(callback); }; __thaw_fs.ReadStream = ReadStream; __thaw_fs.WriteStream = WriteStream; __thaw_fs.createReadStream = function(path, options) { return new ReadStream(path, options); }; __thaw_fs.createWriteStream = function(path, options) { return new WriteStream(path, options); };\n\
             function fsMode(options, fallback) { var mode = typeof options === 'number' ? options : options && typeof options === 'object' ? options.mode : undefined; if (mode === undefined) return fallback; return typeof mode === 'string' ? parseInt(mode, 8) : Number(mode); } function fsWriteMode(path, value, options, defaultFlag) { var flag = options && typeof options === 'object' && options.flag || defaultFlag, operation = String(flag).charAt(0) === 'a' ? 'append_mode' : 'write_mode'; if (String(flag).indexOf('x') >= 0 && __thaw_fs.existsSync(path)) throw exclusiveError(path, 'open'); invoke(operation, path, String(fsMode(options, 438)) + ',' + data(value, options).toString('hex')); } __thaw_fs.writeFileSync = function(path, value, options) { fsWriteMode(path, value, options, 'w'); }; __thaw_fs.appendFileSync = function(path, value, options) { fsWriteMode(path, value, options, 'a'); }; __thaw_fs.mkdirSync = function(path, options) { var recursive = Boolean(options && typeof options === 'object' && options.recursive); invoke('mkdir_mode', path, String(fsMode(options, 511)), recursive); }; function fsOpenHandle(path, flags, mode) { flags = flags === undefined ? 'r' : String(flags); var exists = __thaw_fs.existsSync(path); if (flags.indexOf('x') >= 0 && exists) throw exclusiveError(path, 'open'); if ((flags.charAt(0) === 'w' || flags.charAt(0) === 'a') && !exists) invoke(flags.charAt(0) === 'a' ? 'append_mode' : 'write_mode', path, String(fsMode(mode, 438)) + ','); var handle = new FileHandle(path, flags.replace('x', '')); handle.flags = flags; return handle; } __thaw_fs.openSync = function(path, flags, mode) { return fsOpenHandle(path, flags, mode).fd; }; __thaw_fs.promises.open = function(path, flags, mode) { return Promise.resolve().then(function() { return fsOpenHandle(path, flags, mode); }); }; __thaw_fs.promises.writeFile = promised(__thaw_fs.writeFileSync); __thaw_fs.promises.appendFile = promised(__thaw_fs.appendFileSync); __thaw_fs.promises.mkdir = promised(__thaw_fs.mkdirSync); __thaw_fs.writeFile = callbackMethod(__thaw_fs.writeFileSync, false); __thaw_fs.appendFile = callbackMethod(__thaw_fs.appendFileSync, false); __thaw_fs.mkdir = callbackMethod(__thaw_fs.mkdirSync, false); __thaw_fs.open = callbackMethod(__thaw_fs.openSync, true);\n\
             function initializeWriteQueue(streamValue, highWaterMark) { if (streamValue._writeQueue) return; streamValue.writableHighWaterMark = Math.max(1, Number(highWaterMark || 16384)); streamValue.writableLength = 0; streamValue._writeQueue = []; streamValue._writing = false; streamValue._ending = false; streamValue._finishing = false; streamValue._needsDrain = false; streamValue._endCallbacks = []; } WriteStream.prototype._pumpWrites = function() { var self = this; if (this._writing || !this._writeQueue.length) { if (!this._writing && !this._writeQueue.length && this._ending) this._finishWrites(); return; } this._writing = true; var entry = this._writeQueue[0]; this._write(entry.chunk, entry.encoding, function(error) { self._writing = false; self._writeQueue.shift(); self.writableLength -= entry.chunk.length; if (entry.callback) entry.callback(error); if (error) { self.destroy(error); return; } if (self._needsDrain && self.writableLength < self.writableHighWaterMark) { self._needsDrain = false; self.emit('drain'); } self._pumpWrites(); }); }; WriteStream.prototype.write = function(chunk, encoding, callback) { initializeWriteQueue(this); if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.writableEnded) { var error = new Error('write after end'); error.code = 'ERR_STREAM_WRITE_AFTER_END'; throw error; } var value = typeof chunk === 'string' ? Buffer.from(chunk, encoding) : Buffer.from(chunk); this._writeQueue.push({ chunk: value, encoding: encoding || 'buffer', callback: callback }); this.writableLength += value.length; var available = this.writableLength < this.writableHighWaterMark; if (!available) this._needsDrain = true; this._pumpWrites(); return available; }; WriteStream.prototype._finishWrites = function() { if (this._finishing || this.writableFinished) return; this._finishing = true; var self = this, finish = function(error) { if (error) { self.destroy(error); return; } self.writable = false; self.writableFinished = true; self.emit('finish'); self._endCallbacks.splice(0).forEach(function(callback) { callback(); }); }; if (this._final) this._final(finish); else finish(); }; WriteStream.prototype.end = function(chunk, encoding, callback) { initializeWriteQueue(this); if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk !== undefined) this.write(chunk, encoding); if (callback) this._endCallbacks.push(callback); this.writableEnded = true; this._ending = true; this._pumpWrites(); return this; }; var createWriteStreamWithoutQueue = __thaw_fs.createWriteStream; __thaw_fs.createWriteStream = function(path, options) { var output = createWriteStreamWithoutQueue(path, options); initializeWriteQueue(output, options && options.highWaterMark); return output; }; FileHandle.prototype.createWriteStream = function(options) { this._check(); return __thaw_fs.createWriteStream(this.path, Object.assign({ flags: this.flags }, options || {})); };\n\
             function emitFsStream(event) { if (event === 'close') { if (this._fsCloseEmitted) return false; this._fsCloseEmitted = true; } return stream.Stream.prototype.emit.apply(this, arguments); } ReadStream.prototype.emit = emitFsStream; WriteStream.prototype.emit = emitFsStream;\n\
             WriteStream.prototype._pumpWrites = stream.Writable.prototype._pumpWrites; WriteStream.prototype._finishWrites = stream.Writable.prototype._finishWrites; WriteStream.prototype.write = stream.Writable.prototype.write; WriteStream.prototype.end = stream.Writable.prototype.end; WriteStream.prototype.cork = stream.Writable.prototype.cork; WriteStream.prototype.uncork = stream.Writable.prototype.uncork;\n\
             module.exports = __thaw_fs;\n\
             module.exports.default = __thaw_fs;\n\
             module.exports.__esModule = true;\n",
        ),
        "fs/promises" => Some(
            "var promises = require('node:fs').promises; module.exports = promises; module.exports.default = promises; module.exports.__esModule = true;\n",
        ),
        "http" => Some(
            r#"var net = require('node:net'), EventEmitter = require('node:events');
             function IncomingMessage(socket) { EventEmitter.call(this); this.socket = this.connection = socket; this.statusCode = null; this.statusMessage = null; this.headers = {}; this.headersDistinct = {}; this.rawHeaders = []; this.trailers = {}; this.rawTrailers = []; this.complete = false; this.aborted = false; this.readable = true; this.method = null; this.url = ''; this.httpVersion = '1.1'; }
             IncomingMessage.prototype = Object.create(EventEmitter.prototype); IncomingMessage.prototype.constructor = IncomingMessage; IncomingMessage.prototype.setEncoding = function(encoding) { this._encoding = encoding; return this; }; IncomingMessage.prototype.destroy = function(error) { this.aborted = true; this.readable = false; if (this.socket) this.socket.destroy(error); return this; }; IncomingMessage.prototype.resume = function() { return this; };
             function normalizeOptions(input, options) { var result = {}; if (typeof input === 'string' || input instanceof URL) { var url = input instanceof URL ? input : new URL(String(input)); result.protocol = url.protocol; result.hostname = url.hostname; if (url.port) result.port = Number(url.port); result.path = url.pathname + url.search; result.auth = url.username ? decodeURIComponent(url.username) + ':' + decodeURIComponent(url.password) : undefined; } else if (input) Object.assign(result, input); if (options) Object.assign(result, options); var expectedProtocol = result._defaultProtocol || 'http:'; result.protocol = result.protocol || expectedProtocol; if (result.protocol !== expectedProtocol) throw new Error('Protocol "' + result.protocol + '" not supported. Expected "' + expectedProtocol + '"'); result.hostname = result.hostname || result.host || 'localhost'; result.port = result.port === undefined ? (expectedProtocol === 'https:' ? 443 : 80) : Number(result.port); result.path = result.path || '/'; result.method = String(result.method || 'GET').toUpperCase(); return result; }
             function ClientRequest(input, options, callback) { if (!(this instanceof ClientRequest)) return new ClientRequest(input, options, callback); EventEmitter.call(this); if (typeof options === 'function') { callback = options; options = undefined; } this._options = normalizeOptions(input, options); this.method = this._options.method; this.path = this._options.path; this.host = this._options.hostname; this.protocol = this._options.protocol; this.agent = this._options.agent === false ? undefined : (this._options.agent || this._options._globalAgent || globalAgent); this.socket = this.connection = null; this.aborted = false; this.destroyed = false; this.finished = false; this.writableEnded = false; this._headers = Object.create(null); this._headerNames = Object.create(null); this._chunks = []; if (this._options.headers) for (var name of Object.keys(this._options.headers)) this.setHeader(name, this._options.headers[name]); var defaultPort = this.protocol === 'https:' ? 443 : 80; if (!this.hasHeader('host')) this.setHeader('Host', this.host + (this._options.port === defaultPort ? '' : ':' + this._options.port)); if (this._options.auth && !this.hasHeader('authorization')) this.setHeader('Authorization', 'Basic ' + Buffer.from(this._options.auth).toString('base64')); if (typeof callback === 'function') this.once('response', callback); }
             function validateHeaderName(name, label) { var value = String(name); if (!value || !/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(value)) { var error = new TypeError((label || 'Header name') + ' must be a valid HTTP token ["' + value + '"]'); error.code = 'ERR_INVALID_HTTP_TOKEN'; throw error; } return value; } function validateHeaderValue(name, value) { if (value === undefined) { var missing = new TypeError('Invalid value "undefined" for header "' + name + '"'); missing.code = 'ERR_HTTP_INVALID_HEADER_VALUE'; throw missing; } var values = Array.isArray(value) ? value : [value]; for (var item of values) if (/[^\t\x20-\x7e\x80-\xff]/.test(String(item))) { var error = new TypeError('Invalid character in header content ["' + name + '"]'); error.code = 'ERR_INVALID_CHAR'; throw error; } return value; }
             ClientRequest.prototype = Object.create(EventEmitter.prototype); ClientRequest.prototype.constructor = ClientRequest; ClientRequest.prototype.setHeader = function(name, value) { name = validateHeaderName(name); validateHeaderValue(name, value); var key = name.toLowerCase(); this._headers[key] = value; this._headerNames[key] = name; return this; }; ClientRequest.prototype.getHeader = function(name) { return this._headers[String(name).toLowerCase()]; }; ClientRequest.prototype.getHeaders = function() { return Object.assign({}, this._headers); }; ClientRequest.prototype.getHeaderNames = function() { return Object.keys(this._headers); }; ClientRequest.prototype.hasHeader = function(name) { return Object.prototype.hasOwnProperty.call(this._headers, String(name).toLowerCase()); }; ClientRequest.prototype.removeHeader = function(name) { var key = String(name).toLowerCase(); delete this._headers[key]; delete this._headerNames[key]; }; ClientRequest.prototype.flushHeaders = function() { return this; }; ClientRequest.prototype.write = function(chunk, encoding, callback) { if (this.writableEnded) throw new Error('write after end'); var value = Buffer.isBuffer(chunk) ? chunk : ArrayBuffer.isView(chunk) ? Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength) : chunk instanceof ArrayBuffer ? Buffer.from(chunk) : Buffer.from(String(chunk), encoding); this._chunks.push(value); if (typeof callback === 'function') queueMicrotask(callback); return true; };
             function decodeChunked(body) { var offset = 0, output = []; while (offset < body.length) { var line = body.indexOf('\r\n', offset); if (line < 0) throw new Error('Parse Error: Invalid chunk size'); var size = parseInt(body.slice(offset, line).toString(), 16); if (!Number.isFinite(size)) throw new Error('Parse Error: Invalid chunk size'); offset = line + 2; if (size === 0) break; output.push(body.slice(offset, offset + size)); offset += size + 2; } return Buffer.concat(output); }
             function parseResponse(buffer, socket) { var marker = buffer.indexOf(Buffer.from('\r\n\r\n')), head = marker < 0 ? '' : buffer.slice(0, marker).toString(), body = marker < 0 ? buffer : buffer.slice(marker + 4), lines = head.split('\r\n'), status = (lines.shift() || '').match(/^HTTP\/(\d+\.\d+)\s+(\d+)(?:\s+(.*))?$/); if (!status) throw new Error('Parse Error: Invalid HTTP response'); var response = new IncomingMessage(socket); response.httpVersion = status[1]; response.statusCode = Number(status[2]); response.statusMessage = status[3] || ''; lines.forEach(function(line) { var colon = line.indexOf(':'); if (colon < 0) return; var name = line.slice(0, colon), key = name.toLowerCase(), value = line.slice(colon + 1).trim(); response.rawHeaders.push(name, value); (response.headersDistinct[key] || (response.headersDistinct[key] = [])).push(value); if (key === 'set-cookie') response.headers[key] = response.headersDistinct[key].slice(); else response.headers[key] = response.headersDistinct[key].join(', '); }); if (String(response.headers['transfer-encoding'] || '').toLowerCase().indexOf('chunked') >= 0) body = decodeChunked(body); response.complete = true; return { response: response, body: body }; }
             ClientRequest.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk !== undefined) this.write(chunk, encoding); if (this.writableEnded) return this; this.finished = this.writableEnded = true; var request = this, body = Buffer.concat(this._chunks); if (body.length && !this.hasHeader('content-length') && !this.hasHeader('transfer-encoding')) this.setHeader('Content-Length', String(body.length)); if (!this.hasHeader('connection')) this.setHeader('Connection', 'close'); var head = this.method + ' ' + this.path + ' HTTP/1.1\r\n' + Object.keys(this._headers).map(function(key) { return request._headerNames[key] + ': ' + request._headers[key]; }).join('\r\n') + '\r\n\r\n'; var received = [], transport = this._options._transport || net, connectOptions = Object.assign({}, this._options, { host: this._options.hostname, port: this._options.port }); var socket = this.socket = this.connection = transport.createConnection ? transport.createConnection(connectOptions) : transport.connect(connectOptions); this.emit('socket', socket); socket.on('error', function(error) { request.destroyed = true; request.emit('error', error); }); socket.on('connect', function() { socket.end(Buffer.concat([Buffer.from(head), body])); request.emit('finish'); if (typeof callback === 'function') callback(); }); socket.on('data', function(data) { received.push(Buffer.from(data)); }); socket.on('end', function() { try { var parsed = parseResponse(Buffer.concat(received), socket), response = parsed.response; request.emit('response', response); var value = response._encoding ? parsed.body.toString(response._encoding) : parsed.body; if (parsed.body.length) response.emit('data', value); response.readable = false; response.emit('end'); request.destroyed = true; request.emit('close'); } catch (error) { request.emit('error', error); } }); return this; }; ClientRequest.prototype.abort = function() { this.aborted = true; this.emit('abort'); return this.destroy(); }; ClientRequest.prototype.destroy = function(error) { this.destroyed = true; if (this.socket) this.socket.destroy(error); return this; }; ClientRequest.prototype.setTimeout = function(timeout, callback) { if (typeof callback === 'function') this.once('timeout', callback); return this; }; ClientRequest.prototype.setNoDelay = ClientRequest.prototype.setSocketKeepAlive = function() { return this; };
             ClientRequest.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk !== undefined) this.write(chunk, encoding); if (this.writableEnded) return this; this.finished = this.writableEnded = true; var request = this, body = Buffer.concat(this._chunks); if (body.length && !this.hasHeader('content-length') && !this.hasHeader('transfer-encoding')) this.setHeader('Content-Length', String(body.length)); if (!this.hasHeader('connection')) this.setHeader('Connection', 'close'); var head = this.method + ' ' + this.path + ' HTTP/1.1\r\n' + Object.keys(this._headers).map(function(key) { return request._headerNames[key] + ': ' + request._headers[key]; }).join('\r\n') + '\r\n\r\n'; var pending = Buffer.alloc(0), response = null, remaining = null, chunkSize = null, ended = false, transport = this._options._transport || net, connectOptions = Object.assign({}, this._options, { host: this._options.hostname, port: this._options.port }); function finishResponse() { if (!response || ended) return; ended = true; response.complete = true; response.readable = false; response.emit('end'); } function emitBody(value) { if (!value.length || ended) return; if (remaining !== null) { var count = Math.min(remaining, value.length); if (count) response.emit('data', response._encoding ? value.subarray(0, count).toString(response._encoding) : value.subarray(0, count)); remaining -= count; if (remaining === 0) finishResponse(); return; } response.emit('data', response._encoding ? value.toString(response._encoding) : value); } function consume() { if (!response) { var marker = pending.indexOf(Buffer.from('\r\n\r\n')); if (marker < 0) return; var parsed = parseResponse(pending.subarray(0, marker + 4), request.socket); response = parsed.response; pending = pending.subarray(marker + 4); var length = response.headers['content-length']; remaining = length === undefined ? null : Math.max(0, Number(length)); request.emit('response', response); if (request.method === 'HEAD' || [101, 204, 205, 304].indexOf(response.statusCode) >= 0 || remaining === 0) { finishResponse(); pending = Buffer.alloc(0); return; } } if (String(response.headers['transfer-encoding'] || '').toLowerCase().indexOf('chunked') >= 0) { while (!ended) { if (chunkSize === null) { var line = pending.indexOf(Buffer.from('\r\n')); if (line < 0) return; chunkSize = parseInt(pending.subarray(0, line).toString().split(';')[0], 16); if (!Number.isFinite(chunkSize)) throw new Error('Parse Error: Invalid chunk size'); pending = pending.subarray(line + 2); if (chunkSize === 0) { finishResponse(); return; } } if (pending.length < chunkSize + 2) return; emitBody(pending.subarray(0, chunkSize)); pending = pending.subarray(chunkSize + 2); chunkSize = null; } } else { var available = pending; pending = Buffer.alloc(0); emitBody(available); } } var socket = this.socket = this.connection = transport.createConnection ? transport.createConnection(connectOptions) : transport.connect(connectOptions); this.emit('socket', socket); socket.on('error', function(error) { request.destroyed = true; request.emit('error', error); }); socket.on('connect', function() { socket.end(Buffer.concat([Buffer.from(head), body])); request.emit('finish'); if (typeof callback === 'function') callback(); }); socket.on('data', function(data) { try { pending = Buffer.concat([pending, Buffer.from(data)]); consume(); } catch (error) { request.emit('error', error); socket.destroy(); } }); socket.on('end', function() { try { consume(); if (!response) throw new Error('Parse Error: Invalid HTTP response'); finishResponse(); request.destroyed = true; request.emit('close'); } catch (error) { request.emit('error', error); } }); return this; };
             ClientRequest.prototype.end = function(chunk, encoding, callback) {
               if (typeof chunk === 'function') { callback = chunk; chunk = undefined; }
               else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; }
               if (chunk !== undefined) this.write(chunk, encoding);
               if (this.writableEnded) return this;
               this.finished = this.writableEnded = true;
               var request = this, body = Buffer.concat(this._chunks);
               if (body.length && !this.hasHeader('content-length') && !this.hasHeader('transfer-encoding')) this.setHeader('Content-Length', String(body.length));
               if (!this.hasHeader('connection')) this.setHeader('Connection', 'close');
               var head = this.method + ' ' + this.path + ' HTTP/1.1\r\n' + Object.keys(this._headers).map(function(key) { return request._headerNames[key] + ': ' + request._headers[key]; }).join('\r\n') + '\r\n\r\n';
               var pending = Buffer.alloc(0), response = null, remaining = null, chunkSize = null, ended = false;
               function finishResponse() { if (!response || ended) return; ended = true; response.complete = true; response.readable = false; response.emit('end'); }
               function emitBody(value) {
                 if (!value.length || ended) return;
                 if (remaining !== null) {
                   var count = Math.min(remaining, value.length);
                   if (count) response.emit('data', response._encoding ? value.subarray(0, count).toString(response._encoding) : value.subarray(0, count));
                   remaining -= count; if (remaining === 0) finishResponse(); return;
                 }
                 response.emit('data', response._encoding ? value.toString(response._encoding) : value);
               }
               function parseTrailers(block) {
                 response.trailersDistinct = {};
                 if (!block) return;
                 block.split('\r\n').forEach(function(line) {
                   var colon = line.indexOf(':'); if (colon < 0) return;
                   var name = line.slice(0, colon), key = name.toLowerCase(), value = line.slice(colon + 1).trim();
                   response.rawTrailers.push(name, value);
                   (response.trailersDistinct[key] || (response.trailersDistinct[key] = [])).push(value);
                   if (key === 'set-cookie') response.trailers[key] = response.trailersDistinct[key].slice();
                   else response.trailers[key] = response.trailersDistinct[key].join(', ');
                 });
               }
               function consume() {
                 while (!response) {
                   var marker = pending.indexOf(Buffer.from('\r\n\r\n')); if (marker < 0) return;
                   var parsed = parseResponse(pending.subarray(0, marker + 4), request.socket), candidate = parsed.response;
                   pending = pending.subarray(marker + 4);
                   if (candidate.statusCode >= 100 && candidate.statusCode < 200 && candidate.statusCode !== 101) {
                     var information = { statusCode: candidate.statusCode, statusMessage: candidate.statusMessage, httpVersion: candidate.httpVersion, httpVersionMajor: Number(candidate.httpVersion.split('.')[0]), httpVersionMinor: Number(candidate.httpVersion.split('.')[1]), headers: candidate.headers, rawHeaders: candidate.rawHeaders };
                     request.emit('information', information); if (candidate.statusCode === 100) request.emit('continue');
                     continue;
                   }
                   response = candidate; response.trailersDistinct = {};
                   var length = response.headers['content-length']; remaining = length === undefined ? null : Math.max(0, Number(length));
                   request.emit('response', response);
                   if (request.method === 'HEAD' || [101, 204, 205, 304].indexOf(response.statusCode) >= 0 || remaining === 0) { finishResponse(); pending = Buffer.alloc(0); return; }
                 }
                 if (String(response.headers['transfer-encoding'] || '').toLowerCase().indexOf('chunked') >= 0) {
                   while (!ended) {
                     if (chunkSize === 0) {
                       if (pending.length >= 2 && pending[0] === 13 && pending[1] === 10) { pending = pending.subarray(2); parseTrailers(''); finishResponse(); return; }
                       var trailerEnd = pending.indexOf(Buffer.from('\r\n\r\n')); if (trailerEnd < 0) return;
                       parseTrailers(pending.subarray(0, trailerEnd).toString()); pending = pending.subarray(trailerEnd + 4); finishResponse(); return;
                     }
                     if (chunkSize === null) {
                       var line = pending.indexOf(Buffer.from('\r\n')); if (line < 0) return;
                       chunkSize = parseInt(pending.subarray(0, line).toString().split(';')[0], 16);
                       if (!Number.isFinite(chunkSize)) throw new Error('Parse Error: Invalid chunk size');
                       pending = pending.subarray(line + 2); if (chunkSize === 0) continue;
                     }
                     if (pending.length < chunkSize + 2) return;
                     emitBody(pending.subarray(0, chunkSize)); pending = pending.subarray(chunkSize + 2); chunkSize = null;
                   }
                 } else { var available = pending; pending = Buffer.alloc(0); emitBody(available); }
               }
               var transport = this._options._transport || net, connectOptions = Object.assign({}, this._options, { host: this._options.hostname, port: this._options.port });
               var socket = this.socket = this.connection = transport.createConnection ? transport.createConnection(connectOptions) : transport.connect(connectOptions);
               this.emit('socket', socket);
               socket.on('error', function(error) { request.destroyed = true; request.emit('error', error); });
               socket.on('connect', function() { socket.end(Buffer.concat([Buffer.from(head), body])); request.emit('finish'); if (typeof callback === 'function') callback(); });
               socket.on('data', function(data) { try { pending = Buffer.concat([pending, Buffer.from(data)]); consume(); } catch (error) { request.emit('error', error); socket.destroy(); } });
               socket.on('end', function() { try { consume(); if (!response) throw new Error('Parse Error: Invalid HTTP response'); finishResponse(); request.destroyed = true; request.emit('close'); } catch (error) { request.emit('error', error); } });
               return this;
             };
             var endWithoutAgent = ClientRequest.prototype.end; ClientRequest.prototype.end = function() { if (this.agent && typeof this.agent.createConnection === 'function') { var agent = this.agent; this._options._transport = { createConnection: function(options) { return agent.createConnection(options); } }; } return endWithoutAgent.apply(this, arguments); };
             function request(input, options, callback) { return new ClientRequest(input, options, callback); } function get(input, options, callback) { var result = request(input, options, callback); result.end(); return result; }
             function ServerResponse(request) { EventEmitter.call(this); this.req = request; this.socket = this.connection = request.socket; this.statusCode = 200; this.statusMessage = null; this.sendDate = true; this.headersSent = false; this.finished = false; this.writableEnded = false; this._headers = Object.create(null); this._headerNames = Object.create(null); this._chunks = []; }
             ServerResponse.prototype = Object.create(EventEmitter.prototype); ServerResponse.prototype.constructor = ServerResponse; ServerResponse.prototype.setHeader = ClientRequest.prototype.setHeader; ServerResponse.prototype.getHeader = ClientRequest.prototype.getHeader; ServerResponse.prototype.getHeaders = ClientRequest.prototype.getHeaders; ServerResponse.prototype.getHeaderNames = ClientRequest.prototype.getHeaderNames; ServerResponse.prototype.hasHeader = ClientRequest.prototype.hasHeader; ServerResponse.prototype.removeHeader = ClientRequest.prototype.removeHeader; ServerResponse.prototype.writeHead = function(statusCode, statusMessage, headers) { this.statusCode = Number(statusCode); if (typeof statusMessage === 'object') { headers = statusMessage; statusMessage = undefined; } if (statusMessage !== undefined) this.statusMessage = String(statusMessage); if (headers) for (var name of Object.keys(headers)) this.setHeader(name, headers[name]); this.headersSent = true; return this; }; ServerResponse.prototype.flushHeaders = function() { this.headersSent = true; return this; }; ServerResponse.prototype.write = function(chunk, encoding, callback) { if (this.writableEnded) throw new Error('write after end'); var value = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk), encoding); this._chunks.push(value); if (typeof callback === 'function') queueMicrotask(callback); return true; }; ServerResponse.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk !== undefined) this.write(chunk, encoding); if (this.writableEnded) return this; this.finished = this.writableEnded = true; var body = Buffer.concat(this._chunks); if (!this.hasHeader('content-length') && !this.hasHeader('transfer-encoding')) this.setHeader('Content-Length', String(body.length)); if (!this.hasHeader('connection')) this.setHeader('Connection', 'close'); var message = this.statusMessage === null ? (STATUS_CODES[this.statusCode] || '') : this.statusMessage, response = 'HTTP/1.1 ' + this.statusCode + ' ' + message + '\r\n' + Object.keys(this._headers).map(function(key) { var value = this._headers[key]; if (Array.isArray(value)) return value.map(function(item) { return this._headerNames[key] + ': ' + item; }, this).join('\r\n'); return this._headerNames[key] + ': ' + value; }, this).join('\r\n') + '\r\n\r\n'; this.headersSent = true; var target = this; this.socket.end(Buffer.concat([Buffer.from(response), body]), function() { if (typeof callback === 'function') callback(); target.emit('finish'); target.emit('close'); }); return this; }; ServerResponse.prototype.destroy = function(error) { this.socket.destroy(error); return this; };
             function parseRequest(buffer, socket) { var marker = buffer.indexOf(Buffer.from('\r\n\r\n')), head = marker < 0 ? buffer.toString() : buffer.slice(0, marker).toString(), body = marker < 0 ? Buffer.alloc(0) : buffer.slice(marker + 4), lines = head.split('\r\n'), start = (lines.shift() || '').match(/^(\S+)\s+(\S+)\s+HTTP\/(\d+\.\d+)$/); if (!start) throw new Error('Parse Error: Invalid HTTP request'); var message = new IncomingMessage(socket); message.method = start[1]; message.url = start[2]; message.httpVersion = start[3]; lines.forEach(function(line) { var colon = line.indexOf(':'); if (colon < 0) return; var name = line.slice(0, colon), key = name.toLowerCase(), value = line.slice(colon + 1).trim(); message.rawHeaders.push(name, value); (message.headersDistinct[key] || (message.headersDistinct[key] = [])).push(value); message.headers[key] = message.headersDistinct[key].join(', '); }); message.complete = true; return { request: message, body: body }; }
             function Server(options, listener) { if (!(this instanceof Server)) return new Server(options, listener); EventEmitter.call(this); if (typeof options === 'function') { listener = options; options = {}; } options = options || {}; this.requestTimeout = 300000; this.headersTimeout = 60000; this.keepAliveTimeout = 5000; this.maxHeadersCount = null; this.listening = false; var transport = options._transport || net, server = this, accept = function(socket) { var chunks = [], handled = false; socket.on('data', function(chunk) { if (!handled) chunks.push(Buffer.from(chunk)); }); socket.on('end', function() { if (handled) return; handled = true; try { var parsed = parseRequest(Buffer.concat(chunks), socket), request = parsed.request, response = new ServerResponse(request); server.emit('request', request, response); if (parsed.body.length) request.emit('data', parsed.body); request.readable = false; request.emit('end'); } catch (error) { server.emit('clientError', error, socket); } }); }; this._net = transport.createServer(options, accept); this._net.on('listening', function() { server.listening = true; server.emit('listening'); }); this._net.on('close', function() { server.listening = false; server.emit('close'); }); this._net.on('error', function(error) { server.emit('error', error); }); this._net.on('connection', function(socket) { server.emit('connection', socket); }); this._net.on('secureConnection', function(socket) { server.emit('secureConnection', socket); }); if (typeof listener === 'function') this.on('request', listener); }
             Server.prototype = Object.create(EventEmitter.prototype); Server.prototype.constructor = Server; Server.prototype.listen = function() { this._net.listen.apply(this._net, arguments); return this; }; Server.prototype.close = function(callback) { this._net.close(callback); return this; }; Server.prototype.address = function() { return this._net.address(); }; Server.prototype.getConnections = function(callback) { return this._net.getConnections(callback); }; Server.prototype.closeAllConnections = function() { return this._net.closeAllConnections(); }; Server.prototype.closeIdleConnections = function() { return this._net.closeIdleConnections(); }; Server.prototype.ref = function() { this._net.ref(); return this; }; Server.prototype.unref = function() { this._net.unref(); return this; }; function createServer(options, listener) { return new Server(options, listener); }
             var METHODS = ['ACL','BIND','CHECKOUT','CONNECT','COPY','DELETE','GET','HEAD','LINK','LOCK','M-SEARCH','MERGE','MKACTIVITY','MKCALENDAR','MKCOL','MOVE','NOTIFY','OPTIONS','PATCH','POST','PROPFIND','PROPPATCH','PURGE','PUT','REBIND','REPORT','SEARCH','SOURCE','SUBSCRIBE','TRACE','UNBIND','UNLINK','UNLOCK','UNSUBSCRIBE']; var STATUS_CODES = { 200: 'OK', 201: 'Created', 202: 'Accepted', 204: 'No Content', 301: 'Moved Permanently', 302: 'Found', 304: 'Not Modified', 400: 'Bad Request', 401: 'Unauthorized', 403: 'Forbidden', 404: 'Not Found', 500: 'Internal Server Error', 502: 'Bad Gateway', 503: 'Service Unavailable' };
             function Agent(options, transport, protocol) { if (!(this instanceof Agent)) return new Agent(options, transport, protocol); EventEmitter.call(this); options = options || {}; this.options = Object.assign({}, options); this.keepAlive = Boolean(options.keepAlive); this.keepAliveMsecs = options.keepAliveMsecs === undefined ? 1000 : Number(options.keepAliveMsecs); this.maxSockets = options.maxSockets === undefined ? Infinity : Number(options.maxSockets); this.maxFreeSockets = options.maxFreeSockets === undefined ? 256 : Number(options.maxFreeSockets); this.maxTotalSockets = options.maxTotalSockets === undefined ? Infinity : Number(options.maxTotalSockets); this.totalSocketCount = 0; this.requests = Object.create(null); this.sockets = Object.create(null); this.freeSockets = Object.create(null); this.protocol = protocol || 'http:'; this.defaultPort = this.protocol === 'https:' ? 443 : 80; this._transport = transport || net; }
             Agent.prototype = Object.create(EventEmitter.prototype); Agent.prototype.constructor = Agent; Agent.prototype.getName = function(options) { options = options || {}; var host = options.host || options.hostname || 'localhost', port = options.port || this.defaultPort, localAddress = options.localAddress || '', family = options.family || ''; return host + ':' + port + ':' + localAddress + ':' + family; }; Agent.prototype.createConnection = function(options, callback) { var agent = this, name = this.getName(options), socket = this._transport.createConnection ? this._transport.createConnection(options) : this._transport.connect(options); (this.sockets[name] || (this.sockets[name] = [])).push(socket); this.totalSocketCount++; socket.once('close', function() { agent.removeSocket(socket, options); }); if (typeof callback === 'function') socket.once(this.protocol === 'https:' ? 'secureConnect' : 'connect', function() { callback(null, socket); }); return socket; }; Agent.prototype.keepSocketAlive = function(socket) { socket.setKeepAlive(true, this.keepAliveMsecs); socket.unref(); return true; }; Agent.prototype.reuseSocket = function(socket) { socket.ref(); }; Agent.prototype.removeSocket = function(socket, options) { var name = this.getName(options), list = this.sockets[name] || [], index = list.indexOf(socket); if (index >= 0) list.splice(index, 1); if (!list.length) delete this.sockets[name]; this.totalSocketCount = Math.max(0, this.totalSocketCount - 1); }; Agent.prototype.destroy = function() { for (var group of [this.sockets, this.freeSockets]) for (var name of Object.keys(group)) for (var socket of group[name]) socket.destroy(); this.sockets = Object.create(null); this.freeSockets = Object.create(null); this.totalSocketCount = 0; };
             var globalAgent = new Agent({ keepAlive: true }, net, 'http:'); function setMaxIdleHTTPParsers() {}
             function createSecureModule(tls) { function SecureAgent(options) { Agent.call(this, options, tls, 'https:'); } SecureAgent.prototype = Object.create(Agent.prototype); SecureAgent.prototype.constructor = SecureAgent; var secureGlobalAgent = new SecureAgent({ keepAlive: true }); function secureOptions(options) { return Object.assign({}, options || {}, { _transport: tls, _defaultProtocol: 'https:', _globalAgent: secureGlobalAgent }); } function secureRequest(input, options, callback) { if (typeof options === 'function') { callback = options; options = undefined; } return new ClientRequest(input, secureOptions(options), callback); } function secureGet(input, options, callback) { var result = secureRequest(input, options, callback); result.end(); return result; } function secureCreateServer(options, listener) { if (typeof options === 'function') { listener = options; options = {}; } return new Server(Object.assign({}, options || {}, { _transport: tls }), listener); } return { request: secureRequest, get: secureGet, createServer: secureCreateServer, ClientRequest: ClientRequest, IncomingMessage: IncomingMessage, ServerResponse: ServerResponse, Server: Server, METHODS: METHODS, STATUS_CODES: STATUS_CODES, maxHeaderSize: 16384, globalAgent: secureGlobalAgent, Agent: SecureAgent, validateHeaderName: validateHeaderName, validateHeaderValue: validateHeaderValue, setMaxIdleHTTPParsers: setMaxIdleHTTPParsers }; }
             module.exports = { request: request, get: get, createServer: createServer, ClientRequest: ClientRequest, IncomingMessage: IncomingMessage, ServerResponse: ServerResponse, Server: Server, Agent: Agent, METHODS: METHODS, STATUS_CODES: STATUS_CODES, maxHeaderSize: 16384, globalAgent: globalAgent, validateHeaderName: validateHeaderName, validateHeaderValue: validateHeaderValue, setMaxIdleHTTPParsers: setMaxIdleHTTPParsers, __createSecureModule: createSecureModule }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "http2" => Some(
            r#"var EventEmitter = require('node:events'), PREFACE = Buffer.from('PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n'), sensitiveHeaders = Symbol('nodejs.http2.sensitiveHeaders');
             var constants = { NGHTTP2_NO_ERROR: 0, NGHTTP2_PROTOCOL_ERROR: 1, NGHTTP2_INTERNAL_ERROR: 2, NGHTTP2_FLOW_CONTROL_ERROR: 3, NGHTTP2_SETTINGS_TIMEOUT: 4, NGHTTP2_STREAM_CLOSED: 5, NGHTTP2_FRAME_SIZE_ERROR: 6, NGHTTP2_REFUSED_STREAM: 7, NGHTTP2_CANCEL: 8, NGHTTP2_COMPRESSION_ERROR: 9, NGHTTP2_CONNECT_ERROR: 10, NGHTTP2_ENHANCE_YOUR_CALM: 11, NGHTTP2_INADEQUATE_SECURITY: 12, NGHTTP2_HTTP_1_1_REQUIRED: 13, NGHTTP2_FLAG_NONE: 0, NGHTTP2_FLAG_END_STREAM: 1, NGHTTP2_FLAG_ACK: 1, NGHTTP2_FLAG_END_HEADERS: 4, NGHTTP2_SETTINGS_HEADER_TABLE_SIZE: 1, NGHTTP2_SETTINGS_ENABLE_PUSH: 2, NGHTTP2_SETTINGS_MAX_CONCURRENT_STREAMS: 3, NGHTTP2_SETTINGS_INITIAL_WINDOW_SIZE: 4, NGHTTP2_SETTINGS_MAX_FRAME_SIZE: 5, NGHTTP2_SETTINGS_MAX_HEADER_LIST_SIZE: 6, HTTP2_HEADER_STATUS: ':status', HTTP2_HEADER_METHOD: ':method', HTTP2_HEADER_AUTHORITY: ':authority', HTTP2_HEADER_SCHEME: ':scheme', HTTP2_HEADER_PATH: ':path' };
             var staticTable = [null, [':authority',''], [':method','GET'], [':method','POST'], [':path','/'], [':path','/index.html'], [':scheme','http'], [':scheme','https'], [':status','200'], [':status','204'], [':status','206'], [':status','304'], [':status','400'], [':status','404'], [':status','500'], ['accept-charset',''], ['accept-encoding','gzip, deflate'], ['accept-language',''], ['accept-ranges',''], ['accept',''], ['access-control-allow-origin',''], ['age',''], ['allow',''], ['authorization',''], ['cache-control',''], ['content-disposition',''], ['content-encoding',''], ['content-language',''], ['content-length',''], ['content-location',''], ['content-range',''], ['content-type',''], ['cookie',''], ['date',''], ['etag',''], ['expect',''], ['expires',''], ['from',''], ['host',''], ['if-match',''], ['if-modified-since',''], ['if-none-match',''], ['if-range',''], ['if-unmodified-since',''], ['last-modified',''], ['link',''], ['location',''], ['max-forwards',''], ['proxy-authenticate',''], ['proxy-authorization',''], ['range',''], ['referer',''], ['refresh',''], ['retry-after',''], ['server',''], ['set-cookie',''], ['strict-transport-security',''], ['transfer-encoding',''], ['user-agent',''], ['vary',''], ['via',''], ['www-authenticate','']];
             function encodeInteger(value, prefix, first) { value = Number(value); var limit = (1 << prefix) - 1, out = []; if (value < limit) return Buffer.from([first | value]); out.push(first | limit); value -= limit; while (value >= 128) { out.push((value & 127) | 128); value = Math.floor(value / 128); } out.push(value); return Buffer.from(out); }
             function decodeInteger(buffer, offset, prefix) { var limit = (1 << prefix) - 1, value = buffer[offset] & limit, consumed = 1, shift = 0; if (value < limit) return [value, consumed]; for (;;) { if (offset + consumed >= buffer.length) throw new Error('truncated HPACK integer'); var octet = buffer[offset + consumed++]; value += (octet & 127) << shift; if (!(octet & 128)) return [value, consumed]; shift += 7; if (shift > 28) throw new Error('HPACK integer overflow'); } }
             function encodeString(value) { var bytes = Buffer.from(String(value)), encoded = Buffer.from(__thaw_hpack_huffman_encode(bytes.toString('hex')), 'hex'); return encoded.length < bytes.length ? Buffer.concat([encodeInteger(encoded.length, 7, 128), encoded]) : Buffer.concat([encodeInteger(bytes.length, 7, 0), bytes]); }
             function decodeString(buffer, offset) { var length = decodeInteger(buffer, offset, 7), start = offset + length[1], end = start + length[0]; if (end > buffer.length) throw new Error('truncated HPACK string'); var bytes = buffer.subarray(start, end); if (buffer[offset] & 128) bytes = Buffer.from(__thaw_hpack_huffman_decode(bytes.toString('hex')), 'hex'); return [bytes.toString(), length[1] + length[0]]; }
             function staticIndex(name, value) { for (var index = 1; index < staticTable.length; index++) if (staticTable[index][0] === name && staticTable[index][1] === value) return index; return 0; }
             function staticNameIndex(name) { for (var index = 1; index < staticTable.length; index++) if (staticTable[index][0] === name) return index; return 0; }
             function tableEntry(context, index, side) { if (index < staticTable.length) return staticTable[index]; var table = context[side], entry = table[index - staticTable.length]; if (!entry) throw new Error('invalid HPACK table index'); return entry; }
             function dynamicIndex(context, name, value, side) { var table = context[side]; for (var index = 0; index < table.length; index++) if (table[index][0] === name && (value === undefined || table[index][1] === value)) return staticTable.length + index; return 0; }
             function resizeDynamic(context, side, maximum) { var table = context[side], sizeKey = side + 'Size'; context[sizeKey] = context[sizeKey] || 0; while (context[sizeKey] > maximum && table.length) { var removed = table.pop(); context[sizeKey] -= Buffer.byteLength(removed[0]) + Buffer.byteLength(removed[1]) + 32; } }
             function addDynamic(context, side, name, value, maximum) { var sizeKey = side + 'Size', size = Buffer.byteLength(name) + Buffer.byteLength(value) + 32; context[sizeKey] = context[sizeKey] || 0; if (size > maximum) { context[side] = []; context[sizeKey] = 0; return; } context[side].unshift([name, value]); context[sizeKey] += size; resizeDynamic(context, side, maximum); }
             function encodeHeaders(headers, context) { var chunks = [], sensitive = new Set((headers[sensitiveHeaders] || []).map(function(name) { return String(name).toLowerCase(); })), names = Object.keys(headers).filter(function(name) { return headers[name] !== undefined; }).sort(function(a, b) { return (a.charAt(0) === ':' ? 0 : 1) - (b.charAt(0) === ':' ? 0 : 1); }); names.forEach(function(original) { var name = original.toLowerCase(), values = Array.isArray(headers[original]) ? headers[original] : [headers[original]]; values.forEach(function(item) { var value = String(item), exact = staticIndex(name, value) || dynamicIndex(context, name, value, '_encoderTable'); if (exact) { chunks.push(encodeInteger(exact, 7, 128)); return; } var named = staticNameIndex(name) || dynamicIndex(context, name, undefined, '_encoderTable'), indexed = !sensitive.has(name) && name.charAt(0) !== ':', prefix = indexed ? 6 : 4, first = indexed ? 64 : (sensitive.has(name) ? 16 : 0); chunks.push(encodeInteger(named, prefix, first), named ? Buffer.alloc(0) : encodeString(name), encodeString(value)); if (indexed) addDynamic(context, '_encoderTable', name, value, Number(context.remoteSettings.headerTableSize || 4096)); }); }); return Buffer.concat(chunks); }
             function decodeHeaders(buffer, context) { var result = {}, offset = 0, allowSizeUpdate = true; while (offset < buffer.length) { var first = buffer[offset], name, value, decoded, incremental = false; if (first & 128) { decoded = decodeInteger(buffer, offset, 7); offset += decoded[1]; var indexed = tableEntry(context, decoded[0], '_decoderTable'); name = indexed[0]; value = indexed[1]; allowSizeUpdate = false; } else if ((first & 224) === 32) { if (!allowSizeUpdate) throw new Error('HPACK table size update must precede headers'); decoded = decodeInteger(buffer, offset, 5); offset += decoded[1]; if (decoded[0] > Number(context.localSettings.headerTableSize || 4096)) throw new Error('HPACK table size exceeds SETTINGS limit'); context._decoderMaxSize = decoded[0]; resizeDynamic(context, '_decoderTable', decoded[0]); continue; } else { incremental = Boolean(first & 64); var prefix = incremental ? 6 : 4; decoded = decodeInteger(buffer, offset, prefix); offset += decoded[1]; if (decoded[0]) name = tableEntry(context, decoded[0], '_decoderTable')[0]; else { decoded = decodeString(buffer, offset); name = decoded[0].toLowerCase(); offset += decoded[1]; } decoded = decodeString(buffer, offset); value = decoded[0]; offset += decoded[1]; allowSizeUpdate = false; if (incremental) addDynamic(context, '_decoderTable', name, value, context._decoderMaxSize); } if (name === 'set-cookie') { if (!result[name]) result[name] = []; result[name].push(value); } else if (result[name] === undefined) result[name] = value; else result[name] += ', ' + value; } return result; }
             function frame(type, flags, stream, payload) { payload = payload || Buffer.alloc(0); var header = Buffer.alloc(9), length = payload.length; header[0] = length >>> 16; header[1] = length >>> 8; header[2] = length; header[3] = type; header[4] = flags; header[5] = (stream >>> 24) & 127; header[6] = stream >>> 16; header[7] = stream >>> 8; header[8] = stream; return Buffer.concat([header, payload]); }
             function settingsPayload(settings) { var ids = { headerTableSize: 1, enablePush: 2, maxConcurrentStreams: 3, initialWindowSize: 4, maxFrameSize: 5, maxHeaderListSize: 6 }, chunks = []; Object.keys(ids).forEach(function(name) { if (settings[name] === undefined) return; var value = Number(settings[name]), id = ids[name], item = Buffer.alloc(6); item[0] = id >>> 8; item[1] = id; item[2] = value >>> 24; item[3] = value >>> 16; item[4] = value >>> 8; item[5] = value; chunks.push(item); }); return Buffer.concat(chunks); }
             function unpackSettings(payload) { var names = { 1: 'headerTableSize', 2: 'enablePush', 3: 'maxConcurrentStreams', 4: 'initialWindowSize', 5: 'maxFrameSize', 6: 'maxHeaderListSize' }, result = {}; for (var offset = 0; offset + 6 <= payload.length; offset += 6) { var id = payload[offset] * 256 + payload[offset + 1], value = (payload[offset + 2] * 0x1000000) + (payload[offset + 3] << 16) + (payload[offset + 4] << 8) + payload[offset + 5]; if (names[id]) result[names[id]] = value; } return result; }
             function getDefaultSettings() { return { headerTableSize: 4096, enablePush: true, initialWindowSize: 65535, maxFrameSize: 16384, maxHeaderListSize: 65535 }; }
             function getPackedSettings(settings) { return settingsPayload(Object.assign(getDefaultSettings(), settings || {})); }
             function getUnpackedSettings(buffer) { return unpackSettings(Buffer.from(buffer)); }
             function Http2Stream(session, id) { EventEmitter.call(this); this.session = session; this.id = id; this.closed = false; this.destroyed = false; this.pending = false; this.rstCode = 0; this.sentHeaders = undefined; this._sendWindow = Number(session.remoteSettings.initialWindowSize || 65535); this._receiveWindow = Number(session.localSettings.initialWindowSize || 65535); this._writeQueue = []; }
             Http2Stream.prototype = Object.create(EventEmitter.prototype); Http2Stream.prototype.constructor = Http2Stream;
             Http2Stream.prototype.setEncoding = function(encoding) { this._encoding = encoding; return this; }; Http2Stream.prototype._maybeClose = function() { if (!this.closed && this.readableEnded && this.writableEnded) { this.closed = true; this.emit('close'); } }; Http2Stream.prototype.write = function(chunk, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.closed || this._ending || this.writableEnded) return false; return this.session._queueData(this, Buffer.from(chunk, typeof encoding === 'string' ? encoding : undefined), false, callback); }; Http2Stream.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.closed || this._ending || this.writableEnded) return this; this._ending = true; this.session._queueData(this, chunk === undefined ? Buffer.alloc(0) : Buffer.from(chunk, typeof encoding === 'string' ? encoding : undefined), true, callback); return this; }; Http2Stream.prototype.close = function(code, callback) { if (this.closed) return; code = code === undefined ? 0 : Number(code); var payload = Buffer.alloc(4); payload[0] = code >>> 24; payload[1] = code >>> 16; payload[2] = code >>> 8; payload[3] = code; this.session._send(frame(3, 0, this.id, payload)); this.rstCode = code; this.closed = true; this.emit('close'); if (callback) queueMicrotask(callback); }; Http2Stream.prototype.destroy = function(error) { this.destroyed = true; if (error) this.emit('error', error); this.close(constants.NGHTTP2_CANCEL); return this; }; Http2Stream.prototype.pause = function() { this._paused = true; return this; }; Http2Stream.prototype.resume = function() { this._paused = false; while (this._queuedData && this._queuedData.length) this.emit('data', this._queuedData.shift()); return this; };
             function ClientHttp2Stream(session, id) { Http2Stream.call(this, session, id); } ClientHttp2Stream.prototype = Object.create(Http2Stream.prototype); ClientHttp2Stream.prototype.constructor = ClientHttp2Stream;
             function ServerHttp2Stream(session, id) { Http2Stream.call(this, session, id); this.headersSent = false; } ServerHttp2Stream.prototype = Object.create(Http2Stream.prototype); ServerHttp2Stream.prototype.constructor = ServerHttp2Stream; ServerHttp2Stream.prototype.respond = function(headers, options) { if (this.headersSent) throw new Error('Response has already been initiated'); headers = Object.assign({ ':status': 200 }, headers || {}); this.sentHeaders = headers; this.headersSent = true; this.session._sendHeaders(this.id, headers, Boolean(options && options.endStream)); if (options && options.endStream) { this.writableEnded = true; this.emit('finish'); this._maybeClose(); } }; ServerHttp2Stream.prototype.additionalHeaders = function(headers) { this.session._sendHeaders(this.id, headers, false); };
             function Http2Session(handle, server, transport) { EventEmitter.call(this); this._handle = handle; this._server = server; this._transport = transport || 'tcp'; this._buffer = Buffer.alloc(0); this._streams = new Map(); this._headerBlocks = new Map(); this._encoderTable = []; this._encoderTableSize = 0; this._decoderTable = []; this._decoderTableSize = 0; this._decoderMaxSize = 4096; this._nextStream = server ? 2 : 1; this._sendWindow = 65535; this._receiveWindow = 65535; this.closed = false; this.destroyed = false; this.connecting = false; this.localSettings = getDefaultSettings(); this.remoteSettings = getDefaultSettings(); this.type = server ? 0 : 1; this.encrypted = this._transport === 'tls'; this.alpnProtocol = this.encrypted ? 'h2' : 'h2c'; }
             Http2Session.prototype = Object.create(EventEmitter.prototype); Http2Session.prototype.constructor = Http2Session; Http2Session.prototype._send = function(value) { if (!this._handle || this.destroyed) return false; var outcome = this._transport === 'tls' ? __thaw_tls_write(this._handle, Buffer.from(value).toString('hex')) : __thaw_net_write(this._handle, Buffer.from(value).toString('hex')); if (outcome !== 'ok') { var error = new Error(outcome.substring(4)); error.code = 'ERR_HTTP2_ERROR'; this.emit('error', error); return false; } return true; }; Http2Session.prototype._sendHeaders = function(id, headers, endStream) { var block = encodeHeaders(headers, this), maximum = Math.max(16384, Number(this.remoteSettings.maxFrameSize || 16384)), offset = 0, first = true; do { var end = Math.min(block.length, offset + maximum), final = end === block.length, flags = (final ? 4 : 0) | (first && endStream ? 1 : 0); this._send(frame(first ? 1 : 9, flags, id, block.subarray(offset, end))); offset = end; first = false; } while (offset < block.length); }; Http2Session.prototype._queueData = function(stream, data, end, callback) { stream._writeQueue.push({ data: data, offset: 0, end: end, callback: callback }); this._flushStream(stream); return stream._writeQueue.length === 0; }; Http2Session.prototype._flushStream = function(stream) { var maximum = Math.max(16384, Number(this.remoteSettings.maxFrameSize || 16384)); while (stream._writeQueue.length && !stream.destroyed && !this.destroyed) { var item = stream._writeQueue[0], remaining = item.data.length - item.offset; if (remaining === 0) { this._send(frame(0, item.end ? 1 : 0, stream.id)); stream._writeQueue.shift(); if (item.end) { stream.writableEnded = true; stream._ending = false; stream.emit('finish'); stream._maybeClose(); } if (item.callback) queueMicrotask(item.callback); continue; } var available = Math.min(remaining, maximum, this._sendWindow, stream._sendWindow); if (available <= 0) break; var final = available === remaining, flags = final && item.end ? 1 : 0; this._send(frame(0, flags, stream.id, item.data.subarray(item.offset, item.offset + available))); item.offset += available; this._sendWindow -= available; stream._sendWindow -= available; if (final) { stream._writeQueue.shift(); if (item.end) { stream.writableEnded = true; stream._ending = false; stream.emit('finish'); stream._maybeClose(); } if (item.callback) queueMicrotask(item.callback); } } }; Http2Session.prototype._sendWindowUpdate = function(id, amount) { if (amount <= 0) return; var payload = Buffer.alloc(4); payload[0] = (amount >>> 24) & 127; payload[1] = amount >>> 16; payload[2] = amount >>> 8; payload[3] = amount; this._send(frame(8, 0, id, payload)); }; Http2Session.prototype._start = function() { var self = this; if (!this._server) this._send(Buffer.concat([PREFACE, frame(4, 0, 0, settingsPayload(this.localSettings))])); function pump() { if (!self._handle || self.destroyed) return; var outcome = self._transport === 'tls' ? __thaw_tls_poll_read(self._handle) : __thaw_net_poll_read(self._handle); if (outcome === 'pending') { setTimeout(pump, 0); return; } if (outcome.indexOf('ok:') === 0) { var chunk = Buffer.from(outcome.substring(3), 'hex'); if (chunk.length) { self._buffer = Buffer.concat([self._buffer, chunk]); try { self._parse(); } catch (error) { self.emit('error', error); self.destroy(error); return; } } setTimeout(pump, 0); return; } if (outcome === 'eof') self.destroy(); else { var error = new Error(outcome.substring(4)); error.code = 'ERR_HTTP2_ERROR'; self.destroy(error); } } queueMicrotask(pump); };
             Http2Session.prototype._parse = function() { if (this._server && !this._preface) { if (this._buffer.length < PREFACE.length) return; if (!this._buffer.subarray(0, PREFACE.length).equals(PREFACE)) throw new Error('Invalid HTTP/2 connection preface'); this._buffer = this._buffer.subarray(PREFACE.length); this._preface = true; this._send(frame(4, 0, 0, settingsPayload(this.localSettings))); this.emit('connect', this, null); } while (this._buffer.length >= 9) { var length = (this._buffer[0] << 16) | (this._buffer[1] << 8) | this._buffer[2]; if (this._buffer.length < 9 + length) return; var type = this._buffer[3], flags = this._buffer[4], id = ((this._buffer[5] & 127) * 0x1000000) + (this._buffer[6] << 16) + (this._buffer[7] << 8) + this._buffer[8], payload = this._buffer.subarray(9, 9 + length); this._buffer = this._buffer.subarray(9 + length); this._frame(type, flags, id, payload); } };
             Http2Session.prototype._deliverHeaders = function(id, flags, payload) { var stream = this._streams.get(id), headers = decodeHeaders(payload, this); if (!stream) { stream = new ServerHttp2Stream(this, id); stream.sentHeaders = headers; this._streams.set(id, stream); this.emit('stream', stream, headers, 0, []); } else { stream.emit(stream._responseReceived ? 'trailers' : 'response', headers, flags); stream._responseReceived = true; } if (flags & 1) { stream.readableEnded = true; stream.emit('end'); stream._maybeClose(); } }; Http2Session.prototype._frame = function(type, flags, id, payload) { if (type === 4) { if (!(flags & 1)) { var previousWindow = Number(this.remoteSettings.initialWindowSize || 65535), update = unpackSettings(payload); this.remoteSettings = Object.assign({}, this.remoteSettings, update); if (update.initialWindowSize !== undefined) { var delta = Number(update.initialWindowSize) - previousWindow, session = this; this._streams.forEach(function(stream) { stream._sendWindow += delta; session._flushStream(stream); }); } if (update.headerTableSize !== undefined) resizeDynamic(this, '_encoderTable', Number(update.headerTableSize)); this.emit('remoteSettings', this.remoteSettings); this._send(frame(4, 1, 0)); } else this.emit('localSettings', this.localSettings); return; } if (type === 6) { if (!(flags & 1)) this._send(frame(6, 1, 0, payload)); else this.emit('_pingAck', payload); return; } if (type === 7) { this.closed = true; this.emit('goaway', payload.length >= 8 ? ((payload[4] * 0x1000000) + (payload[5] << 16) + (payload[6] << 8) + payload[7]) : 0, payload.length >= 4 ? ((payload[0] & 127) * 0x1000000 + (payload[1] << 16) + (payload[2] << 8) + payload[3]) : 0, payload.subarray(8)); return; } if (type === 8) { if (payload.length !== 4) throw new Error('Invalid WINDOW_UPDATE frame'); var increment = ((payload[0] & 127) * 0x1000000) + (payload[1] << 16) + (payload[2] << 8) + payload[3]; if (!increment) throw new Error('Invalid zero WINDOW_UPDATE increment'); if (id === 0) { this._sendWindow += increment; var owner = this; this._streams.forEach(function(value) { owner._flushStream(value); }); } else { var writable = this._streams.get(id); if (writable) { writable._sendWindow += increment; this._flushStream(writable); } } return; } if (type === 1) { var pad = flags & 8 ? payload[0] : 0, start = (flags & 8 ? 1 : 0) + (flags & 32 ? 5 : 0), block = payload.subarray(start, payload.length - pad); if (flags & 4) this._deliverHeaders(id, flags, block); else this._headerBlocks.set(id, { chunks: [Buffer.from(block)], flags: flags }); return; } if (type === 9) { var pending = this._headerBlocks.get(id); if (!pending) throw new Error('Unexpected CONTINUATION frame'); pending.chunks.push(Buffer.from(payload)); if (flags & 4) { this._headerBlocks.delete(id); this._deliverHeaders(id, pending.flags | 4, Buffer.concat(pending.chunks)); } return; } var stream = this._streams.get(id); if (!stream) return; if (type === 0) { if (payload.length > this._receiveWindow || payload.length > stream._receiveWindow) throw new Error('HTTP/2 flow-control window exceeded'); this._receiveWindow -= payload.length; stream._receiveWindow -= payload.length; var value = stream._encoding ? payload.toString(stream._encoding) : Buffer.from(payload); if (stream._paused) (stream._queuedData || (stream._queuedData = [])).push(value); else stream.emit('data', value); if (payload.length) { this._receiveWindow += payload.length; stream._receiveWindow += payload.length; this._sendWindowUpdate(0, payload.length); this._sendWindowUpdate(id, payload.length); } if (flags & 1) { stream.readableEnded = true; stream.emit('end'); stream._maybeClose(); } } else if (type === 3) { stream.rstCode = payload.length >= 4 ? (payload[0] * 0x1000000 + (payload[1] << 16) + (payload[2] << 8) + payload[3]) : 0; stream.closed = true; stream.emit('aborted'); stream.emit('close'); } };
             Http2Session.prototype.request = function(headers, options) { if (this._server) throw new Error('request is only available on client sessions'); headers = Object.assign({ ':method': 'GET', ':path': '/', ':scheme': this.encrypted ? 'https' : 'http', ':authority': this.authority }, headers || {}); var id = this._nextStream; this._nextStream += 2; var stream = new ClientHttp2Stream(this, id); stream.sentHeaders = headers; this._streams.set(id, stream); this._sendHeaders(id, headers, Boolean(options && options.endStream)); if (options && options.endStream) { stream.writableEnded = true; stream.emit('finish'); } return stream; }; Http2Session.prototype.settings = function(settings, callback) { this.localSettings = Object.assign({}, this.localSettings, settings || {}); this._send(frame(4, 0, 0, settingsPayload(settings || {}))); if (callback) this.once('localSettings', function(value) { callback(null, value); }); }; Http2Session.prototype.ping = function(payload, callback) { if (typeof payload === 'function') { callback = payload; payload = Buffer.alloc(8); } payload = Buffer.from(payload || Buffer.alloc(8)); if (payload.length !== 8) throw new RangeError('HTTP/2 ping payload must be 8 bytes'); var start = Date.now(), self = this, listener = function(value) { if (value.equals(payload)) { self.off('_pingAck', listener); if (callback) callback(null, Date.now() - start, value); } }; this.on('_pingAck', listener); this._send(frame(6, 0, 0, payload)); return true; }; Http2Session.prototype.goaway = function(code, lastStreamID, opaqueData) { code = code || 0; lastStreamID = lastStreamID || 0; var payload = Buffer.alloc(8), opaque = opaqueData ? Buffer.from(opaqueData) : Buffer.alloc(0); payload[0] = (lastStreamID >>> 24) & 127; payload[1] = lastStreamID >>> 16; payload[2] = lastStreamID >>> 8; payload[3] = lastStreamID; payload[4] = code >>> 24; payload[5] = code >>> 16; payload[6] = code >>> 8; payload[7] = code; this._send(frame(7, 0, 0, Buffer.concat([payload, opaque]))); }; Http2Session.prototype.close = function(callback) { if (callback) this.once('close', callback); if (this.closed) return; this.closed = true; this.goaway(); this.destroy(); }; Http2Session.prototype.destroy = function(error) { if (this.destroyed) return; this.destroyed = true; if (this._handle) __thaw_net_destroy(this._handle); this._handle = 0; if (error) this.emit('error', error); this.emit('close'); }; Http2Session.prototype.ref = function() { return this; }; Http2Session.prototype.unref = function() { return this; };
             Http2Session.prototype.destroy = function(error) { if (this.destroyed) return; this.destroyed = true; if (this._handle) { if (this._transport === 'tls') __thaw_tls_destroy(this._handle); else __thaw_net_destroy(this._handle); } this._handle = 0; if (error) this.emit('error', error); this.emit('close'); };
             function ClientHttp2Session(handle, transport) { Http2Session.call(this, handle, false, transport); } ClientHttp2Session.prototype = Object.create(Http2Session.prototype); ClientHttp2Session.prototype.constructor = ClientHttp2Session;
             function ServerHttp2Session(handle, transport) { Http2Session.call(this, handle, true, transport); } ServerHttp2Session.prototype = Object.create(Http2Session.prototype); ServerHttp2Session.prototype.constructor = ServerHttp2Session;
             function connect(authority, options, listener) { if (typeof options === 'function') { listener = options; options = {}; } options = options || {}; var target = authority instanceof URL ? authority : new URL(String(authority)), port = Number(target.port || (target.protocol === 'https:' ? 443 : 80)), transport = target.protocol === 'https:' ? 'tls' : 'tcp', outcome; if (transport === 'tls') { var ca = options.ca, cert = options.cert, key = options.key; if (Array.isArray(ca)) ca = ca[0]; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; outcome = __thaw_tls_connect_with_options(target.hostname, port, options.servername || target.hostname, ca === undefined ? '' : Buffer.from(ca).toString('hex'), cert === undefined ? '' : Buffer.from(cert).toString('hex'), key === undefined ? '' : Buffer.from(key).toString('hex'), (options.rejectUnauthorized === false ? '0' : '1') + '|' + Buffer.from('h2').toString('hex')); } else if (target.protocol === 'http:') outcome = __thaw_net_connect(target.hostname, port); else { var protocolError = new Error('Unsupported protocol ' + target.protocol); protocolError.code = 'ERR_HTTP2_UNSUPPORTED_PROTOCOL'; throw protocolError; } if (outcome.indexOf('ok:') !== 0) throw new Error(outcome.substring(4)); var parts = outcome.split(':'); if (transport === 'tls' && Buffer.from(parts[2] || '', 'hex').toString() !== 'h2') { __thaw_tls_destroy(Number(parts[1])); var alpnError = new Error('Remote peer did not negotiate h2'); alpnError.code = 'ERR_HTTP2_ALPN_NEGOTIATION_FAILED'; throw alpnError; } var session = new ClientHttp2Session(Number(parts[1]), transport); session.authority = target.host; session.socket = { encrypted: transport === 'tls', alpnProtocol: session.alpnProtocol }; if (listener) session.once('connect', listener); session._start(); queueMicrotask(function() { session.emit('connect', session, null); }); return session; }
             function Http2Server(options, listener, secure) { EventEmitter.call(this); if (typeof options === 'function') { listener = options; options = {}; } this.options = options || {}; this._secure = Boolean(secure); this.listening = false; this._handle = 0; this._sessions = new Set(); this._refed = true; if (listener) this.on('stream', listener); }
             Http2Server.prototype = Object.create(EventEmitter.prototype); Http2Server.prototype.constructor = Http2Server; Http2Server.prototype.listen = function(port, host, callback) { if (typeof host === 'function') { callback = host; host = undefined; } host = host || '127.0.0.1'; if (callback) this.once('listening', callback); var outcome; if (this._secure) { var cert = this.options.cert, key = this.options.key, ca = this.options.ca; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; if (Array.isArray(ca)) ca = ca[0]; if (cert === undefined || key === undefined) throw new Error('cert and key are required'); var flags = (this.options.requestCert ? 1 : 0) | (this.options.rejectUnauthorized !== false ? 2 : 0); outcome = __thaw_tls_server_listen_with_options(String(host), Number(port || 0), Buffer.from(cert).toString('hex'), Buffer.from(key).toString('hex'), ca === undefined ? '' : Buffer.from(ca).toString('hex'), flags, Buffer.from('h2').toString('hex')); } else outcome = __thaw_net_listen(String(host), Number(port || 0)); if (outcome.indexOf('ok:') !== 0) throw new Error(outcome.substring(4)); var parts = outcome.split(':'), server = this; this._handle = Number(parts[1]); this._address = { address: String(host), family: String(host).indexOf(':') >= 0 ? 'IPv6' : 'IPv4', port: Number(parts[2]) }; this.listening = true; function accept() { if (!server.listening || !server._handle) return; var value = server._secure ? __thaw_tls_server_poll_accept(server._handle) : __thaw_net_poll_accept(server._handle); if (value === 'err:pending') { setTimeout(accept, 0); return; } if (value.indexOf('ok:') !== 0) { server.emit('sessionError', new Error(value.substring(4))); setTimeout(accept, 1); return; } var peer = value.split(':'), transport = server._secure ? 'tls' : 'tcp'; if (server._secure && Buffer.from(peer[4] || '', 'hex').toString() !== 'h2') { __thaw_tls_destroy(Number(peer[1])); server.emit('unknownProtocol', { alpnProtocol: Buffer.from(peer[4] || '', 'hex').toString() }); setTimeout(accept, 0); return; } var session = new ServerHttp2Session(Number(peer[1]), transport); server._sessions.add(session); session.socket = { remoteAddress: peer[2], remotePort: Number(peer[3]), encrypted: server._secure, alpnProtocol: session.alpnProtocol }; session.on('stream', function(stream, headers, flags, rawHeaders) { server.emit('stream', stream, headers, flags, rawHeaders); }); session.once('close', function() { server._sessions.delete(session); }); server.emit('session', session); session._start(); setTimeout(accept, 0); } queueMicrotask(function() { server.emit('listening'); accept(); }); return this; }; Http2Server.prototype.address = function() { return this._address || null; }; Http2Server.prototype.close = function(callback) { if (callback) this.once('close', callback); if (this._handle) { if (this._secure) __thaw_tls_server_close(this._handle); else __thaw_net_close_listener(this._handle); } this._handle = 0; this.listening = false; var server = this; this._sessions.forEach(function(session) { session.close(); }); queueMicrotask(function() { server.emit('close'); }); return this; }; Http2Server.prototype.closeAllConnections = Http2Server.prototype.closeIdleConnections = function() { this._sessions.forEach(function(session) { session.destroy(); }); }; Http2Server.prototype.setTimeout = function(timeout, callback) { this.timeout = Number(timeout); if (callback) this.on('timeout', callback); return this; }; Http2Server.prototype.ref = function() { this._refed = true; return this; }; Http2Server.prototype.unref = function() { this._refed = false; return this; };
             function Http2SecureServer(options, listener) { Http2Server.call(this, options, listener, true); } Http2SecureServer.prototype = Object.create(Http2Server.prototype); Http2SecureServer.prototype.constructor = Http2SecureServer; function createServer(options, listener) { return new Http2Server(options, listener, false); } function createSecureServer(options, listener) { return new Http2SecureServer(options, listener); }
             module.exports = { connect: connect, createServer: createServer, createSecureServer: createSecureServer, getDefaultSettings: getDefaultSettings, getPackedSettings: getPackedSettings, getUnpackedSettings: getUnpackedSettings, sensitiveHeaders: sensitiveHeaders, constants: constants, Http2Session: Http2Session, ClientHttp2Session: ClientHttp2Session, ServerHttp2Session: ServerHttp2Session, Http2Stream: Http2Stream, ClientHttp2Stream: ClientHttp2Stream, ServerHttp2Stream: ServerHttp2Stream, Http2Server: Http2Server, Http2SecureServer: Http2SecureServer }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "test" => Some(
            r#"var EventEmitter = require('node:events'), assertion = require('node:assert'), root = { name: '', parent: null, tests: [], before: [], after: [], beforeEach: [], afterEach: [] }, declaring = root, chain = Promise.resolve(), history = [], resultBus = new EventEmitter(), sequence = 1;
             function MockTracker() { this._restores = []; this.timers = { enabled: false, enable: function() { this.enabled = true; }, reset: function() { this.enabled = false; }, tick: function(milliseconds) { return new Promise(function(resolve) { setTimeout(resolve, Number(milliseconds) || 0); }); }, runAll: function() { return Promise.resolve(); } }; }
             MockTracker.prototype.fn = function(implementation, options) { implementation = typeof implementation === 'function' ? implementation : function() {}; options = options || {}; var calls = [], once = [], limit = options.times === undefined ? Infinity : Number(options.times), wrapper = function() { var record = { arguments: Array.prototype.slice.call(arguments), this: this, target: new.target }, selected = once.length ? once.shift() : implementation; calls.push(record); try { record.result = selected.apply(this, arguments); return record.result; } catch (error) { record.error = error; throw error; } }; Object.defineProperty(wrapper, 'mock', { value: { calls: calls, callCount: function() { return calls.length; }, resetCalls: function() { calls.length = 0; }, mockImplementation: function(value) { implementation = value; return wrapper; }, mockImplementationOnce: function(value) { once.push(value); return wrapper; }, restore: function() {} } }); if (Number.isFinite(limit)) { var original = wrapper; wrapper = function() { if (calls.length >= limit) throw new Error('mock function call limit exceeded'); return original.apply(this, arguments); }; Object.defineProperty(wrapper, 'mock', { value: original.mock }); } return wrapper; };
             MockTracker.prototype.method = function(object, name, implementation) { if (!object) throw new TypeError('object is required'); var descriptor = Object.getOwnPropertyDescriptor(object, name), original = object[name], replacement = this.fn(implementation || original); object[name] = replacement; var restore = function() { if (descriptor) Object.defineProperty(object, name, descriptor); else delete object[name]; }; replacement.mock.restore = restore; this._restores.push(restore); return replacement; }; MockTracker.prototype.getter = function(object, name, implementation) { var descriptor = Object.getOwnPropertyDescriptor(object, name); Object.defineProperty(object, name, { configurable: true, enumerable: descriptor ? descriptor.enumerable : true, get: this.fn(implementation) }); var restore = function() { if (descriptor) Object.defineProperty(object, name, descriptor); else delete object[name]; }; this._restores.push(restore); }; MockTracker.prototype.setter = function(object, name, implementation) { var descriptor = Object.getOwnPropertyDescriptor(object, name); Object.defineProperty(object, name, { configurable: true, enumerable: descriptor ? descriptor.enumerable : true, set: this.fn(implementation) }); var restore = function() { if (descriptor) Object.defineProperty(object, name, descriptor); else delete object[name]; }; this._restores.push(restore); }; MockTracker.prototype.reset = function() { this.restoreAll(); }; MockTracker.prototype.restoreAll = function() { this._restores.splice(0).reverse().forEach(function(restore) { restore(); }); };
             var mock = new MockTracker();
             function TestContext(record) { this.name = record.name; this.fullName = record.fullName; this.signal = new AbortController().signal; this.mock = new MockTracker(); this.assert = assertion; this._diagnostics = []; this._plan = null; this._assertions = 0; this._record = record; }
             TestContext.prototype.diagnostic = function(message) { this._diagnostics.push(String(message)); }; TestContext.prototype.plan = function(count) { this._plan = Number(count); }; TestContext.prototype.skip = function(message) { var error = new Error(message || 'test skipped'); error.__testSkip = true; throw error; }; TestContext.prototype.todo = function(message) { var error = new Error(message || 'test todo'); error.__testTodo = true; throw error; }; TestContext.prototype.test = function(name, options, fn) { return runCase(normalizeTest(name, options, fn, this._record.suite), this._record.suite); }; TestContext.prototype.before = function(fn) { this._record.localBefore.push(fn); }; TestContext.prototype.after = function(fn) { this._record.localAfter.push(fn); }; TestContext.prototype.beforeEach = function(fn) { this._record.localBeforeEach.push(fn); }; TestContext.prototype.afterEach = function(fn) { this._record.localAfterEach.push(fn); };
             function normalizeTest(name, options, fn, suite) { if (typeof name === 'function') { fn = name; name = fn.name || '<anonymous>'; options = {}; } else if (typeof options === 'function') { fn = options; options = {}; } options = options || {}; return { id: sequence++, name: String(name), fullName: (suite && suite.name ? suite.name + ' > ' : '') + String(name), options: options, fn: fn || function() {}, suite: suite || root, localBefore: [], localAfter: [], localBeforeEach: [], localAfterEach: [] }; }
             function ancestors(suite) { var values = []; while (suite) { values.unshift(suite); suite = suite.parent; } return values; }
             async function invokeAll(functions, context) { for (var fn of functions) await fn(context); }
             function publish(result) { history.push(result); resultBus.emit('result', result); resultBus.emit('test:' + result.status, result); }
             async function runCase(record) { var start = Date.now(), context = new TestContext(record), suites = ancestors(record.suite), result = { type: 'test', name: record.name, fullName: record.fullName, nesting: Math.max(0, suites.length - 1), status: 'passed', duration_ms: 0, diagnostics: context._diagnostics }; if (record.options.skip) { result.status = 'skipped'; result.message = typeof record.options.skip === 'string' ? record.options.skip : undefined; publish(result); return result; } if (record.options.todo) { result.status = 'todo'; result.message = typeof record.options.todo === 'string' ? record.options.todo : undefined; publish(result); return result; } try { await invokeAll(suites.flatMap(function(suite) { return suite.beforeEach; }).concat(record.localBefore, record.localBeforeEach), context); await record.fn(context); if (context._plan !== null && context._plan !== context._assertions) throw new Error('plan expected ' + context._plan + ' assertions but saw ' + context._assertions); } catch (error) { if (error && error.__testSkip) { result.status = 'skipped'; result.message = error.message; } else if (error && error.__testTodo) { result.status = 'todo'; result.message = error.message; } else { result.status = 'failed'; result.error = error; result.message = error && error.message || String(error); } } finally { try { await invokeAll(record.localAfterEach.concat(record.localAfter).concat(suites.slice().reverse().flatMap(function(suite) { return suite.afterEach; })), context); } catch (error) { result.status = 'failed'; result.error = error; result.message = error.message; } context.mock.restoreAll(); result.duration_ms = Date.now() - start; publish(result); } return result; }
             function enqueue(operation) { var promise = chain.then(operation); chain = promise.catch(function() {}); return promise; }
             function test(name, options, fn) { var record = normalizeTest(name, options, fn, declaring); if (declaring !== root) { declaring.tests.push({ test: record }); return Promise.resolve(record); } return enqueue(function() { return runCase(record); }); }
             function describe(name, options, fn) { if (typeof options === 'function') { fn = options; options = {}; } options = options || {}; var suite = { name: String(name), parent: declaring, tests: [], before: [], after: [], beforeEach: [], afterEach: [], options: options }, previous = declaring; declaring = suite; try { fn(); } finally { declaring = previous; } if (previous !== root) { previous.tests.push({ suite: suite }); return Promise.resolve(suite); } return enqueue(function() { return runSuite(suite); }); }
             async function runSuite(suite) { var context = { name: suite.name, signal: new AbortController().signal }; if (suite.options.skip) { var skipped = { type: 'suite', name: suite.name, status: 'skipped', duration_ms: 0 }; publish(skipped); return skipped; } var start = Date.now(), status = 'passed', error; try { await invokeAll(suite.before, context); for (var item of suite.tests) { var result = item.test ? await runCase(item.test) : await runSuite(item.suite); if (result.status === 'failed') status = 'failed'; } } catch (failure) { status = 'failed'; error = failure; } finally { try { await invokeAll(suite.after, context); } catch (failure) { status = 'failed'; error = failure; } } var result = { type: 'suite', name: suite.name, status: status, duration_ms: Date.now() - start, error: error }; publish(result); return result; }
             function hook(kind, fn) { declaring[kind].push(fn); } function before(fn) { hook('before', fn); } function after(fn) { hook('after', fn); } function beforeEach(fn) { hook('beforeEach', fn); } function afterEach(fn) { hook('afterEach', fn); }
             function TestsStream() { EventEmitter.call(this); this._queue = history.slice(); this._waiters = []; this._closed = false; var self = this; this._listener = function(result) { if (self._waiters.length) self._waiters.shift()({ value: result, done: false }); else self._queue.push(result); self.emit('data', result); }; resultBus.on('result', this._listener); } TestsStream.prototype = Object.create(EventEmitter.prototype); TestsStream.prototype.constructor = TestsStream; TestsStream.prototype[Symbol.asyncIterator] = function() { return this; }; TestsStream.prototype.next = function() { if (this._queue.length) return Promise.resolve({ value: this._queue.shift(), done: false }); if (this._closed) return Promise.resolve({ done: true }); var self = this; return new Promise(function(resolve) { self._waiters.push(resolve); }); }; TestsStream.prototype.close = function() { if (this._closed) return; this._closed = true; resultBus.off('result', this._listener); while (this._waiters.length) this._waiters.shift()({ done: true }); this.emit('end'); };
             function run(options) { var stream = new TestsStream(); chain.finally(function() { queueMicrotask(function() { stream.close(); }); }); return stream; }
             test.skip = function(name, options, fn) { if (typeof options === 'function') { fn = options; options = {}; } return test(name, Object.assign({}, options || {}, { skip: true }), fn); }; test.todo = function(name, options, fn) { if (typeof options === 'function') { fn = options; options = {}; } return test(name, Object.assign({}, options || {}, { todo: true }), fn); }; test.only = test; describe.skip = function(name, options, fn) { if (typeof options === 'function') { fn = options; options = {}; } return describe(name, Object.assign({}, options || {}, { skip: true }), fn); }; describe.only = describe;
             Object.assign(test, { test: test, it: test, describe: describe, suite: describe, before: before, after: after, beforeEach: beforeEach, afterEach: afterEach, run: run, mock: mock, assert: assertion, skip: test.skip, todo: test.todo, only: test.only, snapshot: { setDefaultSnapshotSerializers: function() {} } }); module.exports = test; module.exports.default = test; module.exports.__esModule = true;
"#,
        ),
        "test/reporters" => Some(
            r#"async function* records(source) { for await (var event of source) yield event; }
             async function* dot(source) { for await (var event of records(source)) if (event.type === 'test') yield event.status === 'passed' ? '.' : event.status === 'failed' ? 'X' : '-'; }
             async function* spec(source) { for await (var event of records(source)) yield (event.status === 'passed' ? '✔ ' : event.status === 'failed' ? '✖ ' : '- ') + event.fullName + '\n'; }
             async function* tap(source) { var index = 0; yield 'TAP version 13\n'; for await (var event of records(source)) if (event.type === 'test') yield (++index) + (event.status === 'passed' ? ' ok ' : ' not ok ') + '- ' + event.fullName + '\n'; yield '1..' + index + '\n'; }
             async function* junit(source) { yield '<testsuite>'; for await (var event of records(source)) if (event.type === 'test') yield '<testcase name="' + String(event.fullName).replace(/&/g, '&amp;').replace(/"/g, '&quot;') + '">' + (event.status === 'failed' ? '<failure>' + String(event.message || '') + '</failure>' : '') + '</testcase>'; yield '</testsuite>'; }
             async function* lcov(source) { for await (var event of records(source)) if (event.coverage) yield String(event.coverage); }
             module.exports = { dot: dot, spec: spec, tap: tap, junit: junit, lcov: lcov }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "wasi" => Some(
            r#"function validateOptions(options) {
               options = options || {};
               if (options.version !== undefined && options.version !== 'preview1') { var error = new TypeError('The "version" option must be "preview1"'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
               var args = options.args === undefined ? [] : options.args;
               if (!Array.isArray(args) || args.some(function(value) { return typeof value !== 'string'; })) { var argsError = new TypeError('The "args" option must be an array of strings'); argsError.code = 'ERR_INVALID_ARG_TYPE'; throw argsError; }
               var env = options.env === undefined ? {} : options.env, preopens = options.preopens === undefined ? {} : options.preopens;
               if (!env || typeof env !== 'object' || Array.isArray(env) || Object.keys(env).some(function(key) { return typeof env[key] !== 'string'; })) { var envError = new TypeError('The "env" option must be an object of strings'); envError.code = 'ERR_INVALID_ARG_TYPE'; throw envError; }
               if (!preopens || typeof preopens !== 'object' || Array.isArray(preopens) || Object.keys(preopens).some(function(key) { return typeof preopens[key] !== 'string'; })) { var preopenError = new TypeError('The "preopens" option must be an object of strings'); preopenError.code = 'ERR_INVALID_ARG_TYPE'; throw preopenError; }
               return { args: args.slice(), env: Object.assign({}, env), preopens: Object.assign({}, preopens), returnOnExit: Boolean(options.returnOnExit), version: 'preview1' };
             }
             function WASI(options) { if (!(this instanceof WASI)) throw new TypeError("Class constructor WASI cannot be invoked without 'new'"); this._options = validateOptions(options); this._started = false; this._import = Object.freeze({ __thawWasiOptions: JSON.stringify(this._options) }); }
             WASI.prototype.getImportObject = function() { return { wasi_snapshot_preview1: this._import }; };
             Object.defineProperty(WASI.prototype, 'wasiImport', { enumerable: true, get: function() { return this._import; } });
             WASI.prototype._invoke = function(instance, name) { if (!(instance instanceof WebAssembly.Instance)) { var typeError = new TypeError('instance must be a WebAssembly.Instance'); typeError.code = 'ERR_INVALID_ARG_TYPE'; throw typeError; } if (this._started) { var stateError = new Error('WASI instance has already started'); stateError.code = 'ERR_WASI_ALREADY_STARTED'; throw stateError; } if (!(instance.exports.memory instanceof WebAssembly.Memory)) { var memoryError = new Error('WASI instance must export a memory'); memoryError.code = 'ERR_WASI_NOT_STARTED'; throw memoryError; } if (typeof instance.exports[name] !== 'function') { var exportError = new Error('WASI instance does not export ' + name); exportError.code = 'ERR_WASI_NOT_STARTED'; throw exportError; } this._started = true; try { instance.exports[name](); return 0; } catch (error) { if (error && error.__thawWasiExit !== undefined) { var code = Number(error.__thawWasiExit); if (!this._options.returnOnExit) process.exitCode = code; return code; } throw error; } };
             WASI.prototype.start = function(instance) { var code = this._invoke(instance, '_start'); return this._options.returnOnExit ? code : undefined; };
             WASI.prototype.initialize = function(instance) { if (instance && instance.exports && typeof instance.exports._start === 'function') { var error = new Error('WASI reactor must not export _start'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; } this._invoke(instance, '_initialize'); };
             module.exports = { WASI: WASI }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "https" => Some(
            "var http = require('node:http'), tls = require('node:tls'); module.exports = http.__createSecureModule(tls); module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "buffer" => Some(
            "function byteLength(value, encoding) { return globalThis.Buffer.byteLength(value, encoding); }\n\
             function isUtf8(value) { try { new TextDecoder('utf-8', { fatal: true }).decode(value); return true; } catch (_) { return false; } }\n\
             function isAscii(value) { return Array.from(value).every(function(byte) { return byte <= 127; }); }\n\
             function transcode(value, fromEncoding, toEncoding) { return globalThis.Buffer.from(globalThis.Buffer.from(value).toString(fromEncoding), toEncoding); }\n\
             module.exports = { Buffer: globalThis.Buffer, SlowBuffer: globalThis.SlowBuffer, byteLength: byteLength, isUtf8: isUtf8, isAscii: isAscii, transcode: transcode, atob: globalThis.atob, btoa: globalThis.btoa, constants: { MAX_LENGTH: 4294967296, MAX_STRING_LENGTH: 536870888 } };\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "string_decoder" => Some(
            "function normalizeEncoding(encoding) { var value = String(encoding || 'utf8').toLowerCase().replace(/[-_]/g, ''); if (!globalThis.Buffer.isEncoding(value)) throw new TypeError('Unknown encoding: ' + encoding); return value; }\n\
             function utf8CompleteLength(buffer) {\n\
             \x20\x20var index = buffer.length - 1; while (index >= 0 && (buffer[index] & 192) === 128) index--;\n\
             \x20\x20if (index < 0) return Math.max(0, buffer.length - Math.min(buffer.length, 3));\n\
             \x20\x20var lead = buffer[index]; var expected = lead >= 240 && lead <= 247 ? 4 : lead >= 224 && lead <= 239 ? 3 : lead >= 192 && lead <= 223 ? 2 : 1;\n\
             \x20\x20return buffer.length - index < expected ? index : buffer.length;\n\
             }\n\
             function StringDecoder(encoding) {\n\
             \x20\x20if (!(this instanceof StringDecoder)) return new StringDecoder(encoding);\n\
             \x20\x20this.encoding = normalizeEncoding(encoding); this._pending = globalThis.Buffer.alloc(0); this.lastNeed = 0; this.lastTotal = 0; this.lastChar = globalThis.Buffer.alloc(4);\n\
             }\n\
             StringDecoder.prototype.write = function(value) {\n\
             \x20\x20var input = globalThis.Buffer.from(value); var combined = this._pending.length ? globalThis.Buffer.concat([this._pending, input]) : input; var complete = combined.length;\n\
             \x20\x20if (this.encoding === 'utf8' || this.encoding === 'utf') complete = utf8CompleteLength(combined);\n\
             \x20\x20else if (this.encoding === 'utf16le' || this.encoding === 'ucs2') complete -= complete % 2;\n\
             \x20\x20else if (this.encoding === 'base64' || this.encoding === 'base64url') complete -= complete % 3;\n\
             \x20\x20this._pending = globalThis.Buffer.from(combined.subarray(complete)); this.lastNeed = this._pending.length; this.lastTotal = complete === combined.length ? 0 : this.lastNeed + 1;\n\
             \x20\x20return combined.subarray(0, complete).toString(this.encoding);\n\
             };\n\
             StringDecoder.prototype.text = function(value, offset) { return this.write(globalThis.Buffer.from(value).subarray(offset || 0)); };\n\
             StringDecoder.prototype.end = function(value) { var output = value === undefined ? '' : this.write(value); if (this._pending.length) output += this._pending.toString(this.encoding); this._pending = globalThis.Buffer.alloc(0); this.lastNeed = 0; this.lastTotal = 0; return output; };\n\
             module.exports = { StringDecoder: StringDecoder };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "timers" => Some(
            "module.exports = { setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout, setInterval: globalThis.setInterval, clearInterval: globalThis.clearInterval, setImmediate: globalThis.setImmediate, clearImmediate: globalThis.clearImmediate };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "timers/promises" => Some(
            "function abortReason(signal) { return signal && signal.reason !== undefined ? signal.reason : new DOMException('This operation was aborted', 'AbortError'); }\n\
             function promiseTimer(schedule, delay, value, options) {\n\
             \x20\x20options = options || {}; var signal = options.signal;\n\
             \x20\x20return new Promise(function(resolve, reject) {\n\
             \x20\x20\x20\x20if (signal && signal.aborted) { reject(abortReason(signal)); return; }\n\
             \x20\x20\x20\x20var id; function aborted() { if (schedule === globalThis.setImmediate) globalThis.clearImmediate(id); else globalThis.clearTimeout(id); reject(abortReason(signal)); }\n\
             \x20\x20\x20\x20id = schedule(function() { if (signal) signal.removeEventListener('abort', aborted); resolve(value); }, delay);\n\
             \x20\x20\x20\x20if (signal) signal.addEventListener('abort', aborted, { once: true });\n\
             \x20\x20});\n\
             }\n\
             function setTimeoutPromise(delay, value, options) { return promiseTimer(globalThis.setTimeout, delay, value, options); }\n\
             function setImmediatePromise(value, options) { return promiseTimer(globalThis.setImmediate, 0, value, options); }\n\
             function setIntervalPromise(delay, value, options) {\n\
             \x20\x20options = options || {}; var signal = options.signal; var values = []; var waiters = []; var done = false; var failure;\n\
             \x20\x20var id = globalThis.setInterval(function() { var result = { value: value, done: false }; if (waiters.length) waiters.shift().resolve(result); else values.push(result); }, delay);\n\
             \x20\x20function stop(error) { if (done) return; done = true; failure = error; globalThis.clearInterval(id); while (waiters.length) { var waiter = waiters.shift(); if (error) waiter.reject(error); else waiter.resolve({ value: undefined, done: true }); } }\n\
             \x20\x20if (signal) { if (signal.aborted) stop(abortReason(signal)); else signal.addEventListener('abort', function() { stop(abortReason(signal)); }, { once: true }); }\n\
             \x20\x20return { next: function() { if (values.length) return Promise.resolve(values.shift()); if (done) return failure ? Promise.reject(failure) : Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { waiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { stop(); return Promise.resolve({ value: undefined, done: true }); }, [Symbol.asyncIterator]: function() { return this; } };\n\
             }\n\
             var scheduler = { wait: function(delay, options) { return setTimeoutPromise(delay, undefined, options); }, yield: function() { return setImmediatePromise(); } };\n\
             module.exports = { setTimeout: setTimeoutPromise, setImmediate: setImmediatePromise, setInterval: setIntervalPromise, scheduler: scheduler };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "readline" => Some(
            "function Interface(input, output, completer, terminal) { if (!(this instanceof Interface)) return new Interface(input, output, completer, terminal); var options = input && input.input ? input : { input: input, output: output, completer: completer, terminal: terminal }; this.input = options.input || null; this.output = options.output || null; this.completer = options.completer || null; this.terminal = Boolean(options.terminal); this.history = Array.isArray(options.history) ? options.history.slice() : []; this.historySize = options.historySize === undefined ? 30 : Number(options.historySize); this.removeHistoryDuplicates = Boolean(options.removeHistoryDuplicates); this.line = ''; this.cursor = 0; this.closed = false; this.paused = false; this._prompt = options.prompt === undefined ? '> ' : String(options.prompt); this._events = Object.create(null); this._buffer = ''; this._iteratorValues = []; this._iteratorWaiters = []; var self = this; this._onData = function(chunk) { self.write(chunk); }; this._onEnd = function() { if (self._buffer) { self._emitLine(self._buffer); self._buffer = ''; } self.close(); }; if (this.input && this.input.on) { this.input.on('data', this._onData); this.input.on('end', this._onEnd); } }\n\
             Interface.prototype.on = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; }; Interface.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; }; Interface.prototype.off = Interface.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; Interface.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; };\n\
             Interface.prototype._emitLine = function(line) { this.line = String(line); this.cursor = this.line.length; if (this.line && this.historySize > 0) { if (!this.removeHistoryDuplicates || this.history[0] !== this.line) this.history.unshift(this.line); this.history.length = Math.min(this.history.length, this.historySize); } this.emit('line', this.line); var result = { value: this.line, done: false }; if (this._iteratorWaiters.length) this._iteratorWaiters.shift().resolve(result); else this._iteratorValues.push(result); }; Interface.prototype.write = function(data) { if (this.closed) return; this._buffer += Buffer.isBuffer(data) ? data.toString() : String(data); var lines = this._buffer.split(/\\r?\\n|\\r/); this._buffer = lines.pop(); for (var index = 0; index < lines.length; index++) this._emitLine(lines[index]); };\n\
             Interface.prototype.setPrompt = function(prompt) { this._prompt = String(prompt); }; Interface.prototype.getPrompt = function() { return this._prompt; }; Interface.prototype.prompt = function() { if (this.output && this.output.write) this.output.write(this._prompt); return this; }; Interface.prototype.question = function(query, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } options = options || {}; if (this.output && this.output.write) this.output.write(String(query)); var self = this; function answered(value) { if (options.signal) options.signal.removeEventListener('abort', aborted); callback(value); } function aborted() { self.off('line', answered); } if (options.signal) { if (options.signal.aborted) return; options.signal.addEventListener('abort', aborted, { once: true }); } this.once('line', answered); }; Interface.prototype.pause = function() { this.paused = true; if (this.input && this.input.pause) this.input.pause(); this.emit('pause'); return this; }; Interface.prototype.resume = function() { this.paused = false; if (this.input && this.input.resume) this.input.resume(); this.emit('resume'); return this; }; Interface.prototype.close = function() { if (this.closed) return; this.closed = true; if (this.input && this.input.off) { this.input.off('data', this._onData); this.input.off('end', this._onEnd); } this.emit('close'); while (this._iteratorWaiters.length) this._iteratorWaiters.shift().resolve({ value: undefined, done: true }); }; Interface.prototype[Symbol.asyncIterator] = function() { var self = this; return { next: function() { if (self._iteratorValues.length) return Promise.resolve(self._iteratorValues.shift()); if (self.closed) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { self._iteratorWaiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { self.close(); return Promise.resolve({ value: undefined, done: true }); }, [Symbol.asyncIterator]: function() { return this; } }; };\n\
             function createInterface() { return new (Function.prototype.bind.apply(Interface, [null].concat(Array.prototype.slice.call(arguments))))(); } function write(output, value, callback) { if (!output || typeof output.write !== 'function') return false; var result = output.write(value); if (typeof callback === 'function') queueMicrotask(callback); return result !== false; } function clearLine(output, direction, callback) { return write(output, '\\u001b[' + (direction < 0 ? '1' : direction > 0 ? '0' : '2') + 'K', callback); } function clearScreenDown(output, callback) { return write(output, '\\u001b[0J', callback); } function cursorTo(output, x, y, callback) { if (typeof y === 'function') { callback = y; y = undefined; } return write(output, y === undefined ? '\\u001b[' + (Number(x) + 1) + 'G' : '\\u001b[' + (Number(y) + 1) + ';' + (Number(x) + 1) + 'H', callback); } function moveCursor(output, dx, dy, callback) { var value = ''; dx = Number(dx); dy = Number(dy); if (dx < 0) value += '\\u001b[' + (-dx) + 'D'; else if (dx > 0) value += '\\u001b[' + dx + 'C'; if (dy < 0) value += '\\u001b[' + (-dy) + 'A'; else if (dy > 0) value += '\\u001b[' + dy + 'B'; return write(output, value, callback); }\n\
             module.exports = { Interface: Interface, ReadLine: Interface, createInterface: createInterface, clearLine: clearLine, clearScreenDown: clearScreenDown, cursorTo: cursorTo, moveCursor: moveCursor, emitKeypressEvents: function() {} }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "readline/promises" => Some(
            "function Interface(input, output) { if (!(this instanceof Interface)) return new Interface(input, output); var options = input && input.input ? input : { input: input, output: output }; this.input = options.input || null; this.output = options.output || null; this.closed = false; this.line = ''; this._events = Object.create(null); this._buffer = ''; this._iteratorValues = []; this._iteratorWaiters = []; var self = this; this._onData = function(chunk) { self.write(chunk); }; this._onEnd = function() { self.close(); }; if (this.input && this.input.on) { this.input.on('data', this._onData); this.input.on('end', this._onEnd); } } Interface.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push(listener); return this; }; Interface.prototype.off = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry !== listener; }); return this; }; Interface.prototype._emitLine = function(line) { this.line = line; var listeners = (this._events.line || []).slice(); this._events.line = []; listeners.forEach(function(listener) { listener(line); }); var result = { value: line, done: false }; if (this._iteratorWaiters.length) this._iteratorWaiters.shift()(result); else this._iteratorValues.push(result); }; Interface.prototype.write = function(data) { this._buffer += Buffer.isBuffer(data) ? data.toString() : String(data); var lines = this._buffer.split(/\\r?\\n|\\r/); this._buffer = lines.pop(); for (var index = 0; index < lines.length; index++) this._emitLine(lines[index]); }; Interface.prototype.question = function(query, options) { var self = this; options = options || {}; if (this.output && this.output.write) this.output.write(String(query)); return new Promise(function(resolve, reject) { if (options.signal && options.signal.aborted) { reject(options.signal.reason); return; } function answered(value) { if (options.signal) options.signal.removeEventListener('abort', aborted); resolve(value); } function aborted() { self.off('line', answered); reject(options.signal.reason); } if (options.signal) options.signal.addEventListener('abort', aborted, { once: true }); self.once('line', answered); }); }; Interface.prototype.close = function() { if (this.closed) return; this.closed = true; while (this._iteratorWaiters.length) this._iteratorWaiters.shift()({ value: undefined, done: true }); }; Interface.prototype[Symbol.asyncIterator] = function() { var self = this; return { next: function() { if (self._iteratorValues.length) return Promise.resolve(self._iteratorValues.shift()); if (self.closed) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve) { self._iteratorWaiters.push(resolve); }); }, return: function() { self.close(); return Promise.resolve({ value: undefined, done: true }); }, [Symbol.asyncIterator]: function() { return this; } }; }; function createInterface() { return new (Function.prototype.bind.apply(Interface, [null].concat(Array.prototype.slice.call(arguments))))(); } module.exports = { Interface: Interface, ReadLine: Interface, createInterface: createInterface }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "stream" => Some(
            "var defaultByteHighWaterMark = 16384, defaultObjectHighWaterMark = 16; function getDefaultHighWaterMark(objectMode) { return objectMode ? defaultObjectHighWaterMark : defaultByteHighWaterMark; } function setDefaultHighWaterMark(objectMode, value) { value = Number(value); if (!Number.isInteger(value) || value < 0) throw new RangeError('highWaterMark must be a non-negative integer'); if (objectMode) defaultObjectHighWaterMark = value; else defaultByteHighWaterMark = value; } function resolveHighWaterMark(options, objectMode) { var value = options.highWaterMark === undefined ? getDefaultHighWaterMark(objectMode) : Number(options.highWaterMark); if (!Number.isInteger(value) || value < 0) throw new RangeError('highWaterMark must be a non-negative integer'); return value; } function normalizeStreamEncoding(encoding) { var value = String(encoding || 'utf8').toLowerCase().replace(/[-_]/g, ''); if (!globalThis.Buffer.isEncoding(value)) throw new TypeError('Unknown encoding: ' + encoding); return value; } function streamUtf8CompleteLength(buffer) { var index = buffer.length - 1; while (index >= 0 && (buffer[index] & 192) === 128) index--; if (index < 0) return Math.max(0, buffer.length - Math.min(buffer.length, 3)); var lead = buffer[index], expected = lead >= 240 && lead <= 247 ? 4 : lead >= 224 && lead <= 239 ? 3 : lead >= 192 && lead <= 223 ? 2 : 1; return buffer.length - index < expected ? index : buffer.length; } function StreamStringDecoder(encoding) { this.encoding = normalizeStreamEncoding(encoding); this.pending = globalThis.Buffer.alloc(0); } StreamStringDecoder.prototype.write = function(value) { var input = globalThis.Buffer.from(value), combined = this.pending.length ? globalThis.Buffer.concat([this.pending, input]) : input, complete = combined.length; if (this.encoding === 'utf8' || this.encoding === 'utf') complete = streamUtf8CompleteLength(combined); else if (this.encoding === 'utf16le' || this.encoding === 'ucs2') { complete -= complete % 2; if (complete >= 2) { var unit = combined[complete - 2] | combined[complete - 1] << 8; if (unit >= 55296 && unit <= 56319) complete -= 2; } } else if (this.encoding === 'base64' || this.encoding === 'base64url') complete -= complete % 3; this.pending = globalThis.Buffer.from(combined.subarray(complete)); return combined.subarray(0, complete).toString(this.encoding); }; StreamStringDecoder.prototype.end = function() { var output = this.pending.length ? this.pending.toString(this.encoding) : ''; this.pending = globalThis.Buffer.alloc(0); return output; }; function Stream() { this._events = Object.create(null); this.destroyed = false; this.closed = false; this.errored = null; this._readableDidRead = false; this._constructState = 'ready'; this._constructWaiters = []; this._autoDestroy = true; this._emitClose = true; this._endEmitted = false; } function configureConstruct(stream, options) { if (stream._constructConfigured) return; if (typeof options.construct === 'function') stream._construct = options.construct; if (typeof stream._construct !== 'function') return; stream._constructConfigured = true; stream._constructState = 'pending'; queueMicrotask(function() { if (stream.destroyed) return; var completed = false; function finish(error) { if (completed) return; completed = true; if (error) { stream._constructState = 'failed'; stream._constructWaiters = []; stream.destroy(error); return; } stream._constructState = 'ready'; var waiters = stream._constructWaiters.splice(0); waiters.forEach(function(waiter) { waiter(); }); } try { stream._construct(finish); } catch (error) { finish(error); } }); } function afterConstruct(stream, action) { if (stream._constructState === 'ready') action(); else if (stream._constructState === 'pending') stream._constructWaiters.push(action); } function maybeAutoDestroy(stream) { if (!stream._autoDestroy || stream.destroyed) return; var readableDone = stream.readableEnded === undefined || stream.readableEnded, writableDone = stream.writableFinished === undefined || stream.writableFinished; if (readableDone && writableDone) queueMicrotask(function() { if (!stream.destroyed) stream.destroy(); }); } function emitReadableEnd(stream) { if (!stream._endEmitted && stream.readableEnded && stream._chunks.length === 0) { stream._endEmitted = true; stream.emit('end'); maybeAutoDestroy(stream); } }\n\
             function addStreamListener(stream, name, listener, once, prepend) { if ((stream._events.newListener || []).length) stream.emit('newListener', name, listener); var entry = { listener: listener, once: once, wrapper: null }; if (once) { entry.wrapper = function() { var list = stream._events[name] || [], index = list.indexOf(entry); if (index >= 0) { list.splice(index, 1); if ((stream._events.removeListener || []).length) stream.emit('removeListener', name, listener); } return listener.apply(this, arguments); }; entry.wrapper.listener = listener; } if (prepend) (stream._events[name] || (stream._events[name] = [])).unshift(entry); else (stream._events[name] || (stream._events[name] = [])).push(entry); if (name === 'data' && stream._drainReadable) stream.resume(); if (name === 'end' && stream.readableEnded) queueMicrotask(function() { listener(); }); return stream; } Stream.prototype.on = Stream.prototype.addListener = function(event, listener) { return addStreamListener(this, event, listener, false, false); };\
             Stream.prototype.once = function(event, listener) { return addStreamListener(this, event, listener, true, false); };\
             Stream.prototype.prependListener = function(event, listener) { return addStreamListener(this, event, listener, false, true); }; Stream.prototype.prependOnceListener = function(event, listener) { return addStreamListener(this, event, listener, true, true); };\
             Stream.prototype.off = Stream.prototype.removeListener = function(event, listener) { var name = event, list = this._events[name] || []; for (var index = list.length - 1; index >= 0; index--) { if (list[index].listener === listener || list[index].wrapper === listener) { var original = list[index].listener; list.splice(index, 1); if ((this._events.removeListener || []).length) this.emit('removeListener', name, original); break; } } return this; }; Stream.prototype.removeAllListeners = function(event) { var names = arguments.length ? [event] : Reflect.ownKeys(this._events); names.forEach(function(name) { if (name === 'removeListener') return; var list = (this._events[name] || []).slice().reverse(); delete this._events[name]; list.forEach(function(entry) { if ((this._events.removeListener || []).length) this.emit('removeListener', name, entry.listener); }, this); }, this); if (!arguments.length || event === 'removeListener') delete this._events.removeListener; return this; }; Stream.prototype.listeners = function(event) { return (this._events[event] || []).map(function(entry) { return entry.listener; }); }; Stream.prototype.rawListeners = function(event) { return (this._events[event] || []).map(function(entry) { return entry.wrapper || entry.listener; }); }; Stream.prototype.listenerCount = function(event, listener) { return (this._events[event] || []).filter(function(entry) { return !listener || entry.listener === listener || entry.wrapper === listener; }).length; }; Stream.prototype.eventNames = function() { return Reflect.ownKeys(this._events).filter(function(name) { return (this._events[name] || []).length > 0; }, this); }; Stream.prototype.setMaxListeners = function(value) { this._maxListeners = Number(value); return this; }; Stream.prototype.getMaxListeners = function() { return this._maxListeners === undefined ? 10 : this._maxListeners; };\
             Stream.prototype.emit = function(event) { var name = event; if (name === 'data') this._readableDidRead = true; var entries = (this._events[name] || []).slice(); var args = Array.prototype.slice.call(arguments, 1); if (name === 'error' && entries.length === 0) { if (args[0] instanceof Error) throw args[0]; var unhandled = new Error('Unhandled error' + (args.length ? ': ' + String(args[0]) : '')); unhandled.code = 'ERR_UNHANDLED_ERROR'; unhandled.context = args[0]; throw unhandled; } entries.forEach(function(entry) { (entry.wrapper || entry.listener).apply(this, args); }, this); return entries.length > 0; };\n\
             Stream.prototype.destroy = function(error) { if (this.destroyed) return this; this.destroyed = true; if (this.readable === true && !this.readableEnded) this.readableAborted = true; if (this.writable === true && !this.writableFinished) this.writableAborted = true; this.readable = false; this.writable = false; if (error) this.errored = error; var self = this, completed = false; function finish(finalError) { if (completed) return; completed = true; self.closed = true; finalError = finalError || error; if (finalError) self.errored = finalError; queueMicrotask(function() { if (finalError) self.emit('error', finalError); if (self._emitClose) self.emit('close'); }); } if (typeof this._destroy === 'function') { try { this._destroy(error || null, finish); } catch (destroyError) { if (!completed) finish(destroyError); } } else finish(error); return this; };\
             Stream.prototype._undestroy = function() { this.destroyed = false; this.closed = false; this.errored = null; if (this.readableEnded !== undefined) { this.readableAborted = false; this.readable = !this.readableEnded; } if (this.writableEnded !== undefined) { this.writableAborted = false; this.writable = !this.writableEnded; this._destroyError = null; } return this; };\
             Stream.prototype.pipe = function(destination, options) { var source = this, ended = false; function onData(chunk) { if (destination.writable === false) return; if (destination.write(chunk) === false && source.pause) source.pause(); } function onDrain() { if (source.resume) source.resume(); } function onEnd() { if (ended) return; ended = true; cleanup(); if (!options || options.end !== false) destination.end(); } function onClose() { if (ended) return; ended = true; cleanup(); if (destination.destroy) destination.destroy(); } function onError(error) { cleanup(); if (destination.destroy) destination.destroy(error); } function cleanup() { source.removeListener('data', onData); source.removeListener('end', onEnd); source.removeListener('close', onClose); source.removeListener('error', onError); destination.removeListener('drain', onDrain); } source.on('data', onData); source.once('end', onEnd); source.once('close', onClose); source.once('error', onError); destination.on('drain', onDrain); destination.emit('pipe', source); return destination; };\
             function inherit(child, parent) { child.prototype = Object.create(parent.prototype); child.prototype.constructor = child; }\n\
             function Readable(options) { Stream.call(this); options = options || {}; this._autoDestroy = options.autoDestroy !== false; this._emitClose = options.emitClose !== false; this.readable = true; this.readableEnded = false; this.readableAborted = false; this.readableFlowing = null; this.readableEncoding = null; this._decoder = null; this._readableObjectMode = Boolean(options.objectMode || options.readableObjectMode); this.readableObjectMode = this._readableObjectMode; this.readableHighWaterMark = resolveHighWaterMark(options, this._readableObjectMode); this.readableLength = 0; this._paused = false; this._chunks = []; if (typeof options.read === 'function') this._read = options.read; if (typeof options.destroy === 'function') this._destroy = options.destroy; configureConstruct(this, options); }\
             inherit(Readable, Stream);\n\
             Readable.prototype._read = function() {}; Readable.prototype._destroy = function(error, callback) { callback(error); };\
             Readable.prototype.setEncoding = function(encoding) { if (this._readableObjectMode) return this; this._decoder = new StreamStringDecoder(encoding); this.readableEncoding = this._decoder.encoding; var decoded = [], decoder = this._decoder; this._chunks.forEach(function(chunk) { var value = typeof chunk === 'string' ? chunk : decoder.write(chunk); if (value.length) decoded.push(value); }); this._chunks = decoded; this.readableLength = decoded.reduce(function(total, chunk) { return total + chunk.length; }, 0); return this; };\n\
             function readableChunkLength(stream, chunk) { return stream._readableObjectMode ? 1 : typeof chunk === 'string' ? (stream.readableEncoding ? chunk.length : Buffer.byteLength(chunk)) : chunk.length; } function enqueueReadableChunk(stream, value) { if (value === '' && !stream._readableObjectMode) return; if (!stream._paused && (stream._events.data || []).length) stream.emit('data', value); else { stream._chunks.push(value); stream.readableLength += readableChunkLength(stream, value); stream.emit('readable'); } } function readableInputError(stream, error) { stream.errored = error; stream.readableAborted = true; if (stream._autoDestroy) stream.destroy(error); else stream.emit('error', error); return false; } Readable.prototype.push = function(chunk, encoding) { if (this.destroyed) return false; if (chunk === null) { if (this._decoder) enqueueReadableChunk(this, this._decoder.end()); this.readableEnded = true; this.readable = false; emitReadableEnd(this); return false; } if (this.readableEnded) { var eofError = new Error('stream.push() after EOF'); eofError.code = 'ERR_STREAM_PUSH_AFTER_EOF'; return readableInputError(this, eofError); } if (chunk === undefined) return true; var value; if (this._readableObjectMode) value = chunk; else if (typeof chunk === 'string') value = globalThis.Buffer.from(chunk, encoding); else if (globalThis.ArrayBuffer.isView(chunk)) value = globalThis.Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength); else { var typeError = new TypeError('The chunk argument must be of type string or an instance of Buffer, TypedArray, or DataView'); typeError.code = 'ERR_INVALID_ARG_TYPE'; return readableInputError(this, typeError); } if (this._decoder && !this._readableObjectMode) value = this._decoder.write(value); enqueueReadableChunk(this, value); return this.readableLength < this.readableHighWaterMark; };\
             Readable.prototype._drainReadable = function() { while (!this._paused && this._chunks.length) { var value = this._chunks.shift(); this.readableLength -= readableChunkLength(this, value); if ((this._events.data || []).length) this.emit('data', value); } if (!this._paused) emitReadableEnd(this); }; Readable.prototype.pause = function() { this._paused = true; this.readableFlowing = false; this.emit('pause'); return this; }; Readable.prototype.resume = function() { this._paused = false; this.readableFlowing = true; this.emit('resume'); var self = this; if (!this._chunks.length && !this.readableEnded) afterConstruct(this, function() { self._read(self.readableHighWaterMark); }); this._drainReadable(); return this; }; Readable.prototype.isPaused = function() { return this._paused; };\
             Readable.prototype.read = function(size) { this._readableDidRead = true; if (this._chunks.length === 0) { if (!this.readableEnded) { var self = this; afterConstruct(this, function() { self._read(size); }); } if (this._chunks.length === 0) return null; } if (size === undefined) { if (this._chunks.length === 1) { var only = this._chunks.shift(); this.readableLength = 0; return only; } if (this._readableObjectMode) { this.readableLength--; return this._chunks.shift(); } if (this.readableEncoding) { var text = this._chunks.join(''); this._chunks = []; this.readableLength = 0; return text; } var complete = globalThis.Buffer.concat(this._chunks); this._chunks = []; this.readableLength = 0; return complete; } if (this._readableObjectMode) { this.readableLength--; return this._chunks.shift(); } if (this.readableEncoding) { var joinedText = this._chunks.join(''), characterCount = Math.max(0, Math.min(Number(size), joinedText.length)), textResult = joinedText.slice(0, characterCount), textRemainder = joinedText.slice(characterCount); this._chunks = textRemainder ? [textRemainder] : []; this.readableLength = textRemainder.length; return textResult; } var joined = globalThis.Buffer.concat(this._chunks), count = Math.max(0, Math.min(Number(size), joined.length)), result = joined.subarray(0, count), remainder = joined.subarray(count); this._chunks = remainder.length ? [remainder] : []; this.readableLength = remainder.length; return result; };\
             Readable.prototype.read = function(size) { this._readableDidRead = true; var numericSize = Number(size), requested = size === undefined || !Number.isFinite(numericSize) ? undefined : Math.floor(numericSize), pulled = false; if (requested !== undefined && requested < 0) requested = 0; if (requested === 0) { if (!this.readableEnded && this._chunks.length === 0) { var zeroStream = this; afterConstruct(this, function() { zeroStream._read(0); }); } emitReadableEnd(this); return null; } if (this._chunks.length === 0 && !this.readableEnded) { var stream = this; pulled = true; afterConstruct(this, function() { stream._read(requested); }); } if (this._chunks.length === 0) { emitReadableEnd(this); return null; } if (this._readableObjectMode) { this.readableLength--; var object = this._chunks.shift(); emitReadableEnd(this); return object; } var joined = this.readableEncoding ? this._chunks.join('') : globalThis.Buffer.concat(this._chunks), available = joined.length; if (requested !== undefined && requested > available && !this.readableEnded) { if (!pulled) { var waitingStream = this; afterConstruct(this, function() { waitingStream._read(requested); }); joined = this.readableEncoding ? this._chunks.join('') : globalThis.Buffer.concat(this._chunks); available = joined.length; } if (requested > available && !this.readableEnded) return null; } var count = requested === undefined ? available : Math.min(requested, available), result = this.readableEncoding ? joined.slice(0, count) : joined.subarray(0, count), remainder = this.readableEncoding ? joined.slice(count) : joined.subarray(count); this._chunks = remainder.length ? [remainder] : []; this.readableLength = remainder.length; emitReadableEnd(this); return result; };\n\
             function createReadableIterator(source, destroyOnReturn) { var finished = false; source.pause(); function take() { if (source._chunks.length) { var value = source._chunks.shift(); source.readableLength -= readableChunkLength(source, value); source._readableDidRead = true; emitReadableEnd(source); return Promise.resolve({ value: value, done: false }); } if (finished || source.readableEnded || source.destroyed) { emitReadableEnd(source); return Promise.resolve({ value: undefined, done: true }); } return new Promise(function(resolve, reject) { function cleanup() { source.removeListener('readable', readable); source.removeListener('end', end); source.removeListener('error', error); } function readable() { cleanup(); take().then(resolve, reject); } function end() { cleanup(); finished = true; resolve({ value: undefined, done: true }); } function error(reason) { cleanup(); finished = true; reject(reason); } source.once('readable', readable); source.once('end', end); source.once('error', error); }); } return { next: take, return: function() { finished = true; if (destroyOnReturn) source.destroy(); return Promise.resolve({ value: undefined, done: true }); }, throw: function(error) { finished = true; source.destroy(error); return Promise.reject(error); }, [Symbol.asyncIterator]: function() { return this; } }; } Readable.prototype[Symbol.asyncIterator] = function() { return createReadableIterator(this, true); }; Readable.prototype.iterator = function(options) { return createReadableIterator(this, !options || options.destroyOnReturn !== false); }; Readable.prototype[Symbol.asyncDispose] = async function() { if (!this.destroyed) this.destroy(); }; Readable.prototype.pipe = function(destination, options) { var source = this; this.on('data', function(chunk) { if (!destination.write(chunk)) { source.pause(); destination.once('drain', function() { source.resume(); }); } }); if (!options || options.end !== false) this.once('end', function() { destination.end(); }); this.once('error', function(error) { destination.destroy(error); }); destination.emit('pipe', this); return destination; };\
             Readable.prototype.pipe = function(destination, options) { var source = this, entry = { destination: destination, cleaned: false }, endDestination = !options || options.end !== false; this._pipeEntries = this._pipeEntries || []; this._awaitDrainWriters = this._awaitDrainWriters || new Set(); function cleanup(emitUnpipe) { if (entry.cleaned) return; entry.cleaned = true; source.removeListener('data', onData); source.removeListener('end', onEnd); source.removeListener('close', onClose); source.removeListener('error', onError); destination.removeListener('drain', onDrain); source._awaitDrainWriters.delete(destination); var index = source._pipeEntries.indexOf(entry); if (index >= 0) source._pipeEntries.splice(index, 1); if (emitUnpipe) destination.emit('unpipe', source, { hasUnpiped: false }); if (source._awaitDrainWriters.size === 0 && source._paused && !source.destroyed) source.resume(); } function onData(chunk) { if (destination.writable === false || destination.destroyed) return; if (destination.write(chunk) === false) { source._awaitDrainWriters.add(destination); source.pause(); } } function onDrain() { source._awaitDrainWriters.delete(destination); if (source._awaitDrainWriters.size === 0) source.resume(); } function onEnd() { cleanup(true); if (endDestination) destination.end(); } function onClose() { cleanup(true); } function onError(error) { cleanup(true); if (destination.destroy) destination.destroy(error); } entry.cleanup = cleanup; this._pipeEntries.push(entry); this.once('end', onEnd); this.once('close', onClose); this.once('error', onError); destination.on('drain', onDrain); this.on('data', onData); destination.emit('pipe', this); return destination; }; Readable.prototype.unpipe = function(destination) { var entries = (this._pipeEntries || []).slice(); entries.forEach(function(entry) { if (!destination || entry.destination === destination) entry.cleanup(true); }); return this; };\n\
             Readable.prototype.unshift = function(chunk, encoding) { if (chunk === null) return this.push(null); if (this.destroyed) return false; if (this._endEmitted) { var endError = new Error('stream.unshift() after end event'); endError.code = 'ERR_STREAM_UNSHIFT_AFTER_END_EVENT'; return readableInputError(this, endError); } if (chunk === undefined) return true; var value; if (this._readableObjectMode) value = chunk; else if (typeof chunk === 'string') value = globalThis.Buffer.from(chunk, encoding); else if (globalThis.ArrayBuffer.isView(chunk)) value = globalThis.Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength); else { var typeError = new TypeError('The chunk argument must be of type string or an instance of Buffer, TypedArray, or DataView'); typeError.code = 'ERR_INVALID_ARG_TYPE'; return readableInputError(this, typeError); } if (this._decoder && !this._readableObjectMode) value = this._decoder.write(value); if ((typeof value === 'string' || value.length !== undefined) && value.length === 0 && !this._readableObjectMode) return this.readableLength < this.readableHighWaterMark; this._chunks.unshift(value); this.readableLength += readableChunkLength(this, value); this.emit('readable'); return this.readableLength < this.readableHighWaterMark; }; Readable.prototype.wrap = function(source) { var output = this; source.on('data', function(chunk) { if (!output.push(chunk) && source.pause) source.pause(); }); source.once('end', function() { output.push(null); }); source.once('error', function(error) { output.destroy(error); }); this._read = function() { if (source.resume) source.resume(); }; return this; };\n\
             var unshiftReadable = Readable.prototype.unshift; Readable.prototype.unshift = function() { var result = unshiftReadable.apply(this, arguments); return this.readableEnded ? false : result; }; Readable.prototype.compose = function(destination, options) { var output = compose(this, destination); if (options && options.signal) addAbortSignal(options.signal, output); return output; }; Object.defineProperties(Readable.prototype, { readableDidRead: { get: function() { return this._readableDidRead; } }, readableBuffer: { get: function() { return this._chunks; } } });\n\
             Readable.from = function(iterable, options) { var iterator; if (typeof iterable === 'string' || globalThis.ArrayBuffer.isView(iterable)) iterator = [iterable][Symbol.iterator](); else { if (iterable === null || iterable === undefined || (typeof iterable[Symbol.asyncIterator] !== 'function' && typeof iterable[Symbol.iterator] !== 'function')) { var typeError = new TypeError('The \"iterable\" argument must be an instance of Iterable'); typeError.code = 'ERR_INVALID_ARG_TYPE'; throw typeError; } iterator = typeof iterable[Symbol.asyncIterator] === 'function' ? iterable[Symbol.asyncIterator]() : iterable[Symbol.iterator](); } var stream = new Readable(Object.assign({ objectMode: true }, options || {})); queueMicrotask(async function() { try { while (!stream.destroyed) { var item = await iterator.next(); if (item.done) { stream.push(null); return; } var value = await item.value; if (value === null) { var nullError = new TypeError('May not write null values to stream'); nullError.code = 'ERR_STREAM_NULL_VALUES'; throw nullError; } stream.push(value); } } catch (error) { stream.destroy(error); } finally { if (stream.destroyed && iterator.return) { try { await iterator.return(); } catch (_) {} } } }); return stream; }; function readableOperationOptions(options) { options = options || {}; if (options.signal && options.signal.aborted) throw options.signal.reason || new DOMException('The operation was aborted', 'AbortError'); return options; } function checkReadableOperation(options) { if (options.signal && options.signal.aborted) throw options.signal.reason || new DOMException('The operation was aborted', 'AbortError'); } Readable.prototype.map = function(mapper, options) { var source = this; options = readableOperationOptions(options); return Readable.from((async function*() { var index = 0; for await (var value of source) { checkReadableOperation(options); yield await mapper(value, { signal: options.signal, index: index++ }); } })()); }; Readable.prototype.filter = function(predicate, options) { var source = this; options = readableOperationOptions(options); return Readable.from((async function*() { var index = 0; for await (var value of source) { checkReadableOperation(options); if (await predicate(value, { signal: options.signal, index: index++ })) yield value; } })()); }; Readable.prototype.flatMap = function(mapper, options) { var source = this; options = readableOperationOptions(options); return Readable.from((async function*() { var index = 0; for await (var value of source) { checkReadableOperation(options); var mapped = await mapper(value, { signal: options.signal, index: index++ }); if (mapped && (mapped[Symbol.asyncIterator] || mapped[Symbol.iterator])) { for await (var child of mapped) yield child; } else yield mapped; } })()); }; Readable.prototype.drop = function(limit, options) { var source = this; options = readableOperationOptions(options); limit = Math.max(0, Number(limit)); return Readable.from((async function*() { var index = 0; for await (var value of source) { checkReadableOperation(options); if (index++ >= limit) yield value; } })()); }; Readable.prototype.take = function(limit, options) { var source = this; options = readableOperationOptions(options); limit = Math.max(0, Number(limit)); return Readable.from((async function*() { var index = 0; for await (var value of source) { checkReadableOperation(options); if (index++ >= limit) break; yield value; } })()); }; Readable.prototype.toArray = async function(options) { options = readableOperationOptions(options); var values = []; for await (var value of this) { checkReadableOperation(options); values.push(value); } return values; }; Readable.prototype.forEach = async function(callback, options) { options = readableOperationOptions(options); var index = 0; for await (var value of this) { checkReadableOperation(options); await callback(value, { signal: options.signal, index: index++ }); } }; Readable.prototype.some = async function(predicate, options) { options = readableOperationOptions(options); var index = 0; for await (var value of this) { checkReadableOperation(options); if (await predicate(value, { signal: options.signal, index: index++ })) return true; } return false; }; Readable.prototype.every = async function(predicate, options) { options = readableOperationOptions(options); var index = 0; for await (var value of this) { checkReadableOperation(options); if (!await predicate(value, { signal: options.signal, index: index++ })) return false; } return true; }; Readable.prototype.find = async function(predicate, options) { options = readableOperationOptions(options); var index = 0; for await (var value of this) { checkReadableOperation(options); if (await predicate(value, { signal: options.signal, index: index++ })) return value; } return undefined; }; Readable.prototype.reduce = async function(reducer, initial, options) { var hasInitial = arguments.length >= 2; options = readableOperationOptions(options); var accumulator = initial, index = 0; for await (var value of this) { checkReadableOperation(options); if (!hasInitial) { accumulator = value; hasInitial = true; } else accumulator = await reducer(accumulator, value, { signal: options.signal, index: index }); index++; } if (!hasInitial) throw new TypeError('Reduce of an empty stream requires an initial value'); return accumulator; };\n\
             function readableConcurrency(options) { var concurrency = options.concurrency === undefined ? 1 : Number(options.concurrency); if (!Number.isInteger(concurrency) || concurrency < 1) throw new RangeError('concurrency must be a positive integer'); return concurrency; } function concurrentReadableResults(source, operation, options) { var concurrency = readableConcurrency(options); return (async function*() { var iterator = source[Symbol.asyncIterator](), pending = [], index = 0, inputDone = false; async function fill() { while (!inputDone && pending.length < concurrency) { checkReadableOperation(options); var item = await iterator.next(); if (item.done) { inputDone = true; break; } var itemIndex = index++; pending.push(Promise.resolve().then(function(value, currentIndex) { return function() { return operation(value, { signal: options.signal, index: currentIndex }); }; }(item.value, itemIndex))); } } try { await fill(); while (pending.length) { checkReadableOperation(options); yield await pending.shift(); await fill(); } } finally { if (!inputDone && iterator.return) await iterator.return(); } })(); } Readable.prototype.map = function(mapper, options) { options = readableOperationOptions(options); return Readable.from(concurrentReadableResults(this, mapper, options)); }; Readable.prototype.filter = function(predicate, options) { options = readableOperationOptions(options); return Readable.from((async function*(results) { for await (var result of results) if (result.keep) yield result.value; })(concurrentReadableResults(this, async function(value, context) { return { value: value, keep: await predicate(value, context) }; }, options))); }; Readable.prototype.flatMap = function(mapper, options) { options = readableOperationOptions(options); return Readable.from((async function*(results) { for await (var mapped of results) { if (mapped && (mapped[Symbol.asyncIterator] || mapped[Symbol.iterator])) { for await (var child of mapped) yield child; } else yield mapped; } })(concurrentReadableResults(this, mapper, options))); }; Readable.prototype.forEach = async function(callback, options) { await this.map(async function(value, context) { await callback(value, context); }, options).toArray({ signal: options && options.signal }); };\n\
             Readable.prototype.asIndexedPairs = function(options) { return this.map(function(value, context) { return [context.index, value]; }, options); };\n\
             var pipelineImplementation = pipeline; pipeline = function() { var args = Array.prototype.slice.call(arguments), callback = args[args.length - 1]; if (typeof callback !== 'function') { var callbackError = new TypeError('The callback argument must be of type function'); callbackError.code = 'ERR_INVALID_ARG_TYPE'; throw callbackError; } var stages = args.slice(0, -1), count = stages.length === 1 && Array.isArray(stages[0]) ? stages[0].length : stages.length; if (count < 2) { var missing = new TypeError('The streams argument must be specified'); missing.code = 'ERR_MISSING_ARGS'; throw missing; } return pipelineImplementation.apply(this, args); }; var finishedImplementation = finished; finished = function(stream, options, callback) { if (typeof options === 'function') callback = options; if (typeof callback !== 'function') { var callbackError = new TypeError('The callback argument must be of type function'); callbackError.code = 'ERR_INVALID_ARG_TYPE'; throw callbackError; } if (!stream || typeof stream.once !== 'function' || typeof stream.removeListener !== 'function') { var streamError = new TypeError('The stream argument must be an instance of Stream'); streamError.code = 'ERR_INVALID_ARG_TYPE'; throw streamError; } return finishedImplementation.apply(this, arguments); };\n\
             pipelineImplementation = function() { var streams = Array.prototype.slice.call(arguments), callback = streams.pop(), options = streams.length && streams[streams.length - 1] && typeof streams[streams.length - 1].on !== 'function' && (streams[streams.length - 1].signal !== undefined || streams[streams.length - 1].end !== undefined) ? streams.pop() : {}; if (streams.length === 1 && Array.isArray(streams[0])) streams = streams[0]; var functional = streams.some(function(item) { return typeof item === 'function'; }) || !streams[0] || typeof streams[0].on !== 'function'; if (functional) { if (!options.signal) { var controller = new AbortController(); options = Object.assign({}, options, { signal: controller.signal, controller: controller }); } runFunctionalPipeline(streams, options).then(function(value) { callback(null, value); }, callback); return streams[streams.length - 1]; } var settled = false, listeners = [], abort, lastIndex = streams.length - 1, leaveOpen = options.end === false; function listen(stream, event, listener) { stream.once(event, listener); listeners.push({ stream: stream, event: event, listener: listener }); } function cleanup() { listeners.forEach(function(entry) { entry.stream.removeListener(entry.event, entry.listener); }); if (options.signal && abort) options.signal.removeEventListener('abort', abort); } function finish(error) { if (settled) return; settled = true; cleanup(); if (error) { protectPipelineErrors(streams); streams.forEach(function(item) { if (item.destroy) item.destroy(error); }); } callback(error || null); } streams.forEach(function(item, index) { listen(item, 'error', function(error) { finish(error); }); listen(item, 'close', function() { var readableIncomplete = index < lastIndex && item.readableEnded !== true, writableIncomplete = index > 0 && !(leaveOpen && index === lastIndex) && item.writableFinished !== true; if (readableIncomplete || writableIncomplete) { var error = new Error('Premature close'); error.code = 'ERR_STREAM_PREMATURE_CLOSE'; finish(error); } }); }); for (var index = 0; index < lastIndex; index++) streams[index].pipe(streams[index + 1], leaveOpen && index + 1 === lastIndex ? { end: false } : undefined); if (leaveOpen) listen(streams[lastIndex - 1], 'end', function() { finish(); }); else listen(streams[lastIndex], 'finish', function() { finish(); }); abort = function() { finish(pipelineAbortError(options.signal)); }; if (options.signal) { if (options.signal.aborted) abort(); else options.signal.addEventListener('abort', abort, { once: true }); } return streams[lastIndex]; };\n\
             var composeImplementation = compose; compose = function() { var stages = Array.prototype.slice.call(arguments); if (!stages.length) { var missing = new TypeError('The streams argument must be specified'); missing.code = 'ERR_MISSING_ARGS'; throw missing; } if (stages.length > 1) stages.forEach(function(stage, index) { var needsReadable = index < stages.length - 1, needsWritable = index > 0; if (!stage || (typeof stage !== 'function' && ((needsReadable && (stage.readable !== true || typeof stage.pipe !== 'function')) || (needsWritable && (stage.writable !== true || typeof stage.write !== 'function'))))) { var invalid = new TypeError('Invalid stream stage at index ' + index); invalid.code = 'ERR_INVALID_ARG_VALUE'; throw invalid; } }); stages = stages.map(function(stage) { return typeof stage === 'function' ? Duplex.from(stage) : stage; }); if (stages.length > 1 && stages[stages.length - 1].readable === false) return writableComposition(stages); return composeImplementation.apply(this, stages); };\n\
             function initWritableState(target, options) { options = options || {}; target._autoDestroy = options.autoDestroy !== false; target._emitClose = options.emitClose !== false; target._decodeStrings = options.decodeStrings !== false; target.writable = true; target.writableEnded = false; target.writableFinished = false; target.writableAborted = false; target._writableObjectMode = Boolean(options.objectMode || options.writableObjectMode); target.writableObjectMode = target._writableObjectMode; target.writableHighWaterMark = resolveHighWaterMark(options, target._writableObjectMode); target.writableLength = 0; target.writableNeedDrain = false; target._writeQueue = []; target._writing = false; target._ending = false; target._finishing = false; target._needsDrain = false; target._constructPumpQueued = false; target._defaultEncoding = options.defaultEncoding || 'utf8'; target._corked = 0; target.writableCorked = 0; target._endCallbacks = []; if (typeof options.write === 'function') target._write = options.write; if (typeof options.writev === 'function') target._writev = options.writev; if (typeof options.final === 'function') target._final = options.final; if (typeof options.destroy === 'function') target._destroy = options.destroy; configureConstruct(target, options); } function Writable(options) { Stream.call(this); initWritableState(this, options); }\n\
             inherit(Writable, Stream);\n\
             Writable.prototype._write = function(chunk, encoding, callback) { var error = new Error('The _write() method is not implemented'); error.code = 'ERR_METHOD_NOT_IMPLEMENTED'; callback(error); }; Writable.prototype._destroy = function(error, callback) { callback(error); };\
             Writable.prototype._pumpWrites = function() { var self = this; if (this._constructState !== 'ready') { if (this._constructState === 'pending' && !this._constructPumpQueued) { this._constructPumpQueued = true; afterConstruct(this, function() { self._constructPumpQueued = false; self._pumpWrites(); }); } return; } if (this._writing || this._corked) return; if (!this._writeQueue.length) { if (this._ending) this._finishWrites(); return; } this._writing = true; var entries = this._writev && this._writeQueue.length > 1 ? this._writeQueue.splice(0) : [this._writeQueue.shift()], total = entries.reduce(function(sum, entry) { return sum + entry.length; }, 0), complete = function(error) { self._writing = false; self.writableLength -= total; var finalError = error || self._destroyError; entries.forEach(function(entry) { if (entry.callback) entry.callback(finalError); }); if (self.destroyed) return; if (error) { self.destroy(error); return; } if (self._needsDrain && self.writableLength < self.writableHighWaterMark) { self._needsDrain = false; self.writableNeedDrain = false; self.emit('drain'); } self._pumpWrites(); }; if (this._writev && entries.length > 1) this._writev(entries.map(function(entry) { return { chunk: entry.chunk, encoding: entry.encoding }; }), complete); else this._write(entries[0].chunk, entries[0].encoding, complete); }; Writable.prototype._finishWrites = function() { if (this._finishing || this.writableFinished) return; this._finishing = true; var self = this, finish = function(error) { if (error) { self.destroy(error); return; } self.writable = false; self.writableFinished = true; self.writableNeedDrain = false; self.emit('finish'); self._endCallbacks.splice(0).forEach(function(callback) { callback(); }); }; if (this._final) this._final(finish); else finish(); }; Writable.prototype.write = function(chunk, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.writableEnded) { var error = new Error('write after end'); error.code = 'ERR_STREAM_WRITE_AFTER_END'; throw error; } encoding = encoding || this._defaultEncoding; var value = this._writableObjectMode ? chunk : typeof chunk === 'string' ? globalThis.Buffer.from(chunk, encoding) : globalThis.Buffer.from(chunk), length = this._writableObjectMode ? 1 : value.length; this._writeQueue.push({ chunk: value, encoding: this._writableObjectMode ? this._defaultEncoding : typeof chunk === 'string' ? encoding : 'buffer', callback: callback, length: length }); this.writableLength += length; var available = this.writableLength < this.writableHighWaterMark; if (!available) { this._needsDrain = true; this.writableNeedDrain = true; } this._pumpWrites(); return available; }; Writable.prototype.setDefaultEncoding = function(encoding) { encoding = String(encoding).toLowerCase(); if (!/^(utf8|utf-8|utf16le|utf-16le|ucs2|ucs-2|latin1|binary|ascii|base64|base64url|hex)$/.test(encoding)) throw new TypeError('Unknown encoding: ' + encoding); this._defaultEncoding = encoding; return this; };\n\
             Writable.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk !== undefined) this.write(chunk, encoding); if (callback) this._endCallbacks.push(callback); this.writableEnded = true; this._ending = true; this._corked = 0; this.writableCorked = 0; this._pumpWrites(); return this; }; Writable.prototype.destroy = function(error) { if (this.destroyed) return this; var callbackError = error || new Error('Cannot call write after a stream was destroyed'); if (!callbackError.code) callbackError.code = 'ERR_STREAM_DESTROYED'; this._destroyError = callbackError; this._ending = false; this.writableNeedDrain = false; var pending = this._writeQueue.splice(0), self = this; this.writableLength -= pending.reduce(function(total, entry) { return total + entry.length; }, 0); queueMicrotask(function() { pending.forEach(function(entry) { if (entry.callback) entry.callback(callbackError); }); self._endCallbacks.splice(0).forEach(function(callback) { callback(callbackError); }); }); return Stream.prototype.destroy.call(this, error); }; Writable.prototype.cork = function() { this._corked++; this.writableCorked = this._corked; }; Writable.prototype.uncork = function() { if (this._corked > 0) this._corked--; this.writableCorked = this._corked; if (!this._corked) this._pumpWrites(); };\
             Writable.prototype.write = function(chunk, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk === null) { var nullError = new TypeError('May not write null values to stream'); nullError.code = 'ERR_STREAM_NULL_VALUES'; throw nullError; } if (this.writableEnded) { var endedStream = this, endedError = new Error('write after end'); endedError.code = 'ERR_STREAM_WRITE_AFTER_END'; queueMicrotask(function() { if (callback) callback(endedError); endedStream.errored = endedError; endedStream.emit('error', endedError); }); return false; } if (this.destroyed) { var destroyedError = new Error('Cannot call write after a stream was destroyed'); destroyedError.code = 'ERR_STREAM_DESTROYED'; if (callback) queueMicrotask(function() { callback(destroyedError); }); return false; } encoding = encoding || this._defaultEncoding; var value, length; if (this._writableObjectMode) { value = chunk; length = 1; } else if (typeof chunk === 'string') { value = this._decodeStrings ? globalThis.Buffer.from(chunk, encoding) : chunk; length = this._decodeStrings ? globalThis.Buffer.byteLength(chunk, encoding) : chunk.length; if (this._decodeStrings) encoding = 'buffer'; } else if (globalThis.ArrayBuffer.isView(chunk)) { value = globalThis.Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength); length = value.length; encoding = 'buffer'; } else { var typeError = new TypeError('The chunk argument must be of type string or an instance of Buffer, TypedArray, or DataView'); typeError.code = 'ERR_INVALID_ARG_TYPE'; throw typeError; } this._writeQueue.push({ chunk: value, encoding: this._writableObjectMode ? this._defaultEncoding : encoding, callback: callback, length: length }); this.writableLength += length; var available = this.writableLength < this.writableHighWaterMark; if (!available) { this._needsDrain = true; this.writableNeedDrain = true; } this._pumpWrites(); return available; }; Writable.prototype._finishWrites = function() { if (this._finishing || this.writableFinished) return; this._finishing = true; var self = this, finish = function(error) { if (error) { self.destroy(error); return; } queueMicrotask(function() { if (self.destroyed) return; self.writable = false; self.writableFinished = true; self.writableNeedDrain = false; self._endCallbacks.splice(0).forEach(function(callback) { callback(); }); self.emit('finish'); }); }; if (this._final) this._final(finish); else finish(); }; Writable.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; encoding = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.writableEnded) { if (chunk !== undefined) this.write(chunk, encoding, callback); else if (callback) { if (this.writableFinished) queueMicrotask(callback); else this._endCallbacks.push(callback); } return this; } var self = this; this.once('finish', function() { maybeAutoDestroy(self); }); if (chunk !== undefined) this.write(chunk, encoding); if (callback) this._endCallbacks.push(callback); this.writableEnded = true; this._ending = true; this._corked = 0; this.writableCorked = 0; this._pumpWrites(); return this; }; Writable.prototype.destroySoon = Writable.prototype.end; Writable.prototype[Symbol.asyncDispose] = async function() { if (!this.destroyed && !this.writableFinished) this.destroy(); };\
             var destroyWritable = Writable.prototype.destroy; Writable.prototype.destroy = function(error) { var originalCode = error && error.code, result = destroyWritable.call(this, error); if (error && !originalCode) delete error.code; this._destroyError = null; return result; };\n\
             var writeWritable = Writable.prototype.write; Writable.prototype.write = function() { var result = writeWritable.apply(this, arguments); return this.destroyed ? false : result; };\n\
             Writable.prototype._pumpWrites = function() { var self = this; if (this._constructState !== 'ready') { if (this._constructState === 'pending' && !this._constructPumpQueued) { this._constructPumpQueued = true; afterConstruct(this, function() { self._constructPumpQueued = false; self._pumpWrites(); }); } return; } if (this._writing || this._corked) return; if (!this._writeQueue.length) { if (this._ending) this._finishWrites(); return; } this._writing = true; var entries = this._writev && this._writeQueue.length > 1 ? this._writeQueue.splice(0) : [this._writeQueue.shift()], total = entries.reduce(function(sum, entry) { return sum + entry.length; }, 0), synchronous = true, completed = false; function complete(error) { if (completed) { var repeated = new Error('Callback called multiple times'); repeated.code = 'ERR_MULTIPLE_CALLBACK'; self.destroy(repeated); return; } completed = true; self._writing = false; self.writableLength -= total; var callbacks = function() { entries.forEach(function(entry) { if (entry.callback) entry.callback(error); }); }, deliver = function() { if (synchronous) queueMicrotask(callbacks); else callbacks(); }; if (self.destroyed) { deliver(); return; } if (error) { deliver(); self.destroy(error); return; } if (self._needsDrain && self.writableLength < self.writableHighWaterMark) { self._needsDrain = false; self.writableNeedDrain = false; self.emit('drain'); } self._pumpWrites(); deliver(); } if (this._writev && entries.length > 1) this._writev(entries.map(function(entry) { return { chunk: entry.chunk, encoding: entry.encoding }; }), complete); else this._write(entries[0].chunk, entries[0].encoding, complete); synchronous = false; };\n\
             Writable.prototype._finishWrites = function() { if (this._finishing || this.writableFinished) return; this._finishing = true; var self = this, completed = false; function fail(error) { var callbacks = self._endCallbacks.splice(0); self.destroy(error); self.once('close', function() { callbacks.forEach(function(callback) { callback(error); }); }); } function finish(error) { if (completed) { var repeated = new Error('Callback called multiple times'); repeated.code = 'ERR_MULTIPLE_CALLBACK'; fail(repeated); return; } completed = true; if (error) { fail(error); return; } queueMicrotask(function() { if (self.destroyed) return; self.writable = false; self.writableFinished = true; self.writableNeedDrain = false; self._endCallbacks.splice(0).forEach(function(callback) { callback(); }); self.emit('finish'); }); } if (this._final) { try { this._final(finish); } catch (error) { finish(error); } } else finish(); };\n\
             Writable.prototype.pipe = function(destination) { var source = this; queueMicrotask(function() { if (source.destroyed) return; var error = new Error('Cannot pipe, not readable'); error.code = 'ERR_STREAM_CANNOT_PIPE'; source.destroy(error); }); return destination; };\
             Readable.prototype.pause = function() { if (this._paused) return this; this._paused = true; this.readableFlowing = false; this.emit('pause'); return this; }; Readable.prototype.resume = function() { if (!this._paused && this.readableFlowing === true) return this; this._paused = false; this.readableFlowing = true; if (!this._resumeScheduled) { var resumeStream = this; this._resumeScheduled = true; queueMicrotask(function() { resumeStream._resumeScheduled = false; if (!resumeStream.destroyed) resumeStream.emit('resume'); }); } var self = this; if (!this._chunks.length && !this.readableEnded) afterConstruct(this, function() { self._read(self.readableHighWaterMark); }); this._drainReadable(); return this; }; Object.defineProperty(Writable.prototype, 'writableBuffer', { get: function() { return this._writeQueue; } });\
             function Duplex(options) { options = options || {}; Readable.call(this, options); initWritableState(this, options); this.allowHalfOpen = options.allowHalfOpen !== false; if (!this.allowHalfOpen) { var self = this; this.once('end', function() { if (!self.writableEnded) self.end(); }); } }\
             inherit(Duplex, Readable); Duplex.prototype.write = Writable.prototype.write; Duplex.prototype.end = Writable.prototype.end; Duplex.prototype._write = Writable.prototype._write; Duplex.prototype._pumpWrites = Writable.prototype._pumpWrites; Duplex.prototype._finishWrites = Writable.prototype._finishWrites; Duplex.prototype.cork = Writable.prototype.cork; Duplex.prototype.uncork = Writable.prototype.uncork; Duplex.prototype.destroy = Writable.prototype.destroy;\
             function Transform(options) { options = options || {}; Duplex.call(this, options); if (typeof options.transform === 'function') this._transform = options.transform; if (typeof options.flush === 'function') this._flush = options.flush; var self = this; this._write = function(chunk, encoding, callback) { self._transform(chunk, encoding, function(error, output) { if (!error && output !== undefined && output !== null) self.push(output); callback(error); }); }; this._final = function(callback) { function finish(error, output) { if (!error && output !== undefined && output !== null) self.push(output); if (!error) self.push(null); callback(error); } if (self._flush) self._flush(finish); else finish(); }; }\
             inherit(Transform, Duplex); Transform.prototype.write = Writable.prototype.write; Transform.prototype.end = Writable.prototype.end; Transform.prototype._pumpWrites = Writable.prototype._pumpWrites; Transform.prototype._finishWrites = Writable.prototype._finishWrites; Transform.prototype.cork = Writable.prototype.cork; Transform.prototype.uncork = Writable.prototype.uncork; Transform.prototype.destroy = Writable.prototype.destroy;\
             Transform.prototype._transform = function(chunk, encoding, callback) { var error = new Error('The _transform() method is not implemented'); error.code = 'ERR_METHOD_NOT_IMPLEMENTED'; callback(error); };\
             function PassThrough(options) { Transform.call(this, options); } inherit(PassThrough, Transform); PassThrough.prototype._transform = function(chunk, encoding, callback) { callback(null, chunk); };\n\
             function finished(stream, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } options = options || {}; var settled = false, watchReadable = options.readable !== false && stream.readable !== undefined, watchWritable = options.writable !== false && stream.writable !== undefined, readableDone = !watchReadable || stream.readableEnded, writableDone = !watchWritable || stream.writableFinished, abort; function cleanup() { stream.removeListener('end', onEnd); stream.removeListener('finish', onFinish); stream.removeListener('error', onError); stream.removeListener('close', onClose); if (options.signal && abort) options.signal.removeEventListener('abort', abort); } function complete(error) { if (settled) return; settled = true; if (options.cleanup) cleanup(); queueMicrotask(function() { callback(error); }); } function maybeComplete() { if (readableDone && writableDone) complete(); } function onEnd() { readableDone = true; maybeComplete(); } function onFinish() { writableDone = true; maybeComplete(); } function onError(error) { complete(error); } function onClose() { if (options.error === false && stream.errored) { readableDone = true; writableDone = true; complete(); } else if (!readableDone || !writableDone) { var error = new Error('Premature close'); error.code = 'ERR_STREAM_PREMATURE_CLOSE'; complete(error); } else maybeComplete(); } stream.once('end', onEnd); stream.once('finish', onFinish); if (options.error !== false) stream.once('error', onError); stream.once('close', onClose); abort = function() { complete(pipelineAbortError(options.signal)); }; if (options.signal) { if (options.signal.aborted) abort(); else options.signal.addEventListener('abort', abort, { once: true }); } queueMicrotask(maybeComplete); return cleanup; }\n\
             function pipelineAbortError(signal) { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = signal.reason; return error; } function protectPipelineErrors(resources) { resources.forEach(function(resource) { if (resource && resource.on) resource.on('error', function() {}); }); } async function runFunctionalPipeline(stages, options) { var resources = [], abort, abortPromise; if (options.signal) abortPromise = new Promise(function(resolve, reject) { abort = function() { var error = pipelineAbortError(options.signal); protectPipelineErrors(resources); resources.forEach(function(resource) { if (resource.destroy && !resource.destroyed) resource.destroy(error); }); reject(error); }; if (options.signal.aborted) abort(); else options.signal.addEventListener('abort', abort, { once: true }); }); async function execute() { var first = stages[0], current = typeof first === 'function' ? await first({ signal: options.signal }) : first; if (!current || typeof current.on !== 'function') current = Readable.from(current); resources.push(current); for (var index = 1; index < stages.length; index++) { var stage = stages[index], last = index === stages.length - 1; if (typeof stage === 'function') { var produced = stage(current, { signal: options.signal }); if (last) return await produced; current = Readable.from(await produced); resources.push(current); } else { current.pipe(stage, last ? { end: options.end !== false } : undefined); current = stage; resources.push(current); } } if (current.writableFinished || current.readableEnded) return current; await new Promise(function(resolve, reject) { current.once('error', reject); if (current.writable !== undefined) current.once('finish', resolve); else current.once('end', resolve); }); return current; } try { return await (abortPromise ? Promise.race([execute(), abortPromise]) : execute()); } catch (error) { protectPipelineErrors(resources); resources.forEach(function(resource) { if (resource.destroy && !resource.destroyed) resource.destroy(error); else if (resource.destroyed && !resource.errored) resource.errored = error; }); if (options.controller && !options.controller.signal.aborted) options.controller.abort(error); throw error; } finally { if (options.signal && abort) options.signal.removeEventListener('abort', abort); } } function pipeline() { var streams = Array.prototype.slice.call(arguments), callback = typeof streams[streams.length - 1] === 'function' ? streams.pop() : function(error) { if (error) throw error; }, options = streams.length && streams[streams.length - 1] && typeof streams[streams.length - 1].on !== 'function' && (streams[streams.length - 1].signal !== undefined || streams[streams.length - 1].end !== undefined) ? streams.pop() : {}; if (streams.length === 1 && Array.isArray(streams[0])) streams = streams[0]; var functional = streams.some(function(item) { return typeof item === 'function'; }) || !streams[0] || typeof streams[0].on !== 'function'; if (functional) { if (!options.signal) { var controller = new AbortController(); options = Object.assign({}, options, { signal: controller.signal, controller: controller }); } runFunctionalPipeline(streams, options).then(function(value) { callback(null, value); }, callback); return streams[streams.length - 1]; } var settled = false, listeners = [], abort; function cleanup() { listeners.forEach(function(entry) { entry.stream.removeListener(entry.event, entry.listener); }); if (options.signal && abort) options.signal.removeEventListener('abort', abort); } function finish(error) { if (settled) return; settled = true; cleanup(); if (error) { protectPipelineErrors(streams); streams.forEach(function(item) { if (item.destroy) item.destroy(error); }); } callback(error || null); } streams.forEach(function(item) { var listener = function(error) { finish(error); }; item.once('error', listener); listeners.push({ stream: item, event: 'error', listener: listener }); }); for (var index = 0; index + 1 < streams.length; index++) streams[index].pipe(streams[index + 1]); var last = streams[streams.length - 1], completed = function() { finish(); }; last.once('finish', completed); listeners.push({ stream: last, event: 'finish', listener: completed }); abort = function() { finish(pipelineAbortError(options.signal)); }; if (options.signal) { if (options.signal.aborted) abort(); else options.signal.addEventListener('abort', abort, { once: true }); } return last; }\
             function pipeline() { var streams = Array.prototype.slice.call(arguments), callback = typeof streams[streams.length - 1] === 'function' ? streams.pop() : function(error) { if (error) throw error; }, options = streams.length && streams[streams.length - 1] && typeof streams[streams.length - 1].on !== 'function' && (streams[streams.length - 1].signal !== undefined || streams[streams.length - 1].end !== undefined) ? streams.pop() : {}; if (streams.length === 1 && Array.isArray(streams[0])) streams = streams[0]; var functional = streams.some(function(item) { return typeof item === 'function'; }) || !streams[0] || typeof streams[0].on !== 'function'; if (functional) { if (!options.signal) { var controller = new AbortController(); options = Object.assign({}, options, { signal: controller.signal, controller: controller }); } runFunctionalPipeline(streams, options).then(function(value) { callback(null, value); }, callback); return streams[streams.length - 1]; } var settled = false, listeners = [], abort, lastIndex = streams.length - 1; function listen(stream, event, listener) { stream.once(event, listener); listeners.push({ stream: stream, event: event, listener: listener }); } function cleanup() { listeners.forEach(function(entry) { entry.stream.removeListener(entry.event, entry.listener); }); if (options.signal && abort) options.signal.removeEventListener('abort', abort); } function finish(error) { if (settled) return; settled = true; cleanup(); if (error) { protectPipelineErrors(streams); streams.forEach(function(item) { if (item.destroy) item.destroy(error); }); } callback(error || null); } streams.forEach(function(item, index) { listen(item, 'error', function(error) { finish(error); }); listen(item, 'close', function() { var readableIncomplete = index < lastIndex && item.readableEnded !== true, writableIncomplete = index > 0 && item.writableFinished !== true; if (readableIncomplete || writableIncomplete) { var error = new Error('Premature close'); error.code = 'ERR_STREAM_PREMATURE_CLOSE'; finish(error); } }); }); for (var index = 0; index < lastIndex; index++) streams[index].pipe(streams[index + 1]); var last = streams[lastIndex]; listen(last, 'finish', function() { finish(); }); abort = function() { finish(pipelineAbortError(options.signal)); }; if (options.signal) { if (options.signal.aborted) abort(); else options.signal.addEventListener('abort', abort, { once: true }); } return last; }\
             Readable.toWeb = function(source) { var controller, onData, onEnd, onError; function cleanup() { source.removeListener('data', onData); source.removeListener('end', onEnd); source.removeListener('error', onError); source.removeListener('close', cleanup); } return new globalThis.ReadableStream({ start: function(value) { controller = value; onData = function(chunk) { controller.enqueue(chunk); if (controller.desiredSize <= 0) source.pause(); }; onEnd = function() { cleanup(); controller.close(); }; onError = function(error) { controller.error(error); }; source.on('data', onData); source.once('end', onEnd); source.once('error', onError); source.once('close', cleanup); source.pause(); }, pull: function() { if (!source.destroyed && (source.readableLength > 0 || !source.readableEnded)) source.resume(); }, cancel: function(reason) { return new Promise(function(resolve) { if (source.closed) { cleanup(); resolve(); return; } source.once('close', function() { cleanup(); resolve(); }); source.destroy(reason); }); } }); }; Readable.fromWeb = function(source, options) { options = options || {}; var reader = source.getReader(), output = new Readable(options), reading = false, done = false; function pump() { if (reading || done) return; reading = true; reader.read().then(function(result) { reading = false; if (result.done) { done = true; output.push(null); } else if (output.push(result.value)) pump(); }, function(error) { reading = false; done = true; output.destroy(error); }); } output._read = pump; output._destroy = function(error, callback) { if (done) { callback(error); return; } done = true; Promise.resolve(reader.cancel(error)).then(function() { callback(error); }, callback); }; queueMicrotask(pump); return output; }; Writable.toWeb = function(destination) { return new globalThis.WritableStream({ write: function(chunk) { return new Promise(function(resolve, reject) { try { destination.write(chunk, function(error) { if (error) reject(error); else resolve(); }); } catch (error) { reject(error); } }); }, close: function() { return new Promise(function(resolve, reject) { try { destination.end(function(error) { if (error) reject(error); else resolve(); }); } catch (error) { reject(error); } }); }, abort: function(reason) { destination.destroy(reason); } }); }; Writable.fromWeb = function(source, options) { var writer = source.getWriter(), done = false, output = new Writable(Object.assign({}, options || {}, { write: function(chunk, encoding, callback) { writer.write(chunk).then(function() { callback(); }, callback); }, final: function(callback) { writer.close().then(function() { done = true; callback(); }, callback); } })); output._destroy = function(error, callback) { if (done) { callback(error); return; } done = true; Promise.resolve(writer.abort(error)).then(function() { callback(error); }, callback); }; return output; }; Duplex.toWeb = function(source) { return { readable: Readable.toWeb(source), writable: Writable.toWeb(source) }; }; Duplex.fromWeb = function(pair, options) { return duplexPair(Writable.fromWeb(pair.writable, options), Readable.fromWeb(pair.readable, options)); }; function addAbortSignal(signal, stream) { if (!signal || typeof signal.aborted !== 'boolean' || typeof signal.addEventListener !== 'function') { var signalError = new TypeError('The signal argument must be an instance of AbortSignal'); signalError.code = 'ERR_INVALID_ARG_TYPE'; throw signalError; } var nodeStream = stream && typeof stream.destroy === 'function', webReadable = stream && typeof stream.cancel === 'function', webWritable = stream && typeof stream.abort === 'function'; if (!nodeStream && !webReadable && !webWritable) { var streamError = new TypeError('The stream argument must be a Stream'); streamError.code = 'ERR_INVALID_ARG_TYPE'; throw streamError; } var settled = false; function cleanup() { signal.removeEventListener('abort', abort); if (nodeStream) stream.removeListener('close', cleanup); } function abort() { if (settled) return; settled = true; cleanup(); var error = pipelineAbortError(signal); if (nodeStream) stream.destroy(error); else if (webReadable && stream._controller && typeof stream._controller.error === 'function') stream._controller.error(error); else if (webWritable && stream._state !== undefined && typeof stream._rejectClosed === 'function') { stream._state = 'errored'; stream._error = error; stream._rejectClosed(error); } else Promise.resolve(webReadable ? stream.cancel(error) : stream.abort(error)).catch(function() {}); } if (signal.aborted) abort(); else { signal.addEventListener('abort', abort, { once: true }); if (nodeStream) stream.once('close', cleanup); } return stream; } function isDestroyed(stream) { return stream === null || stream === undefined ? false : Boolean(stream.destroyed); } function isDisturbed(stream) { return stream === null || stream === undefined ? false : Boolean(stream._readableDidRead || stream.readableEnded); } function isErrored(stream) { return stream === null || stream === undefined ? false : stream.errored !== null && stream.errored !== undefined; } function isReadable(stream) { return Boolean(stream && stream.readable === true && !stream.destroyed && !stream.readableEnded); } function isWritable(stream) { return Boolean(stream && stream.writable === true && !stream.destroyed && !stream.writableEnded); } Readable.isDisturbed = isDisturbed; Stream.isDestroyed = isDestroyed; Stream.isDisturbed = isDisturbed; Stream.isErrored = isErrored; Stream.isReadable = isReadable; Stream.isWritable = isWritable;\n\
             function duplexPair(writable, readable, stages) { stages = stages || [writable, readable]; var output = new Duplex({ readableObjectMode: Boolean(readable._readableObjectMode), writableObjectMode: Boolean(writable._writableObjectMode), read: function() { if (readable.resume) readable.resume(); }, write: function(chunk, encoding, callback) { try { writable.write(chunk, encoding, callback); } catch (error) { callback(error); } }, final: function(callback) { try { writable.end(callback); } catch (error) { callback(error); } } }); if (readable.pause) readable.pause(); readable.on('data', function(chunk) { if (!output.push(chunk) && readable.pause) readable.pause(); }); readable.once('end', function() { output.push(null); }); stages.forEach(function(stage) { var needsReadable = stage.readable === true, needsWritable = stage.writable === true; stage.once('error', function(error) { if (!output.destroyed) output.destroy(error); }); stage.once('close', function() { if (output.destroyed) return; if (stage.errored) { output.destroy(stage.errored); return; } if (output._compositionEnding) return; if ((needsReadable && stage.readableEnded !== true) || (needsWritable && stage.writableFinished !== true)) { var error = new Error('Premature close'); error.code = 'ERR_STREAM_PREMATURE_CLOSE'; output.destroy(error); } }); }); var destroyPair = output.destroy; output.destroy = function(error) { stages.forEach(function(stage) { if (stage !== output && stage.destroy && !stage.destroyed) stage.destroy(error); }); return destroyPair.call(output, error); }; if (readable.resume) readable.resume(); return output; } function compose() { var stages = Array.prototype.slice.call(arguments); if (!stages.length) throw new TypeError('compose requires at least one stream'); if (stages.length === 1) return Duplex.from(stages[0]); for (var index = 0; index + 1 < stages.length; index++) { if (!stages[index] || typeof stages[index].pipe !== 'function') throw new TypeError('compose stages must be readable streams'); stages[index].pipe(stages[index + 1]); } return duplexPair(stages[0], stages[stages.length - 1], stages); } Duplex.from = function(source) { if (source instanceof Duplex) return source; if (typeof source === 'function') { var input = new PassThrough({ objectMode: true }), produced, controller = new AbortController(); try { produced = source(input, { signal: controller.signal }); } catch (error) { input.destroy(error); produced = Promise.reject(error); } var sink = Boolean(produced && typeof produced.then === 'function'); if (sink) produced = Promise.resolve(produced).then(function(value) { if (value !== undefined && value !== null) { var invalid = new TypeError('Expected a null or undefined return value from the body function'); invalid.code = 'ERR_INVALID_RETURN_VALUE'; throw invalid; } }); var converted = Duplex.from(produced), functional = duplexPair(input, converted, [input, converted]), destroyFunctional = functional.destroy; if (sink) { functional.readable = false; functional._final = function(callback) { try { input.end(); } catch (error) { callback(error); return; } produced.then(function() { callback(); }, callback); }; } functional.destroy = function(error) { if (!controller.signal.aborted) controller.abort(error); return destroyFunctional.call(functional, error); }; return functional; } if (source && source.readable && source.writable === undefined) { var readableOnly = duplexPair(new Writable({ write: function(chunk, encoding, callback) { var error = new Error('stream is not writable'); error.code = 'ERR_STREAM_CANNOT_PIPE'; callback(error); } }), source, [source]); readableOnly.writable = false; return readableOnly; } if (source && source.writable && source.readable === undefined) { var empty = new Readable(); empty.push(null); var writableOnly = duplexPair(source, empty, [source, empty]); writableOnly.readable = false; return writableOnly; } if (source && source.readable && source.writable) return duplexPair(source.writable, source.readable, [source.writable, source.readable]); var output = new Duplex({ objectMode: true, read: function() {} }), sourceIterator = null, iteratorClosed = false, destroySource = output.destroy; output.writable = false; output.destroy = function(error) { if (!iteratorClosed && sourceIterator && typeof sourceIterator.return === 'function') { iteratorClosed = true; try { Promise.resolve(sourceIterator.return()).catch(function() {}); } catch (_) {} } return destroySource.call(output, error); }; queueMicrotask(async function() { try { if (source && typeof source.then === 'function') { var value = await source; if (value !== undefined && value !== null) output.push(value); } else if (typeof source === 'string' || globalThis.Buffer.isBuffer(source)) { output.push(source); } else { sourceIterator = typeof source[Symbol.asyncIterator] === 'function' ? source[Symbol.asyncIterator]() : source[Symbol.iterator](); while (!output.destroyed) { var step = await sourceIterator.next(); if (step.done || output.destroyed) break; var value = await step.value; if (value === null) { var nullError = new TypeError('May not write null values to stream'); nullError.code = 'ERR_STREAM_NULL_VALUES'; throw nullError; } output.push(value); } } if (!output.destroyed) output.push(null); } catch (error) { output.destroy(error); } }); return output; }; function createDuplexPair(options) { options = options || {}; var left, right; left = new Duplex(Object.assign({}, options, { read: function() {}, write: function(chunk, encoding, callback) { if (right.destroyed) { var error = new Error('Cannot call write after a stream was destroyed'); error.code = 'ERR_STREAM_DESTROYED'; callback(error); } else { right.push(chunk); callback(); } }, final: function(callback) { if (!right.destroyed) right.push(null); callback(); } })); right = new Duplex(Object.assign({}, options, { read: function() {}, write: function(chunk, encoding, callback) { if (left.destroyed) { var error = new Error('Cannot call write after a stream was destroyed'); error.code = 'ERR_STREAM_DESTROYED'; callback(error); } else { left.push(chunk); callback(); } }, final: function(callback) { if (!left.destroyed) left.push(null); callback(); } })); return [left, right]; } function destroyStream(stream, error) { if (!error) { error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; } if (stream && typeof stream.destroy === 'function') stream.destroy(error); } function isArrayBufferView(value) { return globalThis.ArrayBuffer.isView(value); } function isUint8Array(value) { return value instanceof globalThis.Uint8Array; } function uint8ArrayToBuffer(value) { if (!isUint8Array(value)) throw new TypeError('value must be a Uint8Array'); return globalThis.Buffer.from(value.buffer, value.byteOffset, value.byteLength); } var streamPromises = { pipeline: function() { var stages = Array.prototype.slice.call(arguments); return new Promise(function(resolve, reject) { stages.push(function(error, value) { if (error) reject(error); else resolve(value); }); pipeline.apply(null, stages); }); }, finished: function(stream, options) { return new Promise(function(resolve, reject) { finished(stream, Object.assign({}, options || {}, { cleanup: true }), function(error) { if (error) reject(error); else resolve(); }); }); } }; module.exports = Stream; Object.assign(module.exports, { Stream: Stream, Readable: Readable, Writable: Writable, Duplex: Duplex, Transform: Transform, PassThrough: PassThrough, pipeline: pipeline, compose: compose, finished: finished, addAbortSignal: addAbortSignal, isDestroyed: isDestroyed, isDisturbed: isDisturbed, isErrored: isErrored, isReadable: isReadable, isWritable: isWritable, getDefaultHighWaterMark: getDefaultHighWaterMark, setDefaultHighWaterMark: setDefaultHighWaterMark, duplexPair: createDuplexPair, destroy: destroyStream, _isArrayBufferView: isArrayBufferView, _isUint8Array: isUint8Array, _uint8ArrayToBuffer: uint8ArrayToBuffer, promises: streamPromises });\n\
             function writableComposition(stages) { var first = stages[0], last = stages[stages.length - 1], finalCallback = null, finalSettled = false, lastClosed = false; function settle(error) { if (finalSettled || !finalCallback) return; finalSettled = true; var callback = finalCallback; finalCallback = null; callback(error); } var output = new Duplex({ objectMode: true, read: function() {}, write: function(chunk, encoding, callback) { try { first.write(chunk, encoding, callback); } catch (error) { callback(error); } }, final: function(callback) { finalCallback = callback; if (lastClosed) { settle(last.errored); return; } stages.forEach(function(stage) { stage._compositionEnding = true; }); try { first.end(); } catch (error) { settle(error); } } }); output.readable = false; output.readableEnded = true; output._endEmitted = true; stages.forEach(function(stage) { var needsReadable = stage.readable === true, needsWritable = stage.writable === true; stage.once('error', function(error) { settle(error); if (!output.destroyed) output.destroy(error); }); stage.once('close', function() { if (stage === last) { lastClosed = true; settle(stage.errored); } if (output.destroyed || output.writableEnded || output._ending) return; if ((needsReadable && stage.readableEnded !== true) || (needsWritable && stage.writableFinished !== true)) { var error = new Error('Premature close'); error.code = 'ERR_STREAM_PREMATURE_CLOSE'; settle(error); output.destroy(error); } }); }); for (var index = 0; index + 1 < stages.length; index++) stages[index].pipe(stages[index + 1]); var destroyOutput = output.destroy; output.destroy = function(error) { stages.forEach(function(stage) { if (stage.destroy && !stage.destroyed) stage.destroy(error); }); return destroyOutput.call(output, error); }; return output; }\n\
             var duplexFromImplementation = Duplex.from; Duplex.from = function(source) { var valid = source instanceof Duplex || typeof source === 'function' || typeof source === 'string' || globalThis.ArrayBuffer.isView(source) || Boolean(source && (typeof source.then === 'function' || typeof source[Symbol.iterator] === 'function' || typeof source[Symbol.asyncIterator] === 'function' || source.readable === true || source.writable === true || (source.readable && source.writable))); if (!valid) { var invalid = new TypeError('The body argument must be a supported stream source'); invalid.code = 'ERR_INVALID_ARG_TYPE'; throw invalid; } return duplexFromImplementation.call(this, source); };\n\
             module.exports.default = Stream; module.exports.__esModule = true;\n",
        ),
        "_stream_readable" => Some("module.exports = require('node:stream').Readable;\n"),
        "_stream_writable" => Some("module.exports = require('node:stream').Writable;\n"),
        "_stream_duplex" => Some("module.exports = require('node:stream').Duplex;\n"),
        "_stream_transform" => Some("module.exports = require('node:stream').Transform;\n"),
        "_stream_passthrough" => Some("module.exports = require('node:stream').PassThrough;\n"),
        "_stream_wrap" => Some("module.exports = require('node:stream').Duplex;\n"),
        "stream/promises" => Some(
            "var callbackStream = require('node:stream'); function finished(stream, options) { return new Promise(function(resolve, reject) { callbackStream.finished(stream, Object.assign({}, options || {}, { cleanup: true }), function(error) { if (error) reject(error); else resolve(); }); }); }\
            function pipeline() { var stages = Array.prototype.slice.call(arguments); return new Promise(function(resolve, reject) { stages.push(function(error, value) { if (error) reject(error); else resolve(value); }); callbackStream.pipeline.apply(callbackStream, stages); }); }\
             module.exports = { pipeline: pipeline, finished: finished }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "stream/consumers" => Some(
            "function collect(stream) { if (stream && typeof stream[Symbol.asyncIterator] === 'function') return (async function() { var chunks = []; for await (var chunk of stream) chunks.push(Buffer.from(chunk)); return Buffer.concat(chunks); })(); return new Promise(function(resolve, reject) { var chunks = []; function cleanup() { if (stream.off) { stream.off('data', data); stream.off('end', end); stream.off('error', reject); } } function data(chunk) { chunks.push(Buffer.from(chunk)); } function end() { cleanup(); resolve(Buffer.concat(chunks)); } if (!stream || typeof stream.on !== 'function') { reject(new TypeError('stream must be readable')); return; } stream.on('data', data); stream.once('end', end); stream.once('error', reject); }); } function buffer(stream) { return collect(stream); } function text(stream) { return collect(stream).then(function(value) { return value.toString('utf8'); }); } function json(stream) { return text(stream).then(JSON.parse); } function arrayBuffer(stream) { return collect(stream).then(function(value) { var copy = Uint8Array.from(value); return copy.buffer; }); } function blob(stream) { return collect(stream).then(function(value) { return new Blob([value]); }); } module.exports = { arrayBuffer: arrayBuffer, blob: blob, buffer: buffer, json: json, text: text }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "stream/web" => Some(
            "module.exports = { ReadableStream: globalThis.ReadableStream, ReadableStreamDefaultReader: globalThis.ReadableStreamDefaultReader, ReadableStreamDefaultController: globalThis.ReadableStreamDefaultController, ReadableByteStreamController: globalThis.ReadableByteStreamController, ReadableStreamBYOBReader: globalThis.ReadableStreamBYOBReader, ReadableStreamBYOBRequest: globalThis.ReadableStreamBYOBRequest, WritableStream: globalThis.WritableStream, WritableStreamDefaultWriter: globalThis.WritableStreamDefaultWriter, WritableStreamDefaultController: globalThis.WritableStreamDefaultController, TransformStream: globalThis.TransformStream, TransformStreamDefaultController: globalThis.TransformStreamDefaultController, ByteLengthQueuingStrategy: globalThis.ByteLengthQueuingStrategy, CountQueuingStrategy: globalThis.CountQueuingStrategy, TextEncoderStream: globalThis.TextEncoderStream, TextDecoderStream: globalThis.TextDecoderStream, CompressionStream: globalThis.CompressionStream, DecompressionStream: globalThis.DecompressionStream }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "diagnostics_channel" => Some(
            "var registry = globalThis.__thawDiagnosticChannels || (globalThis.__thawDiagnosticChannels = new Map());\n\
             function Channel(name) { this.name = String(name); this._subscribers = []; this._stores = []; }\n\
             Object.defineProperty(Channel.prototype, 'hasSubscribers', { get: function() { return this._subscribers.length !== 0; } });\n\
             Channel.prototype.subscribe = function(callback) { if (typeof callback !== 'function') throw new TypeError('callback must be a function'); if (this._subscribers.indexOf(callback) < 0) this._subscribers.push(callback); };\n\
             Channel.prototype.unsubscribe = function(callback) { var index = this._subscribers.indexOf(callback); if (index < 0) return false; this._subscribers.splice(index, 1); return true; };\n\
             Channel.prototype.bindStore = function(store, transform) { if (!store || typeof store.run !== 'function') throw new TypeError('store must provide run'); this._stores.push({ store: store, transform: typeof transform === 'function' ? transform : function(value) { return value; } }); };\n\
             Channel.prototype.unbindStore = function(store) { var before = this._stores.length; this._stores = this._stores.filter(function(binding) { return binding.store !== store; }); return before !== this._stores.length; };\n\
             Channel.prototype.runStores = function(data, callback, thisArg) { var args = Array.prototype.slice.call(arguments, 3); var bindings = this._stores.slice(); function invoke(index) { if (index === bindings.length) return callback.apply(thisArg, args); var binding = bindings[index]; return binding.store.run(binding.transform(data), function() { return invoke(index + 1); }); } return invoke(0); };\n\
             Channel.prototype.publish = function(data) { var self = this; return this.runStores(data, function() { self._subscribers.slice().forEach(function(callback) { callback(data, self.name); }); }); };\n\
             function channel(name) { var key = String(name); if (!registry.has(key)) registry.set(key, new Channel(key)); return registry.get(key); }\n\
             function hasSubscribers(name) { return channel(name).hasSubscribers; }\n\
             function subscribe(name, callback) { channel(name).subscribe(callback); }\n\
             function unsubscribe(name, callback) { return channel(name).unsubscribe(callback); }\n\
             function tracingChannel(nameOrChannels) {\n\
             \x20\x20var channels = typeof nameOrChannels === 'string' ? { start: channel('tracing:' + nameOrChannels + ':start'), end: channel('tracing:' + nameOrChannels + ':end'), asyncStart: channel('tracing:' + nameOrChannels + ':asyncStart'), asyncEnd: channel('tracing:' + nameOrChannels + ':asyncEnd'), error: channel('tracing:' + nameOrChannels + ':error') } : nameOrChannels;\n\
             \x20\x20return { start: channels.start, end: channels.end, asyncStart: channels.asyncStart, asyncEnd: channels.asyncEnd, error: channels.error, traceSync: function(callback, context, thisArg) { var args = Array.prototype.slice.call(arguments, 3); context = context || {}; channels.start.publish(context); try { var result = channels.start.runStores(context, callback, thisArg, ...args); context.result = result; channels.end.publish(context); return result; } catch (error) { context.error = error; channels.error.publish(context); channels.end.publish(context); throw error; } }, tracePromise: function(callback, context, thisArg) { var args = Array.prototype.slice.call(arguments, 3); context = context || {}; channels.start.publish(context); return Promise.resolve().then(function() { return channels.start.runStores(context, callback, thisArg, ...args); }).then(function(result) { context.result = result; channels.asyncStart.publish(context); channels.asyncEnd.publish(context); channels.end.publish(context); return result; }, function(error) { context.error = error; channels.error.publish(context); channels.asyncStart.publish(context); channels.asyncEnd.publish(context); channels.end.publish(context); throw error; }); }, traceCallback: function(callback, position, context, thisArg) { var args = Array.prototype.slice.call(arguments, 4); context = context || {}; channels.start.publish(context); var original = args[position]; args[position] = function(error, result) { if (error) { context.error = error; channels.error.publish(context); } else context.result = result; channels.asyncStart.publish(context); try { return original.apply(this, arguments); } finally { channels.asyncEnd.publish(context); channels.end.publish(context); } }; return channels.start.runStores(context, callback, thisArg, ...args); } };\n\
             }\n\
             module.exports = { channel: channel, hasSubscribers: hasSubscribers, subscribe: subscribe, unsubscribe: unsubscribe, tracingChannel: tracingChannel, Channel: Channel }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "trace_events" => Some(
            r#"var categoryCounts = new Map();
             function normalizeCategories(categories) { if (!Array.isArray(categories) || !categories.length) throw new TypeError('options.categories must be a non-empty array'); return Array.from(new Set(categories.map(String))); }
             function Tracing(options) { if (!(this instanceof Tracing)) return new Tracing(options); if (!options || typeof options !== 'object') throw new TypeError('options must be an object'); this.categories = normalizeCategories(options.categories); this.enabled = false; }
             Tracing.prototype.enable = function() { if (this.enabled) return; this.enabled = true; this.categories.forEach(function(category) { categoryCounts.set(category, (categoryCounts.get(category) || 0) + 1); }); };
             Tracing.prototype.disable = function() { if (!this.enabled) return; this.enabled = false; this.categories.forEach(function(category) { var count = (categoryCounts.get(category) || 0) - 1; if (count > 0) categoryCounts.set(category, count); else categoryCounts.delete(category); }); };
             function createTracing(options) { return new Tracing(options); }
             function getEnabledCategories() { return Array.from(categoryCounts.keys()).sort().join(','); }
             module.exports = { Tracing: Tracing, createTracing: createTracing, getEnabledCategories: getEnabledCategories }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "inspector" => Some(
            r#"var EventEmitter = require('node:events'), opened = false, inspectorUrl, isolateId = 'thaw-quickjs-main';
             function remoteObject(value, returnByValue) { var type = value === null ? 'object' : typeof value, result = { type: type }; if (value === null) result.subtype = 'null'; else if (type === 'undefined') result.description = 'undefined'; else if (type === 'number' && !Number.isFinite(value)) { result.unserializableValue = String(value); result.description = String(value); } else if (type === 'bigint') { result.unserializableValue = String(value) + 'n'; result.description = String(value) + 'n'; } else if (type === 'symbol' || type === 'function') result.description = String(value); else if (type === 'object' && !returnByValue) { result.className = value.constructor && value.constructor.name || 'Object'; result.description = result.className; } else result.value = value; return result; }
             function Session() { if (!(this instanceof Session)) return new Session(); EventEmitter.call(this); this._connected = false; }
             Session.prototype = Object.create(EventEmitter.prototype); Session.prototype.constructor = Session;
             Session.prototype.connect = function() { if (this._connected) { var error = new Error('The inspector session is already connected'); error.code = 'ERR_INSPECTOR_ALREADY_CONNECTED'; throw error; } this._connected = true; };
             Session.prototype.connectToMainThread = Session.prototype.connect;
             Session.prototype.disconnect = function() { this._connected = false; this.removeAllListeners(); };
             Session.prototype.post = function(method, params, callback) { if (typeof params === 'function') { callback = params; params = {}; } params = params || {}; if (typeof callback !== 'function') callback = function(error) { if (error) throw error; }; var session = this; queueMicrotask(function() { if (!session._connected) { var disconnected = new Error('Session is not connected'); disconnected.code = 'ERR_INSPECTOR_NOT_CONNECTED'; callback(disconnected); return; } var result = {}; try { if (method === 'Runtime.evaluate') { try { var value = (0, eval)(String(params.expression || '')); result.result = remoteObject(value, Boolean(params.returnByValue)); } catch (error) { result.result = remoteObject(error, false); result.exceptionDetails = { text: error.message, exception: remoteObject(error, false), lineNumber: 0, columnNumber: 0 }; } } else if (method === 'Runtime.getIsolateId') result = { id: isolateId }; else if (method === 'Schema.getDomains') result = { domains: ['Runtime','Debugger','Profiler','HeapProfiler','Schema'].map(function(name) { return { name: name, version: '1.3' }; }) }; else if (/^(Runtime|Debugger|Profiler|HeapProfiler)\.(enable|disable)$/.test(String(method))) result = {}; else { var unsupported = new Error('Inspector protocol method is not supported: ' + method); unsupported.code = 'ERR_INSPECTOR_COMMAND'; throw unsupported; } callback(null, result); } catch (error) { callback(error); } }); };
             function open(port, host, wait) { port = port === undefined ? 9229 : Number(port); host = host === undefined ? '127.0.0.1' : String(host); opened = true; inspectorUrl = 'ws://' + host + ':' + port + '/thaw'; return { dispose: close }; }
             function close() { opened = false; inspectorUrl = undefined; }
             function url() { return opened ? inspectorUrl : undefined; }
             function waitForDebugger() { if (!opened) { var error = new Error('Inspector is not active'); error.code = 'ERR_INSPECTOR_NOT_ACTIVE'; throw error; } }
             module.exports = { Session: Session, open: open, close: close, url: url, waitForDebugger: waitForDebugger, console: globalThis.console }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "inspector/promises" => Some(
            r#"var inspector = require('node:inspector');
             function Session() { inspector.Session.call(this); }
             Session.prototype = Object.create(inspector.Session.prototype); Session.prototype.constructor = Session;
             Session.prototype.post = function(method, params) { var session = this; return new Promise(function(resolve, reject) { inspector.Session.prototype.post.call(session, method, params || {}, function(error, result) { if (error) reject(error); else resolve(result); }); }); };
             module.exports = { Session: Session, open: inspector.open, close: inspector.close, url: inspector.url, waitForDebugger: inspector.waitForDebugger, console: inspector.console }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "repl" => Some(
            r#"var EventEmitter = require('node:events'), inspect = require('node:util').inspect;
             var REPL_MODE_SLOPPY = Symbol('repl-sloppy'), REPL_MODE_STRICT = Symbol('repl-strict');
             function Recoverable(error) { if (!(this instanceof Recoverable)) return new Recoverable(error); SyntaxError.call(this, error && error.message || String(error)); this.name = 'Recoverable'; this.err = error; this.message = error && error.message || String(error); }
             Recoverable.prototype = Object.create(SyntaxError.prototype); Recoverable.prototype.constructor = Recoverable;
             function writer(value) { return inspect(value); }
             function defaultEval(code, context, filename, callback) { try { var source = String(code), strict = context && context.__thawReplMode === REPL_MODE_STRICT; var result = (0, eval)(strict ? '\"use strict\";\n' + source : source); callback(null, result); } catch (error) { if (error instanceof SyntaxError && /unexpected end|unterminated|expecting/i.test(String(error.message))) callback(new Recoverable(error)); else callback(error); } }
             function writeOutput(output, value) { if (output && typeof output.write === 'function') output.write(String(value)); }
             function REPLServer(options) { if (!(this instanceof REPLServer)) return new REPLServer(options); EventEmitter.call(this); if (typeof options === 'string') options = { prompt: options }; options = options || {}; this.input = options.input || options.stdin; this.output = options.output || options.stdout; this.prompt = options.prompt === undefined ? '> ' : String(options.prompt); this.eval = typeof options.eval === 'function' ? options.eval : defaultEval; this.writer = typeof options.writer === 'function' ? options.writer : writer; this.context = options.useGlobal ? globalThis : Object.create(globalThis); this.context.global = this.context; this.context.globalThis = this.context; this.context.console = globalThis.console; this.context.__thawReplMode = options.replMode || REPL_MODE_SLOPPY; this.commands = Object.create(null); this.lines = []; this.history = []; this.historySize = options.historySize === undefined ? 30 : Math.max(0, Number(options.historySize)); this._bufferedCommand = ''; this.closed = false; this.paused = false; this.last = undefined; this.defineCommand('break', { help: 'Sometimes you get stuck, this gets you out', action: function() { this.clearBufferedCommand(); this.displayPrompt(); } }); this.defineCommand('clear', { help: 'Alias for .break', action: function() { this.clearBufferedCommand(); this.displayPrompt(); } }); this.defineCommand('exit', { help: 'Exit the REPL', action: function() { this.close(); } }); this.defineCommand('help', { help: 'Print this help message', action: function() { var self = this; Object.keys(this.commands).sort().forEach(function(name) { writeOutput(self.output, '.' + name + '\t' + self.commands[name].help + '\n'); }); this.displayPrompt(); } }); }
             REPLServer.prototype = Object.create(EventEmitter.prototype); REPLServer.prototype.constructor = REPLServer;
             REPLServer.prototype.setPrompt = function(prompt) { this.prompt = String(prompt); };
             REPLServer.prototype.displayPrompt = function(preserveCursor) { if (!this.closed) writeOutput(this.output, this.prompt); return this; };
             REPLServer.prototype.clearBufferedCommand = function() { this._bufferedCommand = ''; };
             REPLServer.prototype.defineCommand = function(keyword, command) { keyword = String(keyword).replace(/^\./, ''); if (!keyword) throw new TypeError('keyword must not be empty'); if (typeof command === 'function') command = { action: command }; if (!command || typeof command.action !== 'function') throw new TypeError('command action must be a function'); this.commands[keyword] = { help: String(command.help || ''), action: command.action }; };
             REPLServer.prototype.setupHistory = function(path, callback) { this.historyPath = String(path); if (typeof callback === 'function') queueMicrotask(function() { callback(null); }); };
             REPLServer.prototype.pause = function() { this.paused = true; if (this.input && this.input.pause) this.input.pause(); return this; };
             REPLServer.prototype.resume = function() { this.paused = false; if (this.input && this.input.resume) this.input.resume(); return this; };
             REPLServer.prototype.close = function() { if (this.closed) return; this.closed = true; this.emit('exit'); this.emit('close'); };
             REPLServer.prototype.write = function(command) { if (this.closed) return false; var line = String(command), trimmed = line.trim(), self = this; this.lines.push(line); if (trimmed.charAt(0) === '.') { var space = trimmed.indexOf(' '), name = trimmed.slice(1, space < 0 ? undefined : space), value = space < 0 ? '' : trimmed.slice(space + 1), entry = this.commands[name]; if (!entry) { writeOutput(this.output, 'Invalid REPL keyword\n'); this.displayPrompt(); return true; } entry.action.call(this, value); return true; } if (trimmed) { this.history.unshift(line); if (this.history.length > this.historySize) this.history.length = this.historySize; } var source = this._bufferedCommand + line; this.eval(source, this.context, 'repl', function(error, result) { if (error instanceof Recoverable) { self._bufferedCommand = source + '\n'; self.displayPrompt(); return; } self._bufferedCommand = ''; if (error) { self.emit('error', error); writeOutput(self.output, String(error) + '\n'); } else { self.last = result; self.context._ = result; writeOutput(self.output, self.writer(result) + '\n'); self.emit('result', result); } self.displayPrompt(); }); return true; };
             function start(options) { var server = new REPLServer(options); server.displayPrompt(); return server; }
             module.exports = { start: start, REPLServer: REPLServer, Recoverable: Recoverable, REPL_MODE_SLOPPY: REPL_MODE_SLOPPY, REPL_MODE_STRICT: REPL_MODE_STRICT, writer: writer, _builtinLibs: [] }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "cluster" => Some(
            r#"var EventEmitter = require('node:events'), processObject = require('node:process'), cluster = new EventEmitter();
             var nextWorkerId = 1, SCHED_NONE = 1, SCHED_RR = 2, schedulingPolicy = SCHED_RR, workers = Object.create(null), settings = {}, isPrimary = processObject.env.NODE_UNIQUE_ID === undefined;
             function Worker(options) { if (!(this instanceof Worker)) return new Worker(options); EventEmitter.call(this); options = options || {}; this.id = options.id === undefined ? nextWorkerId++ : Number(options.id); this.process = options.process || { pid: Number(processObject.pid || 0) + this.id, connected: true, killed: false, kill: function() { this.killed = true; this.connected = false; return true; } }; this.exitedAfterDisconnect = undefined; this.state = 'none'; }
             Worker.prototype = Object.create(EventEmitter.prototype); Worker.prototype.constructor = Worker;
             Worker.prototype.isConnected = function() { return Boolean(this.process && this.process.connected) && this.state !== 'disconnected' && this.state !== 'dead'; };
             Worker.prototype.isDead = function() { return this.state === 'dead' || Boolean(this.process && this.process.killed); };
             Worker.prototype.send = function(message, sendHandle, options, callback) { if (typeof sendHandle === 'function') callback = sendHandle; else if (typeof options === 'function') callback = options; if (!this.isConnected()) { var error = new Error('Channel closed'); error.code = 'ERR_IPC_CHANNEL_CLOSED'; if (typeof callback === 'function') queueMicrotask(function() { callback(error); }); return false; } var worker = this; queueMicrotask(function() { worker.emit('message', message, sendHandle); cluster.emit('message', worker, message, sendHandle); if (typeof callback === 'function') callback(null); }); return true; };
             Worker.prototype.disconnect = function() { if (!this.isConnected()) return this; this.exitedAfterDisconnect = true; this.state = 'disconnected'; if (this.process) this.process.connected = false; var worker = this; queueMicrotask(function() { worker.emit('disconnect'); cluster.emit('disconnect', worker); }); return this; };
             Worker.prototype.kill = Worker.prototype.destroy = function(signal) { if (this.isDead()) return this; var worker = this; this.exitedAfterDisconnect = this.exitedAfterDisconnect === true; this.state = 'dead'; if (this.process) { this.process.connected = false; this.process.killed = true; } delete workers[this.id]; queueMicrotask(function() { worker.emit('exit', 0, signal || 'SIGTERM'); cluster.emit('exit', worker, 0, signal || 'SIGTERM'); }); return this; };
             function setupPrimary(options) { if (!isPrimary) return settings; options = options || {}; settings = Object.assign({}, settings, options); cluster.settings = settings; queueMicrotask(function() { cluster.emit('setup', settings); }); return settings; }
             function fork(environment) { if (!isPrimary) { var error = new Error('cluster.fork() may only be called from a primary process'); error.code = 'ERR_CLUSTER_ONLY_PRIMARY'; throw error; } var worker = new Worker(); worker.state = 'online'; worker.environment = Object.assign({}, processObject.env, environment || {}, { NODE_UNIQUE_ID: String(worker.id) }); workers[worker.id] = worker; queueMicrotask(function() { cluster.emit('fork', worker); worker.emit('online'); cluster.emit('online', worker); }); return worker; }
             function disconnect(callback) { var active = Object.keys(workers).map(function(id) { return workers[id]; }); if (!active.length) { if (typeof callback === 'function') queueMicrotask(callback); return; } var remaining = active.length; active.forEach(function(worker) { worker.once('disconnect', function() { if (--remaining === 0 && typeof callback === 'function') queueMicrotask(callback); }); worker.disconnect(); }); }
             Object.assign(cluster, { Worker: Worker, workers: workers, settings: settings, isPrimary: isPrimary, isMaster: isPrimary, isWorker: !isPrimary, worker: isPrimary ? undefined : new Worker({ id: Number(processObject.env.NODE_UNIQUE_ID) }), SCHED_NONE: SCHED_NONE, SCHED_RR: SCHED_RR, setupPrimary: setupPrimary, setupMaster: setupPrimary, fork: fork, disconnect: disconnect });
             Object.defineProperty(cluster, 'schedulingPolicy', { enumerable: true, get: function() { return schedulingPolicy; }, set: function(value) { value = Number(value); if (value !== SCHED_NONE && value !== SCHED_RR) throw new RangeError('invalid scheduling policy'); schedulingPolicy = value; } });
             module.exports = cluster; module.exports.default = cluster; module.exports.__esModule = true;
"#,
        ),
        "_http_agent" => Some(
            "var http = require('node:http'); module.exports = { Agent: http.Agent, globalAgent: http.globalAgent }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_http_client" => Some(
            "var http = require('node:http'); module.exports = { ClientRequest: http.ClientRequest }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_http_incoming" => Some(
            "var http = require('node:http'); module.exports = { IncomingMessage: http.IncomingMessage, readStart: function(socket) { if (socket && socket.resume) socket.resume(); }, readStop: function(socket) { if (socket && socket.pause) socket.pause(); } }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_http_outgoing" => Some(
            "var http = require('node:http'), OutgoingMessage = http.ServerResponse; module.exports = { OutgoingMessage: OutgoingMessage, validateHeaderName: http.validateHeaderName, validateHeaderValue: http.validateHeaderValue }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_http_server" => Some(
            "var http = require('node:http'); module.exports = { Server: http.Server, ServerResponse: http.ServerResponse, setupConnectionsTracking: function() {}, storeHTTPOptions: function(options) { return options || {}; }, httpServerPreClose: function() {} }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_http_common" => Some(
            r#"var http = require('node:http');
             function HTTPParser(type) { this.type = type; this._headers = []; this._url = ''; }
             HTTPParser.REQUEST = 1; HTTPParser.RESPONSE = 2;
             HTTPParser.prototype.initialize = function(type) { this.type = type; return this; }; HTTPParser.prototype.close = function() {}; HTTPParser.prototype.free = function() {}; HTTPParser.prototype.remove = function() {};
             var parsers = { list: [], alloc: function() { return this.list.pop() || new HTTPParser(); }, free: function(parser) { parser._headers = []; parser._url = ''; this.list.push(parser); } };
             function freeParser(parser) { if (parser && parser.free) parser.free(); }
             function isToken(value) { return /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(String(value)); }
             function invalidHeaderChar(value) { return /[^\t\x20-\x7e\x80-\xff]/.test(String(value)); }
             module.exports = { HTTPParser: HTTPParser, parsers: parsers, freeParser: freeParser, methods: http.METHODS, allMethods: http.METHODS, _checkIsHttpToken: isToken, _checkInvalidHeaderChar: invalidHeaderChar, kLenientNone: 0, kLenientHeaders: 1, kLenientChunkedLength: 2, kLenientKeepAlive: 4, kLenientTransferEncoding: 8, kLenientVersion: 16, kLenientDataAfterClose: 32, kLenientOptionalLFAfterCR: 64, kLenientOptionalCRLFAfterChunk: 128, kLenientOptionalCRBeforeLF: 256, kLenientSpacesAfterChunkSize: 512, kLenientAll: 1023 }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "_tls_common" => Some(
            "var tls = require('node:tls'); module.exports = { SecureContext: tls.SecureContext, createSecureContext: tls.createSecureContext, translatePeerCertificate: function(certificate) { return certificate; } }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_tls_wrap" => Some("module.exports = require('node:tls');\n"),
        "dgram" => Some(
            "function Socket(type, listener) { if (!(this instanceof Socket)) return new Socket(type, listener); var options = typeof type === 'object' ? type : { type: type }; this.type = options.type || 'udp4'; if (this.type !== 'udp4' && this.type !== 'udp6') throw new TypeError('Bad socket type'); this._events = Object.create(null); this._handle = 0; this._address = null; this._remote = null; this._refed = true; if (typeof listener === 'function') this.on('message', listener); } Socket.prototype.on = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; }; Socket.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; }; Socket.prototype.off = Socket.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; Socket.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; };\n\
             Socket.prototype.bind = function(port, address, callback) { var options = typeof port === 'object' ? port : { port: port, address: address }; if (typeof address === 'function') callback = address; if (typeof callback === 'function') this.once('listening', callback); var host = String(options.address || (this.type === 'udp6' ? '::' : '0.0.0.0')); var outcome = __thaw_udp_bind(host, Number(options.port || 0)); if (outcome.indexOf('ok|') !== 0) { var error = new Error(outcome.substring(4)); error.code = 'EADDRINUSE'; queueMicrotask(() => this.emit('error', error)); return this; } var fields = outcome.split('|'); this._handle = Number(fields[1]); this._address = { address: fields[2], family: this.type === 'udp6' ? 'IPv6' : 'IPv4', port: Number(fields[3]) }; queueMicrotask(() => { this.emit('listening'); var incoming = __thaw_udp_receive(this._handle); if (incoming.indexOf('ok|') !== 0) { if (this._handle) { var error = new Error(incoming.substring(4)); error.code = 'EIO'; this.emit('error', error); } return; } var parts = incoming.split('|'); var message = Buffer.from(parts[1], 'hex'); this.emit('message', message, { address: parts[2], family: parts[2].indexOf(':') >= 0 ? 'IPv6' : 'IPv4', port: Number(parts[3]), size: Number(parts[4]) }); }); return this; }; Socket.prototype.send = function(message) { var args = Array.prototype.slice.call(arguments, 1); var callback = typeof args[args.length - 1] === 'function' ? args.pop() : null; var port, address; if (args.length >= 4 && typeof args[0] === 'number' && typeof args[1] === 'number') { var offset = args.shift(), length = args.shift(); message = Buffer.from(message).subarray(offset, offset + length); } port = args.length ? Number(args.shift()) : this._remote && this._remote.port; address = args.length ? String(args.shift()) : this._remote && this._remote.address; if (!this._handle) { var bound = __thaw_udp_bind(this.type === 'udp6' ? '::' : '0.0.0.0', 0); if (bound.indexOf('ok|') !== 0) throw new Error(bound.substring(4)); var fields = bound.split('|'); this._handle = Number(fields[1]); this._address = { address: fields[2], family: this.type === 'udp6' ? 'IPv6' : 'IPv4', port: Number(fields[3]) }; } if (!port || !address) throw new TypeError('Port and address are required'); var buffer = Array.isArray(message) ? Buffer.concat(message.map(function(value) { return Buffer.from(value); })) : Buffer.from(message); var outcome = __thaw_udp_send(this._handle, buffer.toString('hex'), address, port); if (outcome.indexOf('ok|') !== 0) { var error = new Error(outcome.substring(4)); error.code = 'EIO'; if (callback) queueMicrotask(function() { callback(error); }); else queueMicrotask(() => this.emit('error', error)); } else if (callback) queueMicrotask(function() { callback(null, Number(outcome.substring(3))); }); return this; };\n\
             Socket.prototype.connect = function(port, address, callback) { this._remote = { address: String(address || (this.type === 'udp6' ? '::1' : '127.0.0.1')), family: this.type === 'udp6' ? 'IPv6' : 'IPv4', port: Number(port) }; if (callback) queueMicrotask(callback); return this; }; Socket.prototype.disconnect = function() { this._remote = null; }; Socket.prototype.address = function() { if (!this._address) throw new Error('Socket is not running'); return this._address; }; Socket.prototype.remoteAddress = function() { if (!this._remote) throw new Error('Socket is not connected'); return this._remote; }; Socket.prototype.close = function(callback) { if (callback) this.once('close', callback); if (this._handle) __thaw_udp_close(this._handle); this._handle = 0; queueMicrotask(() => this.emit('close')); return this; }; Socket.prototype.ref = function() { this._refed = true; return this; }; Socket.prototype.unref = function() { this._refed = false; return this; }; Socket.prototype.hasRef = function() { return this._refed; }; Socket.prototype.setBroadcast = Socket.prototype.setMulticastLoopback = Socket.prototype.setMulticastTTL = Socket.prototype.setTTL = function() { return this; }; Socket.prototype.addMembership = Socket.prototype.dropMembership = Socket.prototype.addSourceSpecificMembership = Socket.prototype.dropSourceSpecificMembership = function() { return this; }; Socket.prototype.getRecvBufferSize = Socket.prototype.getSendBufferSize = function() { return 212992; }; Socket.prototype.setRecvBufferSize = Socket.prototype.setSendBufferSize = function() { return this; }; function createSocket(type, listener) { return new Socket(type, listener); } module.exports = { Socket: Socket, createSocket: createSocket }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "dns" => Some(
            "var defaultOrder = globalThis.__thaw_dns_order || 'verbatim'; function familyOf(value) { value = String(value); if (/^(?:\\d{1,3}\\.){3}\\d{1,3}$/.test(value) && value.split('.').every(function(part) { return Number(part) <= 255; })) return 4; if (value.indexOf(':') >= 0) return 6; return 0; } function addresses(hostname, family) { var direct = familyOf(hostname); if (direct) return !family || family === direct ? [{ address: String(hostname), family: direct }] : []; if (String(hostname).toLowerCase() === 'localhost') { if (family === 4) return [{ address: '127.0.0.1', family: 4 }]; if (family === 6) return [{ address: '::1', family: 6 }]; return [{ address: '127.0.0.1', family: 4 }, { address: '::1', family: 6 }]; } return []; } function notFound(hostname, syscall) { var error = new Error('getaddrinfo ENOTFOUND ' + hostname); error.code = 'ENOTFOUND'; error.errno = -3008; error.syscall = syscall || 'getaddrinfo'; error.hostname = String(hostname); return error; } function lookup(hostname, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } else if (typeof options === 'number') options = { family: options }; options = options || {}; queueMicrotask(function() { var found = addresses(hostname, Number(options.family || 0)); if (!found.length) { callback(notFound(hostname)); return; } if (options.all) callback(null, found); else callback(null, found[0].address, found[0].family); }); } function resolve(hostname, rrtype, callback) { if (typeof rrtype === 'function') { callback = rrtype; rrtype = 'A'; } queueMicrotask(function() { var found = addresses(hostname, String(rrtype || 'A').toUpperCase() === 'AAAA' ? 6 : 4); if (!found.length) callback(notFound(hostname, 'query' + String(rrtype || 'A'))); else callback(null, found.map(function(entry) { return entry.address; })); }); } function reverse(ip, callback) { queueMicrotask(function() { if (ip === '127.0.0.1' || ip === '::1') callback(null, ['localhost']); else callback(notFound(ip, 'getHostByAddr')); }); } function Resolver() { this._servers = []; } Resolver.prototype.setServers = function(servers) { this._servers = Array.from(servers, String); }; Resolver.prototype.getServers = function() { return this._servers.slice(); }; Resolver.prototype.resolve = resolve; Resolver.prototype.reverse = reverse; var promises = { lookup: function(hostname, options) { return new Promise(function(resolvePromise, reject) { lookup(hostname, options, function(error, address, family) { if (error) reject(error); else if (options && options.all) resolvePromise(address); else resolvePromise({ address: address, family: family }); }); }); }, resolve: function(hostname, rrtype) { return new Promise(function(resolvePromise, reject) { resolve(hostname, rrtype, function(error, value) { error ? reject(error) : resolvePromise(value); }); }); }, reverse: function(ip) { return new Promise(function(resolvePromise, reject) { reverse(ip, function(error, value) { error ? reject(error) : resolvePromise(value); }); }); } }; module.exports = { lookup: lookup, resolve: resolve, resolve4: function(hostname, callback) { resolve(hostname, 'A', callback); }, resolve6: function(hostname, callback) { resolve(hostname, 'AAAA', callback); }, reverse: reverse, Resolver: Resolver, promises: promises, getDefaultResultOrder: function() { return defaultOrder; }, setDefaultResultOrder: function(order) { defaultOrder = String(order); globalThis.__thaw_dns_order = defaultOrder; }, getServers: function() { return []; }, setServers: function() {}, ADDRCONFIG: 32, V4MAPPED: 8, NODATA: 'ENODATA', FORMERR: 'EFORMERR', SERVFAIL: 'ESERVFAIL', NOTFOUND: 'ENOTFOUND', NOTIMP: 'ENOTIMP', REFUSED: 'EREFUSED' }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "dns/promises" => Some(
            "function familyOf(value) { value = String(value); if (/^(?:\\d{1,3}\\.){3}\\d{1,3}$/.test(value) && value.split('.').every(function(part) { return Number(part) <= 255; })) return 4; if (value.indexOf(':') >= 0) return 6; return 0; } function addresses(hostname, family) { var direct = familyOf(hostname); if (direct) return !family || family === direct ? [{ address: String(hostname), family: direct }] : []; if (String(hostname).toLowerCase() === 'localhost') { if (family === 4) return [{ address: '127.0.0.1', family: 4 }]; if (family === 6) return [{ address: '::1', family: 6 }]; return [{ address: '127.0.0.1', family: 4 }, { address: '::1', family: 6 }]; } return []; } function failure(hostname) { var error = new Error('getaddrinfo ENOTFOUND ' + hostname); error.code = 'ENOTFOUND'; error.errno = -3008; error.syscall = 'getaddrinfo'; error.hostname = String(hostname); return error; } function lookup(hostname, options) { options = typeof options === 'number' ? { family: options } : (options || {}); var found = addresses(hostname, Number(options.family || 0)); if (!found.length) return Promise.reject(failure(hostname)); return Promise.resolve(options.all ? found : found[0]); } function resolve(hostname, rrtype) { var found = addresses(hostname, String(rrtype || 'A').toUpperCase() === 'AAAA' ? 6 : 4); return found.length ? Promise.resolve(found.map(function(entry) { return entry.address; })) : Promise.reject(failure(hostname)); } function reverse(ip) { return ip === '127.0.0.1' || ip === '::1' ? Promise.resolve(['localhost']) : Promise.reject(failure(ip)); } function Resolver() { this._servers = []; } Resolver.prototype.setServers = function(servers) { this._servers = Array.from(servers, String); }; Resolver.prototype.getServers = function() { return this._servers.slice(); }; Resolver.prototype.resolve = resolve; Resolver.prototype.reverse = reverse; module.exports = { lookup: lookup, resolve: resolve, resolve4: function(hostname) { return resolve(hostname, 'A'); }, resolve6: function(hostname) { return resolve(hostname, 'AAAA'); }, reverse: reverse, Resolver: Resolver, getDefaultResultOrder: function() { return globalThis.__thaw_dns_order || 'verbatim'; }, setDefaultResultOrder: function(order) { globalThis.__thaw_dns_order = String(order); }, getServers: function() { return []; }, setServers: function() {} }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "async_hooks" => Some(
            "var storages = globalThis.__thawAsyncLocalStorages || (globalThis.__thawAsyncLocalStorages = new Set()); var nextAsyncId = globalThis.__thawNextAsyncId || 2; var currentAsyncId = 1; var currentTriggerId = 0; var currentResource = {};\n\
             function restoreAfter(storage, previous, result) { if (result && typeof result.then === 'function') return Promise.resolve(result).finally(function() { storage._store = previous; }); storage._store = previous; return result; }\n\
             function AsyncLocalStorage(options) { if (!(this instanceof AsyncLocalStorage)) return new AsyncLocalStorage(options); this._store = options && options.defaultValue; this.name = options && options.name || ''; this.enabled = true; storages.add(this); }\n\
             AsyncLocalStorage.prototype.disable = function() { this.enabled = false; this._store = undefined; };\n\
             AsyncLocalStorage.prototype.getStore = function() { return this.enabled ? this._store : undefined; };\n\
             AsyncLocalStorage.prototype.enterWith = function(store) { this.enabled = true; this._store = store; storages.add(this); };\n\
             AsyncLocalStorage.prototype.run = function(store, callback) { var args = Array.prototype.slice.call(arguments, 2); var previous = this._store; this.enabled = true; this._store = store; try { return restoreAfter(this, previous, callback.apply(null, args)); } catch (error) { this._store = previous; throw error; } };\n\
             AsyncLocalStorage.prototype.exit = function(callback) { var args = Array.prototype.slice.call(arguments, 1); var previous = this._store; this._store = undefined; try { return restoreAfter(this, previous, callback.apply(null, args)); } catch (error) { this._store = previous; throw error; } };\n\
             function captureStores() { return Array.from(storages).map(function(storage) { return [storage, storage.getStore()]; }); }\n\
             function invokeCaptured(captured, callback, thisArg, args, index) { if (index === captured.length) return callback.apply(thisArg, args); var entry = captured[index]; return entry[0].run(entry[1], function() { return invokeCaptured(captured, callback, thisArg, args, index + 1); }); }\n\
             AsyncLocalStorage.bind = function(callback) { var captured = captureStores(); return function() { return invokeCaptured(captured, callback, this, Array.prototype.slice.call(arguments), 0); }; };\n\
             AsyncLocalStorage.snapshot = function() { var captured = captureStores(); return function(callback) { return invokeCaptured(captured, callback, null, Array.prototype.slice.call(arguments, 1), 0); }; };\n\
             function AsyncResource(type, options) { if (!(this instanceof AsyncResource)) return new AsyncResource(type, options); this.type = String(type); this._asyncId = nextAsyncId++; globalThis.__thawNextAsyncId = nextAsyncId; this._triggerAsyncId = options && options.triggerAsyncId !== undefined ? Number(options.triggerAsyncId) : currentAsyncId; this._destroyed = false; }\n\
             AsyncResource.prototype.asyncId = function() { return this._asyncId; }; AsyncResource.prototype.triggerAsyncId = function() { return this._triggerAsyncId; };\n\
             AsyncResource.prototype.runInAsyncScope = function(callback, thisArg) { var args = Array.prototype.slice.call(arguments, 2); var previousId = currentAsyncId, previousTrigger = currentTriggerId, previousResource = currentResource; currentAsyncId = this._asyncId; currentTriggerId = this._triggerAsyncId; currentResource = this; try { return callback.apply(thisArg, args); } finally { currentAsyncId = previousId; currentTriggerId = previousTrigger; currentResource = previousResource; } };\n\
             AsyncResource.prototype.emitDestroy = function() { this._destroyed = true; return this; }; AsyncResource.prototype.bind = function(callback, thisArg) { var self = this; return function() { return self.runInAsyncScope(callback, thisArg === undefined ? this : thisArg, ...arguments); }; };\n\
             AsyncResource.bind = function(callback, type, thisArg) { return new AsyncResource(type || callback.name || 'bound-anonymous-fn').bind(callback, thisArg); };\n\
             function createHook(callbacks) { return { enable: function() { return this; }, disable: function() { return this; }, callbacks: callbacks || {} }; }\n\
             function executionAsyncId() { return currentAsyncId; } function triggerAsyncId() { return currentTriggerId; } function executionAsyncResource() { return currentResource; }\n\
             module.exports = { AsyncLocalStorage: AsyncLocalStorage, AsyncResource: AsyncResource, createHook: createHook, executionAsyncId: executionAsyncId, triggerAsyncId: triggerAsyncId, executionAsyncResource: executionAsyncResource }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "tty" => Some(
            "function isatty(fd) { return false; }\n\
             function ReadStream(fd, options) { if (!(this instanceof ReadStream)) return new ReadStream(fd, options); this.fd = Number(fd); this.isRaw = false; this.isTTY = true; this.readable = true; this.destroyed = false; }\n\
             ReadStream.prototype.setRawMode = function(mode) { this.isRaw = Boolean(mode); return this; }; ReadStream.prototype.ref = function() { return this; }; ReadStream.prototype.unref = function() { return this; };\n\
             function WriteStream(fd) { if (!(this instanceof WriteStream)) return new WriteStream(fd); this.fd = Number(fd); this.isTTY = true; this.columns = 80; this.rows = 24; this.writable = true; this.destroyed = false; this._output = ''; }\n\
             WriteStream.prototype.write = function(value, callback) { this._output += String(value); if (typeof callback === 'function') queueMicrotask(callback); return true; };\n\
             WriteStream.prototype.getColorDepth = function(environment) { var env = environment || globalThis.process && process.env || {}; if (env.FORCE_COLOR === '0' || env.NO_COLOR !== undefined) return 1; if (env.FORCE_COLOR === '3' || env.COLORTERM === 'truecolor') return 24; if (env.FORCE_COLOR === '2' || /256color/i.test(env.TERM || '')) return 8; if (env.FORCE_COLOR === '1' || /color|ansi|xterm|screen/i.test(env.TERM || '')) return 4; return 1; };\n\
             WriteStream.prototype.hasColors = function(count, environment) { if (typeof count === 'object') { environment = count; count = 16; } count = count === undefined ? 16 : Number(count); return Math.pow(2, this.getColorDepth(environment)) >= count; };\n\
             WriteStream.prototype._ansi = function(sequence, callback) { this._output += sequence; if (typeof callback === 'function') queueMicrotask(callback); return true; };\n\
             WriteStream.prototype.clearLine = function(direction, callback) { var sequence = Number(direction) < 0 ? '\\u001b[1K' : Number(direction) > 0 ? '\\u001b[0K' : '\\u001b[2K'; return this._ansi(sequence, callback); };\n\
             WriteStream.prototype.clearScreenDown = function(callback) { return this._ansi('\\u001b[0J', callback); };\n\
             WriteStream.prototype.cursorTo = function(x, y, callback) { if (typeof y === 'function') { callback = y; y = undefined; } var sequence = y === undefined ? '\\u001b[' + (Number(x) + 1) + 'G' : '\\u001b[' + (Number(y) + 1) + ';' + (Number(x) + 1) + 'H'; return this._ansi(sequence, callback); };\n\
             WriteStream.prototype.moveCursor = function(dx, dy, callback) { var sequence = ''; dx = Number(dx); dy = Number(dy); if (dx < 0) sequence += '\\u001b[' + -dx + 'D'; else if (dx > 0) sequence += '\\u001b[' + dx + 'C'; if (dy < 0) sequence += '\\u001b[' + -dy + 'A'; else if (dy > 0) sequence += '\\u001b[' + dy + 'B'; return this._ansi(sequence, callback); };\n\
             WriteStream.prototype.getWindowSize = function() { return [this.columns, this.rows]; }; WriteStream.prototype.ref = function() { return this; }; WriteStream.prototype.unref = function() { return this; };\n\
             module.exports = { isatty: isatty, ReadStream: ReadStream, WriteStream: WriteStream }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "module" => Some(
            "var builtinModules = ['_http_agent','_http_client','_http_common','_http_incoming','_http_outgoing','_http_server','_stream_duplex','_stream_passthrough','_stream_readable','_stream_transform','_stream_wrap','_stream_writable','_tls_common','_tls_wrap','assert','assert/strict','async_hooks','buffer','cluster','console','constants','crypto','dgram','diagnostics_channel','dns','dns/promises','domain','events','fs','fs/promises','http','http2','inspector','inspector/promises','module','net','os','path','path/posix','path/win32','perf_hooks','process','punycode','querystring','readline','readline/promises','repl','stream','stream/consumers','stream/promises','stream/web','string_decoder','sys','test','test/reporters','timers','timers/promises','tls','trace_events','tty','url','util','util/types','v8','vm','wasi','worker_threads','zlib']; var builtinSet = new Set(builtinModules);\n\
             function isBuiltin(name) { var value = String(name); return builtinSet.has(value.replace(/^node:/, '')); }\n\
             function createRequire(filename) { if (typeof globalThis.__thaw_bundle_create_require !== 'function') throw new Error('createRequire is only available inside a Thaw bundle'); return globalThis.__thaw_bundle_create_require(filename); }\n\
             function Module(id, parent) { if (!(this instanceof Module)) return new Module(id, parent); this.id = id === undefined ? '' : String(id); this.path = this.id; this.exports = {}; this.filename = null; this.loaded = false; this.parent = parent || null; this.children = []; this.paths = []; if (parent && parent.children) parent.children.push(this); }\n\
             Module.builtinModules = builtinModules; Module.isBuiltin = isBuiltin; Module.createRequire = createRequire; Module._cache = {}; Module._extensions = { '.js': function() {}, '.json': function() {}, '.node': function() {} };\n\
             function syncBuiltinESMExports() {} function findSourceMap() { return undefined; }\n\
             function SourceMap(payload) { if (!(this instanceof SourceMap)) return new SourceMap(payload); this.payload = payload || {}; } SourceMap.prototype.findEntry = function(line, column) { return { generatedLine: Number(line), generatedColumn: Number(column || 0), originalSource: undefined, originalLine: undefined, originalColumn: undefined, name: undefined }; }; SourceMap.prototype.findOrigin = function(line, column) { return this.findEntry(line, column); };\n\
             function register() { return undefined; } function registerHooks(hooks) { var active = true; return { deregister: function() { active = false; }, get active() { return active; }, hooks: hooks }; }\n\
             Object.assign(Module, { Module: Module, createRequire: createRequire, builtinModules: builtinModules, isBuiltin: isBuiltin, syncBuiltinESMExports: syncBuiltinESMExports, findSourceMap: findSourceMap, SourceMap: SourceMap, register: register, registerHooks: registerHooks });\n\
             module.exports = Module; module.exports.default = Module; module.exports.__esModule = true;\n",
        ),
        "net" => Some(concat!(
            "function isIPv4(value) { if (typeof value !== 'string' || !/^(?:\\d{1,3}\\.){3}\\d{1,3}$/.test(value)) return false; return value.split('.').every(function(part) { return String(Number(part)) === part && Number(part) <= 255; }); } function isIPv6(value) { if (typeof value !== 'string') return false; var address = value.split('%')[0]; if (address.indexOf(':') < 0 || address.indexOf(':::') >= 0) return false; var halves = address.split('::'); if (halves.length > 2) return false; function count(part) { if (!part) return 0; var groups = part.split(':'); for (var index = 0; index < groups.length; index++) { if (isIPv4(groups[index])) { if (index !== groups.length - 1) return -1; } else if (!/^[0-9a-fA-F]{1,4}$/.test(groups[index])) return -1; } return groups.reduce(function(total, group) { return total + (isIPv4(group) ? 2 : 1); }, 0); } var total = count(halves[0]) + count(halves[1] || ''); return total >= 0 && (halves.length === 2 ? total < 8 : total === 8); } function isIP(value) { return isIPv4(value) ? 4 : isIPv6(value) ? 6 : 0; } function ipv4Number(value) { return value.split('.').reduce(function(result, part) { return (result * 256 + Number(part)) >>> 0; }, 0); }\n\
             function SocketAddress(options) { if (!(this instanceof SocketAddress)) return new SocketAddress(options); options = options || {}; this.address = options.address === undefined ? '127.0.0.1' : String(options.address); var detected = isIP(this.address); var requested = options.family === undefined ? detected : (String(options.family).toLowerCase() === 'ipv6' || Number(options.family) === 6 ? 6 : 4); if (!detected || detected !== requested) throw new TypeError('Invalid socket address'); this.family = requested === 6 ? 'ipv6' : 'ipv4'; this.port = options.port === undefined ? 0 : Number(options.port); if (!Number.isInteger(this.port) || this.port < 0 || this.port > 65535) throw new RangeError('port must be between 0 and 65535'); this.flowlabel = options.flowlabel === undefined ? 0 : Number(options.flowlabel); } SocketAddress.prototype.toJSON = function() { return { address: this.address, port: this.port, family: this.family, flowlabel: this.flowlabel }; };\n\
             function BlockList() { if (!(this instanceof BlockList)) return new BlockList(); this.rules = []; } BlockList.prototype.addAddress = function(address, type) { var family = type === 'ipv6' ? 6 : type === 'ipv4' ? 4 : isIP(address); if (!family) throw new TypeError('Invalid IP address'); this.rules.push({ kind: 'address', address: String(address), family: family }); }; BlockList.prototype.addRange = function(start, end, type) { var family = type === 'ipv6' ? 6 : type === 'ipv4' ? 4 : isIP(start); if (!family || family !== isIP(end)) throw new TypeError('Invalid IP range'); this.rules.push({ kind: 'range', start: String(start), end: String(end), family: family }); }; BlockList.prototype.addSubnet = function(network, prefix, type) { var family = type === 'ipv6' ? 6 : type === 'ipv4' ? 4 : isIP(network); prefix = Number(prefix); if (!family || !Number.isInteger(prefix) || prefix < 0 || prefix > (family === 4 ? 32 : 128)) throw new TypeError('Invalid subnet'); this.rules.push({ kind: 'subnet', network: String(network), prefix: prefix, family: family }); }; BlockList.prototype.check = function(address, type) { var family = type === 'ipv6' ? 6 : type === 'ipv4' ? 4 : isIP(address); if (!family) return false; return this.rules.some(function(rule) { if (rule.family !== family) return false; if (rule.kind === 'address') return rule.address === address; if (family === 4) { var value = ipv4Number(address); if (rule.kind === 'range') return value >= ipv4Number(rule.start) && value <= ipv4Number(rule.end); var mask = rule.prefix === 0 ? 0 : (0xffffffff << (32 - rule.prefix)) >>> 0; return (value & mask) === (ipv4Number(rule.network) & mask); } if (rule.kind === 'range') return address >= rule.start && address <= rule.end; return rule.prefix === 128 ? address === rule.network : address.toLowerCase().startsWith(rule.network.toLowerCase().split('::')[0]); }); };\n\
             function Socket(options) { if (!(this instanceof Socket)) return new Socket(options); this._events = Object.create(null); this._handle = 0; this.connecting = false; this.destroyed = false; this.readable = false; this.writable = false; this.pending = true; this.bytesRead = 0; this.bytesWritten = 0; this.remoteAddress = undefined; this.remotePort = undefined; this.remoteFamily = undefined; this._refed = true; } Socket.prototype.on = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; }; Socket.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; }; Socket.prototype.off = Socket.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; Socket.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; }; Socket.prototype.connect = function(port, host, listener) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') listener = host; if (typeof listener === 'function') this.once('connect', listener); var hostname = String(options.host || 'localhost'); var numericPort = Number(options.port); this.connecting = true; var outcome = __thaw_net_connect(hostname, numericPort); this.connecting = false; this.pending = false; if (outcome.indexOf('ok:') === 0) { this._handle = Number(outcome.substring(3)); this.readable = true; this.writable = true; this.remoteAddress = hostname; this.remotePort = numericPort; this.remoteFamily = isIPv6(hostname) ? 'IPv6' : 'IPv4'; queueMicrotask(() => this.emit('connect')); } else { var error = new Error(outcome.substring(4)); error.code = 'ECONNREFUSED'; error.syscall = 'connect'; error.address = hostname; error.port = numericPort; this.destroyed = true; queueMicrotask(() => { this.emit('error', error); this.emit('close', true); }); } return this; }; Socket.prototype.write = function(value, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (!this._handle || !this.writable) throw new Error('Socket is not writable'); var buffer = Buffer.isBuffer(value) ? value : Buffer.from(value, encoding); var outcome = __thaw_net_write(this._handle, buffer.toString('hex')); if (outcome !== 'ok') throw new Error(outcome.substring(4)); this.bytesWritten += buffer.length; if (typeof callback === 'function') queueMicrotask(callback); return true; }; Socket.prototype.end = function(value, encoding, callback) { if (typeof value === 'function') { callback = value; value = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (value !== undefined) this.write(value, encoding); if (!this._handle) return this; var outcome = __thaw_net_finish(this._handle); this._handle = 0; this.writable = false; this.readable = false; this.destroyed = true; queueMicrotask(() => { if (outcome.indexOf('ok:') === 0) { var data = Buffer.from(outcome.substring(3), 'hex'); if (data.length) { this.bytesRead += data.length; this.emit('data', data); } this.emit('end'); if (typeof callback === 'function') callback(); this.emit('close', false); } else { var error = new Error(outcome.substring(4)); error.code = 'ECONNRESET'; this.emit('error', error); this.emit('close', true); } }); return this; }; Socket.prototype.destroy = function(error) { if (this._handle) __thaw_net_destroy(this._handle); this._handle = 0; this.destroyed = true; this.readable = false; this.writable = false; queueMicrotask(() => { if (error) this.emit('error', error); this.emit('close', Boolean(error)); }); return this; }; Socket.prototype.setEncoding = function(encoding) { var original = this.emit; this.emit = function(name, value) { if (name === 'data' && Buffer.isBuffer(value)) value = value.toString(encoding); return original.call(this, name, value); }; return this; }; Socket.prototype.setTimeout = function(timeout, callback) { if (typeof callback === 'function') this.once('timeout', callback); return this; }; Socket.prototype.setNoDelay = Socket.prototype.setKeepAlive = function() { return this; }; Socket.prototype.ref = function() { this._refed = true; return this; }; Socket.prototype.unref = function() { this._refed = false; return this; }; Socket.prototype.hasRef = function() { return this._refed; }; function createConnection() { var socket = new Socket(); return socket.connect.apply(socket, arguments); }\n\
             function Server(options, listener) { if (!(this instanceof Server)) return new Server(options, listener); if (typeof options === 'function') { listener = options; options = {}; } this._events = Object.create(null); this._handle = 0; this._address = null; this.listening = false; this.maxConnections = 0; this.connections = 0; this._refed = true; if (typeof listener === 'function') this.on('connection', listener); } Server.prototype.on = Socket.prototype.on; Server.prototype.once = Socket.prototype.once; Server.prototype.off = Server.prototype.removeListener = Socket.prototype.off; Server.prototype.emit = Socket.prototype.emit; Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var outcome = __thaw_net_listen(hostname, Number(options.port)); if (outcome.indexOf('ok:') !== 0) { var error = new Error(outcome.substring(4)); error.code = 'EADDRINUSE'; queueMicrotask(() => this.emit('error', error)); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: isIPv6(hostname) ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; queueMicrotask(() => { this.emit('listening'); var accepted = __thaw_net_accept(this._handle); if (accepted.indexOf('ok:') !== 0) return; var peer = accepted.split(':'); var socket = new Socket(); socket._handle = Number(peer[1]); socket.pending = false; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = isIPv6(peer[2]) ? 'IPv6' : 'IPv4'; this.connections++; this.emit('connection', socket); var incoming = __thaw_net_read(socket._handle); if (incoming.indexOf('ok:') === 0) { var data = Buffer.from(incoming.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } socket.emit('end'); } else { var readError = new Error(incoming.substring(4)); readError.code = 'ECONNRESET'; socket.emit('error', readError); } }); return this; }; Server.prototype.address = function() { return this._address; }; Server.prototype.getConnections = function(callback) { queueMicrotask(() => callback(null, this.connections)); }; Server.prototype.close = function(callback) { if (typeof callback === 'function') this.once('close', callback); if (this._handle) __thaw_net_close_listener(this._handle); this._handle = 0; this.listening = false; queueMicrotask(() => this.emit('close')); return this; }; Server.prototype.closeAllConnections = Server.prototype.closeIdleConnections = function() {}; Server.prototype.ref = function() { this._refed = true; return this; }; Server.prototype.unref = function() { this._refed = false; return this; }; function createServer(options, listener) { return new Server(options, listener); }\n\
             module.exports = { isIP: isIP, isIPv4: isIPv4, isIPv6: isIPv6, BlockList: BlockList, SocketAddress: SocketAddress, Socket: Socket, createConnection: createConnection, connect: createConnection, Server: Server, createServer: createServer }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
            r#"
             Socket.prototype.end = function(value, encoding, callback) { if (typeof value === 'function') { callback = value; value = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (value !== undefined) this.write(value, encoding); if (!this._handle || !this.writable) return this; var socket = this, outcome = __thaw_net_shutdown(this._handle); this.writable = false; if (outcome !== 'ok') { var shutdownError = new Error(outcome.substring(4)); shutdownError.code = 'ECONNRESET'; this.destroy(shutdownError); return this; } function pump() { if (!socket._handle || socket.destroyed) return; var result = __thaw_net_poll_read(socket._handle); if (result === 'pending') { setTimeout(pump, 0); return; } if (result.indexOf('ok:') === 0) { var data = Buffer.from(result.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } setTimeout(pump, 0); return; } var failed = result.indexOf('err:') === 0; socket._handle = 0; socket.readable = false; socket.destroyed = true; if (failed) { var error = new Error(result.substring(4)); error.code = 'ECONNRESET'; socket.emit('error', error); socket.emit('close', true); } else { socket.emit('end'); if (typeof callback === 'function') callback(); socket.emit('close', false); } } queueMicrotask(pump); return this; };
             Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var outcome = __thaw_net_listen(hostname, Number(options.port)); if (outcome.indexOf('ok:') !== 0) { var error = new Error(outcome.substring(4)); error.code = 'EADDRINUSE'; queueMicrotask(() => this.emit('error', error)); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: isIPv6(hostname) ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; var server = this; function pump() { if (!server._handle || !server.listening) return; if (server.maxConnections > 0 && server.connections >= server.maxConnections) { setTimeout(pump, 1); return; } var accepted = __thaw_net_poll_accept(server._handle); if (accepted === 'err:pending') { setTimeout(pump, 1); return; } if (accepted.indexOf('ok:') !== 0) { var error = new Error(accepted.substring(4)); error.code = 'ECONNABORTED'; server.emit('error', error); if (server._handle) setTimeout(pump, 1); return; } var peer = accepted.split(':'); var socket = new Socket(); socket._handle = Number(peer[1]); socket.pending = false; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = isIPv6(peer[2]) ? 'IPv6' : 'IPv4'; server.connections++; socket.once('close', function() { server.connections = Math.max(0, server.connections - 1); }); server.emit('connection', socket); var incoming = __thaw_net_read(socket._handle); if (incoming.indexOf('ok:') === 0) { var data = Buffer.from(incoming.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } socket.emit('end'); } else { var readError = new Error(incoming.substring(4)); readError.code = 'ECONNRESET'; socket.emit('error', readError); } if (server._handle) setTimeout(pump, 0); } queueMicrotask(function() { server.emit('listening'); pump(); }); return this; };
"#,
        )),
        "tls" => Some(concat!(
            "function TLSSocket(socket, options) { if (!(this instanceof TLSSocket)) return new TLSSocket(socket, options); this._events = Object.create(null); this._handle = 0; this.connecting = false; this.destroyed = false; this.readable = false; this.writable = false; this.encrypted = true; this.authorized = false; this.authorizationError = null; this.alpnProtocol = false; this.servername = null; this.bytesRead = 0; this.bytesWritten = 0; this._refed = true; } TLSSocket.prototype.on = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; }; TLSSocket.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; }; TLSSocket.prototype.off = TLSSocket.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; TLSSocket.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; }; TLSSocket.prototype.connect = function(options, listener) { if (typeof options === 'number') options = { port: options }; options = options || {}; if (typeof listener === 'function') this.once('secureConnect', listener); var host = String(options.host || 'localhost'), port = Number(options.port), servername = String(options.servername || host); var ca = options.ca; if (Array.isArray(ca)) ca = ca[0]; var caHex = ca === undefined ? '' : Buffer.from(ca).toString('hex'); this.connecting = true; var outcome = __thaw_tls_connect(host, port, servername, caHex); this.connecting = false; this.servername = servername; if (outcome.indexOf('ok:') === 0) { this._handle = Number(outcome.substring(3)); this.readable = true; this.writable = true; this.authorized = true; queueMicrotask(() => { this.emit('connect'); this.emit('secureConnect'); }); } else { var error = new Error(outcome.substring(4)); error.code = 'ERR_TLS_CERT_ALTNAME_INVALID'; this.authorizationError = error.message; this.destroyed = true; queueMicrotask(() => { this.emit('error', error); this.emit('close', true); }); } return this; }; TLSSocket.prototype.write = function(value, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (!this._handle || !this.writable) throw new Error('TLS socket is not writable'); var buffer = Buffer.isBuffer(value) ? value : Buffer.from(value, encoding); var outcome = __thaw_tls_write(this._handle, buffer.toString('hex')); if (outcome !== 'ok') throw new Error(outcome.substring(4)); this.bytesWritten += buffer.length; if (callback) queueMicrotask(callback); return true; }; TLSSocket.prototype.end = function(value, encoding, callback) { if (typeof value === 'function') { callback = value; value = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (value !== undefined) this.write(value, encoding); var outcome = __thaw_tls_finish(this._handle); this._handle = 0; this.writable = false; this.readable = false; this.destroyed = true; queueMicrotask(() => { if (outcome.indexOf('ok:') === 0) { var data = Buffer.from(outcome.substring(3), 'hex'); if (data.length) { this.bytesRead += data.length; this.emit('data', data); } this.emit('end'); if (callback) callback(); this.emit('close', false); } else { var error = new Error(outcome.substring(4)); error.code = 'ECONNRESET'; this.emit('error', error); this.emit('close', true); } }); return this; }; TLSSocket.prototype.destroy = function(error) { if (this._handle) __thaw_tls_destroy(this._handle); this._handle = 0; this.destroyed = true; if (error) queueMicrotask(() => this.emit('error', error)); queueMicrotask(() => this.emit('close', Boolean(error))); return this; }; TLSSocket.prototype.setEncoding = function(encoding) { var emit = this.emit; this.emit = function(name, value) { if (name === 'data' && Buffer.isBuffer(value)) value = value.toString(encoding); return emit.call(this, name, value); }; return this; }; TLSSocket.prototype.getProtocol = function() { return this.authorized ? 'TLSv1.3' : null; }; TLSSocket.prototype.getCipher = function() { return { name: 'TLS_AES_256_GCM_SHA384', standardName: 'TLS_AES_256_GCM_SHA384', version: 'TLSv1.3' }; }; TLSSocket.prototype.getPeerCertificate = function() { return this.authorized ? { subject: {}, issuer: {}, valid_from: '', valid_to: '' } : {}; }; TLSSocket.prototype.getCertificate = function() { return {}; }; TLSSocket.prototype.getFinished = TLSSocket.prototype.getPeerFinished = function() { return undefined; }; TLSSocket.prototype.isSessionReused = function() { return false; }; TLSSocket.prototype.renegotiate = function(options, callback) { if (callback) queueMicrotask(function() { callback(new Error('TLS renegotiation is not supported')); }); return false; }; TLSSocket.prototype.setMaxSendFragment = function() { return true; }; TLSSocket.prototype.enableTrace = function() {}; TLSSocket.prototype.ref = function() { this._refed = true; return this; }; TLSSocket.prototype.unref = function() { this._refed = false; return this; }; function connect(options, listener) { return new TLSSocket().connect(options, listener); } function SecureContext(options) { if (!(this instanceof SecureContext)) return new SecureContext(options); this.context = options || {}; this.options = options || {}; } SecureContext.prototype.setKey = function(key) { this.options.key = key; }; SecureContext.prototype.setCert = function(cert) { this.options.cert = cert; }; SecureContext.prototype.addCACert = function(ca) { var list = this.options.ca || (this.options.ca = []); if (!Array.isArray(list)) list = this.options.ca = [list]; list.push(ca); }; function createSecureContext(options) { return new SecureContext(options); } function checkServerIdentity() { return undefined; } function getCiphers() { return ['tls_aes_128_gcm_sha256', 'tls_aes_256_gcm_sha384', 'tls_chacha20_poly1305_sha256']; } module.exports = { TLSSocket: TLSSocket, SecureContext: SecureContext, connect: connect, createSecureContext: createSecureContext, checkServerIdentity: checkServerIdentity, getCiphers: getCiphers, rootCertificates: [], DEFAULT_MIN_VERSION: 'TLSv1.2', DEFAULT_MAX_VERSION: 'TLSv1.3', CLIENT_RENEG_LIMIT: 3, CLIENT_RENEG_WINDOW: 600 }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
            r#"
             function Server(options, listener) { if (!(this instanceof Server)) return new Server(options, listener); this._events = Object.create(null); this._options = options || {}; this._handle = 0; this._address = null; this.listening = false; this.connections = 0; this.maxConnections = 0; this._refed = true; if (typeof listener === 'function') this.on('secureConnection', listener); }
             Server.prototype.on = TLSSocket.prototype.on; Server.prototype.once = TLSSocket.prototype.once; Server.prototype.off = Server.prototype.removeListener = TLSSocket.prototype.off; Server.prototype.emit = TLSSocket.prototype.emit;
             TLSSocket.prototype.end = function(value, encoding, callback) { if (typeof value === 'function') { callback = value; value = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (value !== undefined) this.write(value, encoding); if (!this._handle || !this.writable) return this; var socket = this, outcome = __thaw_tls_shutdown(this._handle); this.writable = false; if (outcome !== 'ok') { this.destroy(new Error(outcome.substring(4))); return this; } function pump() { if (!socket._handle || socket.destroyed) return; var result = __thaw_tls_poll_read(socket._handle); if (result === 'pending') { setTimeout(pump, 0); return; } if (result.indexOf('ok:') === 0) { var data = Buffer.from(result.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } setTimeout(pump, 0); return; } var failed = result.indexOf('err:') === 0; socket._handle = 0; socket.readable = false; socket.destroyed = true; if (failed) { var error = new Error(result.substring(4)); error.code = 'ECONNRESET'; socket.emit('error', error); socket.emit('close', true); } else { socket.emit('end'); if (callback) callback(); socket.emit('close', false); } } queueMicrotask(pump); return this; };
             Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var cert = this._options.cert, key = this._options.key; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; if (cert === undefined || key === undefined) { queueMicrotask(() => this.emit('error', new Error('cert and key are required'))); return this; } var outcome = __thaw_tls_server_listen(hostname, Number(options.port), Buffer.from(cert).toString('hex'), Buffer.from(key).toString('hex')); if (outcome.indexOf('ok:') !== 0) { queueMicrotask(() => this.emit('error', new Error(outcome.substring(4)))); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: hostname.indexOf(':') >= 0 ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; queueMicrotask(() => { this.emit('listening'); var accepted = __thaw_tls_server_accept(this._handle); if (accepted.indexOf('ok:') !== 0) { this.emit('tlsClientError', new Error(accepted.substring(4))); return; } var peer = accepted.split(':'); var socket = new TLSSocket(); socket._handle = Number(peer[1]); socket.authorized = true; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = peer[2].indexOf(':') >= 0 ? 'IPv6' : 'IPv4'; this.connections++; this.emit('secureConnection', socket); var incoming = __thaw_tls_server_read(socket._handle); if (incoming.indexOf('ok:') === 0) { var data = Buffer.from(incoming.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } socket.emit('end'); } else socket.emit('error', new Error(incoming.substring(4))); }); return this; };
             Server.prototype.address = function() { return this._address; }; Server.prototype.getConnections = function(callback) { queueMicrotask(() => callback(null, this.connections)); }; Server.prototype.close = function(callback) { if (typeof callback === 'function') this.once('close', callback); if (this._handle) __thaw_tls_server_close(this._handle); this._handle = 0; this.listening = false; queueMicrotask(() => this.emit('close')); return this; }; Server.prototype.ref = function() { this._refed = true; return this; }; Server.prototype.unref = function() { this._refed = false; return this; }; function createServer(options, listener) { return new Server(options, listener); }
             var connectWithoutIdentity = TLSSocket.prototype.connect; TLSSocket.prototype.connect = function(options, listener) { options = options || {}; if (options.cert === undefined && options.key === undefined) return connectWithoutIdentity.call(this, options, listener); var cert = options.cert, key = options.key; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; var nativeConnect = __thaw_tls_connect; __thaw_tls_connect = function(host, port, servername, ca) { return __thaw_tls_connect_with_identity(host, port, servername, ca, cert === undefined ? '' : Buffer.from(cert).toString('hex'), key === undefined ? '' : Buffer.from(key).toString('hex')); }; try { return connectWithoutIdentity.call(this, options, listener); } finally { __thaw_tls_connect = nativeConnect; } };
             var listenWithoutClientAuth = Server.prototype.listen; Server.prototype.listen = function() { if (!this._options.requestCert) return listenWithoutClientAuth.apply(this, arguments); var ca = this._options.ca; if (Array.isArray(ca)) ca = ca[0]; var caHex = ca === undefined ? '' : Buffer.from(ca).toString('hex'); var rejectUnauthorized = this._options.rejectUnauthorized !== false; var nativeListen = __thaw_tls_server_listen; __thaw_tls_server_listen = function(host, port, cert, key) { return __thaw_tls_server_listen_with_ca(host, port, cert, key, caHex, rejectUnauthorized); }; try { return listenWithoutClientAuth.apply(this, arguments); } finally { __thaw_tls_server_listen = nativeListen; } };
             Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var cert = this._options.cert, key = this._options.key, ca = this._options.ca; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; if (Array.isArray(ca)) ca = ca[0]; if (cert === undefined || key === undefined) { queueMicrotask(() => this.emit('error', new Error('cert and key are required'))); return this; } var certHex = Buffer.from(cert).toString('hex'), keyHex = Buffer.from(key).toString('hex'); var outcome = this._options.requestCert ? __thaw_tls_server_listen_with_ca(hostname, Number(options.port), certHex, keyHex, ca === undefined ? '' : Buffer.from(ca).toString('hex'), this._options.rejectUnauthorized !== false) : __thaw_tls_server_listen(hostname, Number(options.port), certHex, keyHex); if (outcome.indexOf('ok:') !== 0) { queueMicrotask(() => this.emit('error', new Error(outcome.substring(4)))); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: hostname.indexOf(':') >= 0 ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; var server = this; function pump() { if (!server._handle || !server.listening) return; if (server.maxConnections > 0 && server.connections >= server.maxConnections) { setTimeout(pump, 1); return; } var accepted = __thaw_tls_server_poll_accept(server._handle); if (accepted === 'err:pending') { setTimeout(pump, 1); return; } if (accepted.indexOf('ok:') !== 0) { server.emit('tlsClientError', new Error(accepted.substring(4))); if (server._handle) setTimeout(pump, 1); return; } var peer = accepted.split(':'); var socket = new TLSSocket(); socket._handle = Number(peer[1]); socket.authorized = true; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = peer[2].indexOf(':') >= 0 ? 'IPv6' : 'IPv4'; server.connections++; socket.once('close', function() { server.connections = Math.max(0, server.connections - 1); }); server.emit('secureConnection', socket); var incoming = __thaw_tls_server_read(socket._handle); if (incoming.indexOf('ok:') === 0) { var data = Buffer.from(incoming.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } socket.emit('end'); } else socket.emit('error', new Error(incoming.substring(4))); if (server._handle) setTimeout(pump, 0); } queueMicrotask(function() { server.emit('listening'); pump(); }); return this; };
             function alpnSpec(value) { if (value === undefined) return ''; var values = Array.isArray(value) ? value : [value]; return values.map(function(protocol) { return Buffer.from(protocol).toString('hex'); }).join(','); }
             TLSSocket.prototype.connect = function(options, listener) { if (typeof options === 'number') options = { port: options }; options = options || {}; if (typeof listener === 'function') this.once('secureConnect', listener); var host = String(options.host || 'localhost'), port = Number(options.port), servername = String(options.servername || host), ca = options.ca, cert = options.cert, key = options.key; if (Array.isArray(ca)) ca = ca[0]; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; this.connecting = true; var tlsOptions = (options.rejectUnauthorized === false ? '0|' : '1|') + alpnSpec(options.ALPNProtocols); var outcome = __thaw_tls_connect_with_options(host, port, servername, ca === undefined ? '' : Buffer.from(ca).toString('hex'), cert === undefined ? '' : Buffer.from(cert).toString('hex'), key === undefined ? '' : Buffer.from(key).toString('hex'), tlsOptions); this.connecting = false; this.servername = servername; if (outcome.indexOf('ok:') === 0) { var fields = outcome.split(':'); this._handle = Number(fields[1]); this.alpnProtocol = fields[2] ? Buffer.from(fields[2], 'hex').toString() : false; this.readable = true; this.writable = true; this.authorized = options.rejectUnauthorized !== false; this.authorizationError = this.authorized ? null : 'UNABLE_TO_VERIFY_LEAF_SIGNATURE'; queueMicrotask(() => { this.emit('connect'); this.emit('secureConnect'); }); } else { var error = new Error(outcome.substring(4)); error.code = 'ERR_TLS_CERT_ALTNAME_INVALID'; this.authorizationError = error.message; this.destroyed = true; queueMicrotask(() => { this.emit('error', error); this.emit('close', true); }); } return this; };
             var listenWithoutAlpn = Server.prototype.listen; Server.prototype.listen = function() { var serverOptions = this._options, protocols = alpnSpec(serverOptions.ALPNProtocols), ca = serverOptions.ca; if (Array.isArray(ca)) ca = ca[0]; var caHex = ca === undefined ? '' : Buffer.from(ca).toString('hex'), flags = (serverOptions.requestCert ? 1 : 0) | (serverOptions.rejectUnauthorized !== false ? 2 : 0); var basic = __thaw_tls_server_listen, withCa = __thaw_tls_server_listen_with_ca; __thaw_tls_server_listen = __thaw_tls_server_listen_with_ca = function(host, port, cert, key) { return __thaw_tls_server_listen_with_options(host, port, cert, key, caHex, flags, protocols); }; try { return listenWithoutAlpn.apply(this, arguments); } finally { __thaw_tls_server_listen = basic; __thaw_tls_server_listen_with_ca = withCa; } };
             var emitWithoutAlpn = Server.prototype.emit; Server.prototype.emit = function(name) { if (name === 'secureConnection' && arguments[1]) { var protocol = __thaw_tls_alpn(arguments[1]._handle); arguments[1].alpnProtocol = protocol ? Buffer.from(protocol, 'hex').toString() : false; } return emitWithoutAlpn.apply(this, arguments); };
             function rememberCertificates(socket) { if (!socket || !socket._handle) return; var peer = __thaw_tls_peer_certificate(socket._handle), local = __thaw_tls_local_certificate(socket._handle); socket._peerCertificateRaw = peer ? Buffer.from(peer, 'hex') : undefined; socket._localCertificateRaw = local ? Buffer.from(local, 'hex') : undefined; socket._peerCertificateMetadata = JSON.parse(__thaw_tls_peer_certificate_metadata(socket._handle)); socket._localCertificateMetadata = JSON.parse(__thaw_tls_local_certificate_metadata(socket._handle)); }
             function certificateInfo(raw, metadata) { if (!raw) return {}; var digest = __thaw_crypto_hash_hex('sha256', raw.toString('hex')).toUpperCase().replace(/(..)(?=.)/g, '$1:'); return Object.assign({ raw: Buffer.from(raw), fingerprint256: digest }, metadata || {}); }
             var connectWithoutCertificates = TLSSocket.prototype.connect; TLSSocket.prototype.connect = function() { var result = connectWithoutCertificates.apply(this, arguments); rememberCertificates(this); return result; }; TLSSocket.prototype.getPeerCertificate = function() { return certificateInfo(this._peerCertificateRaw, this._peerCertificateMetadata); }; TLSSocket.prototype.getCertificate = function() { return certificateInfo(this._localCertificateRaw, this._localCertificateMetadata); };
             var emitWithoutCertificates = Server.prototype.emit; Server.prototype.emit = function(name) { if (name === 'secureConnection') rememberCertificates(arguments[1]); return emitWithoutCertificates.apply(this, arguments); };
             module.exports.Server = Server; module.exports.createServer = createServer;
"#,
        )),
        "console" => Some(
            "module.exports = globalThis.console; module.exports.Console = globalThis.Console; module.exports.console = globalThis.console; module.exports.default = globalThis.console; module.exports.__esModule = true;\n",
        ),
        "constants" => Some(
            "var constants = globalThis.__thaw_fs_constants || (globalThis.__thaw_fs_constants = { F_OK: 0, X_OK: 1, W_OK: 2, R_OK: 4, O_RDONLY: 0, O_WRONLY: 1, O_RDWR: 2, O_CREAT: 64, O_EXCL: 128, O_NOCTTY: 256, O_TRUNC: 512, O_APPEND: 1024, O_DIRECTORY: 65536, O_NOFOLLOW: 131072, O_SYNC: 1052672, S_IFMT: 61440, S_IFREG: 32768, S_IFDIR: 16384, S_IFCHR: 8192, S_IFBLK: 24576, S_IFIFO: 4096, S_IFLNK: 40960, S_IFSOCK: 49152, COPYFILE_EXCL: 1, COPYFILE_FICLONE: 2, COPYFILE_FICLONE_FORCE: 4 }); module.exports = constants; module.exports.default = constants; module.exports.__esModule = true;\n",
        ),
        "crypto" => Some(
            "module.exports = globalThis.__thaw_crypto_module; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "perf_hooks" => Some(
            "function Histogram() { this.enabled = false; this.min = 0; this.max = 0; this.mean = 0; this.stddev = 0; this.exceeds = 0; this.count = 0; this.percentiles = new Map([[0, 0], [50, 0], [75, 0], [90, 0], [99, 0], [100, 0]]); }\n\
             Histogram.prototype.enable = function() { this.enabled = true; return true; }; Histogram.prototype.disable = function() { this.enabled = false; return true; }; Histogram.prototype.reset = function() { this.min = this.max = this.mean = this.stddev = this.exceeds = this.count = 0; }; Histogram.prototype.percentile = function() { return 0; }; Histogram.prototype.percentileBigInt = function() { return 0n; };\n\
             function monitorEventLoopDelay() { return new Histogram(); } function createHistogram() { return new Histogram(); }\n\
             var started = globalThis.performance.timeOrigin; globalThis.performance.nodeTiming = globalThis.performance.nodeTiming || { name: 'node', entryType: 'node', startTime: 0, duration: globalThis.performance.now(), nodeStart: 0, v8Start: 0, bootstrapComplete: 0, environment: 0, loopStart: 0, loopExit: -1, idleTime: 0 };\n\
             globalThis.performance.eventLoopUtilization = globalThis.performance.eventLoopUtilization || function(previous) { var active = globalThis.performance.now(); if (previous) active = Math.max(0, active - Number(previous.active || 0)); return { idle: 0, active: active, utilization: active === 0 ? 0 : 1 }; };\n\
             module.exports = { performance: globalThis.performance, PerformanceEntry: globalThis.PerformanceEntry, PerformanceMark: globalThis.PerformanceMark, PerformanceMeasure: globalThis.PerformanceMeasure, PerformanceObserver: globalThis.PerformanceObserver, PerformanceObserverEntryList: globalThis.PerformanceObserverEntryList, monitorEventLoopDelay: monitorEventLoopDelay, createHistogram: createHistogram, constants: { NODE_PERFORMANCE_GC_MAJOR: 4, NODE_PERFORMANCE_GC_MINOR: 1, NODE_PERFORMANCE_GC_INCREMENTAL: 8, NODE_PERFORMANCE_GC_WEAKCB: 16 }, timerify: globalThis.performance.timerify };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "v8" => Some(
            "function encode(root) { var seen = new Map(); var nodes = []; function visit(value) { if (value === undefined) return { t: 'u' }; if (typeof value === 'bigint') return { t: 'i', v: String(value) }; if (typeof value === 'number' && !Number.isFinite(value)) return { t: 'n', v: String(value) }; if (value === null || typeof value !== 'object') return { t: 'p', v: value }; if (seen.has(value)) return { t: 'r', v: seen.get(value) }; var id = nodes.length; seen.set(value, id); nodes.push(null); var node; if (Buffer.isBuffer(value)) node = { k: 'b', v: value.toString('base64') }; else if (value instanceof Date) node = { k: 'd', v: value.toISOString() }; else if (value instanceof RegExp) node = { k: 'x', v: value.source, f: value.flags, l: value.lastIndex }; else if (value instanceof Map) node = { k: 'm', v: Array.from(value, function(entry) { return [visit(entry[0]), visit(entry[1])]; }) }; else if (value instanceof Set) node = { k: 's', v: Array.from(value, visit) }; else if (Array.isArray(value)) node = { k: 'a', v: value.map(visit) }; else node = { k: 'o', v: Object.keys(value).map(function(key) { return [key, visit(value[key])]; }) }; nodes[id] = node; return { t: 'r', v: id }; } return JSON.stringify({ root: visit(root), nodes: nodes }); }\n\
             function decode(text) { var graph = JSON.parse(text); var values = new Array(graph.nodes.length); graph.nodes.forEach(function(node, index) { if (node.k === 'b') values[index] = Buffer.from(node.v, 'base64'); else if (node.k === 'd') values[index] = new Date(node.v); else if (node.k === 'x') values[index] = new RegExp(node.v, node.f); else if (node.k === 'm') values[index] = new Map(); else if (node.k === 's') values[index] = new Set(); else if (node.k === 'a') values[index] = []; else values[index] = {}; }); function read(value) { if (value.t === 'u') return undefined; if (value.t === 'i') return BigInt(value.v); if (value.t === 'n') return Number(value.v); if (value.t === 'p') return value.v; return values[value.v]; } graph.nodes.forEach(function(node, index) { var target = values[index]; if (node.k === 'm') node.v.forEach(function(entry) { target.set(read(entry[0]), read(entry[1])); }); else if (node.k === 's') node.v.forEach(function(entry) { target.add(read(entry)); }); else if (node.k === 'a') node.v.forEach(function(entry) { target.push(read(entry)); }); else if (node.k === 'o') node.v.forEach(function(entry) { target[entry[0]] = read(entry[1]); }); else if (node.k === 'x') target.lastIndex = node.l; }); return read(graph.root); }\n\
             function serialize(value) { return Buffer.from(encode(value), 'utf8'); } function deserialize(value) { return decode(Buffer.from(value).toString('utf8')); }\n\
             function getHeapStatistics() { return { total_heap_size: 0, total_heap_size_executable: 0, total_physical_size: 0, total_available_size: 0, used_heap_size: 0, heap_size_limit: Number.MAX_SAFE_INTEGER, malloced_memory: 0, peak_malloced_memory: 0, does_zap_garbage: 0, number_of_native_contexts: 1, number_of_detached_contexts: 0, total_global_handles_size: 0, used_global_handles_size: 0, external_memory: 0 }; }\n\
             function getHeapSpaceStatistics() { return []; } function getHeapCodeStatistics() { return { code_and_metadata_size: 0, bytecode_and_metadata_size: 0, external_script_source_size: 0, cpu_profiler_metadata_size: 0 }; } function cachedDataVersionTag() { return 0; } function setFlagsFromString() {}\n\
             module.exports = { serialize: serialize, deserialize: deserialize, getHeapStatistics: getHeapStatistics, getHeapSpaceStatistics: getHeapSpaceStatistics, getHeapCodeStatistics: getHeapCodeStatistics, cachedDataVersionTag: cachedDataVersionTag, setFlagsFromString: setFlagsFromString }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "vm" => Some(
            "var contexts = globalThis.__thaw_vm_contexts || (globalThis.__thaw_vm_contexts = new WeakSet()); function createContext(object, options) { var context = object === undefined ? {} : object; if ((typeof context !== 'object' && typeof context !== 'function') || context === null) throw new TypeError('contextObject must be an object'); contexts.add(context); if (!Object.prototype.hasOwnProperty.call(context, 'globalThis')) Object.defineProperty(context, 'globalThis', { value: context, configurable: true }); if (!Object.prototype.hasOwnProperty.call(context, 'global')) Object.defineProperty(context, 'global', { value: context, configurable: true }); return context; } function isContext(value) { return (typeof value === 'object' || typeof value === 'function') && value !== null && contexts.has(value); } function scopeFor(context) { return new Proxy(context, { has: function(target, key) { return key !== 'scope' && key !== 'source'; }, get: function(target, key) { if (key === Symbol.unscopables) return undefined; return key in target ? target[key] : globalThis[key]; }, set: function(target, key, value) { target[key] = value; return true; } }); } function runInContext(code, context, options) { if (!isContext(context)) throw new TypeError('contextifiedObject must be a vm.Context'); return Function('scope', 'source', 'with (scope) { return eval(source); }')(scopeFor(context), String(code)); } function runInNewContext(code, context, options) { return runInContext(code, createContext(context === undefined ? {} : context), options); } function runInThisContext(code, options) { return (0, eval)(String(code)); }\n\
             function Script(code, options) { if (!(this instanceof Script)) return new Script(code, options); this.code = String(code); this.filename = options && options.filename ? String(options.filename) : 'evalmachine.<anonymous>'; this.cachedDataRejected = false; this.sourceMapURL = undefined; Function(this.code); } Script.prototype.runInContext = function(context, options) { return runInContext(this.code, context, options); }; Script.prototype.runInNewContext = function(context, options) { return runInNewContext(this.code, context, options); }; Script.prototype.runInThisContext = function(options) { return runInThisContext(this.code, options); }; Script.prototype.createCachedData = function() { return Buffer.from(this.code, 'utf8'); }; function compileFunction(code, params, options) { params = params || []; var fn = Function.apply(null, params.concat(String(code))); if (options && options.filename) Object.defineProperty(fn, 'filename', { value: String(options.filename) }); return fn; } function measureMemory(options) { return Promise.resolve({ total: { jsMemoryEstimate: 0, jsMemoryRange: [0, 0] }, current: { jsMemoryEstimate: 0, jsMemoryRange: [0, 0] }, other: [] }); } function getDefaultContext() { return globalThis; }\n\
             module.exports = { Script: Script, createScript: function(code, options) { return new Script(code, options); }, createContext: createContext, isContext: isContext, runInContext: runInContext, runInNewContext: runInNewContext, runInThisContext: runInThisContext, compileFunction: compileFunction, measureMemory: measureMemory, constants: { USE_MAIN_CONTEXT_DEFAULT_LOADER: Symbol.for('vm_dynamic_import_main_context_default'), DONT_CONTEXTIFY: Symbol.for('vm_context_no_contextify') }, getDefaultContext: getDefaultContext }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "zlib" => Some(
            "var stream = require('node:stream'); function input(value) { return Buffer.isBuffer(value) ? value : Buffer.from(value); } function run(operation, format, value) { return Buffer.from(__thaw_zlib_hex(operation, format, input(value).toString('hex')), 'hex'); } function sync(operation, format) { return function(value) { return run(operation, format, value); }; } function async(syncFunction) { return function(value, options, callback) { if (typeof options === 'function') callback = options; try { var result = syncFunction(value, options); queueMicrotask(function() { callback(null, result); }); } catch (error) { queueMicrotask(function() { callback(error); }); } }; }\n\
             var gzipSync = sync('compress', 'gzip'), gunzipSync = sync('decompress', 'gzip'), deflateSync = sync('compress', 'deflate'), inflateSync = sync('decompress', 'deflate'), deflateRawSync = sync('compress', 'deflateRaw'), inflateRawSync = sync('decompress', 'deflateRaw'), brotliCompressSync = sync('compress', 'brotli'), brotliDecompressSync = sync('decompress', 'brotli');\n\
             function ZlibTransform(operation, format, options) { if (!(this instanceof ZlibTransform)) return new ZlibTransform(operation, format, options); var chunks = [], self = this; stream.Transform.call(this, { transform: function(chunk, encoding, callback) { self.bytesWritten += chunk.length; chunks.push(Buffer.from(chunk)); callback(); }, flush: function(callback) { try { callback(null, run(operation, format, Buffer.concat(chunks))); } catch (error) { callback(error); } } }); this.bytesWritten = 0; this._operation = operation; this._format = format; } ZlibTransform.prototype = Object.create(stream.Transform.prototype); ZlibTransform.prototype.constructor = ZlibTransform; ZlibTransform.prototype.flush = function(kind, callback) { if (typeof kind === 'function') callback = kind; if (callback) queueMicrotask(callback); }; ZlibTransform.prototype.close = function(callback) { this.destroy(); if (callback) queueMicrotask(callback); }; ZlibTransform.prototype.reset = function() { if (this.writableEnded) throw new Error('Cannot reset a finished zlib stream'); return this; };\n\
             function type(name, operation, format) { var Constructor = function(options) { if (!(this instanceof Constructor)) return new Constructor(options); ZlibTransform.call(this, operation, format, options); }; Constructor.prototype = Object.create(ZlibTransform.prototype); Constructor.prototype.constructor = Constructor; Object.defineProperty(Constructor, 'name', { value: name }); return Constructor; } var Gzip = type('Gzip', 'compress', 'gzip'), Gunzip = type('Gunzip', 'decompress', 'gzip'), Deflate = type('Deflate', 'compress', 'deflate'), Inflate = type('Inflate', 'decompress', 'deflate'), DeflateRaw = type('DeflateRaw', 'compress', 'deflateRaw'), InflateRaw = type('InflateRaw', 'decompress', 'deflateRaw'), BrotliCompress = type('BrotliCompress', 'compress', 'brotli'), BrotliDecompress = type('BrotliDecompress', 'decompress', 'brotli'); function factory(Constructor) { return function(options) { return new Constructor(options); }; }\n\
             module.exports = { gzipSync: gzipSync, gunzipSync: gunzipSync, deflateSync: deflateSync, inflateSync: inflateSync, deflateRawSync: deflateRawSync, inflateRawSync: inflateRawSync, brotliCompressSync: brotliCompressSync, brotliDecompressSync: brotliDecompressSync, gzip: async(gzipSync), gunzip: async(gunzipSync), deflate: async(deflateSync), inflate: async(inflateSync), deflateRaw: async(deflateRawSync), inflateRaw: async(inflateRawSync), brotliCompress: async(brotliCompressSync), brotliDecompress: async(brotliDecompressSync), Gzip: Gzip, Gunzip: Gunzip, Deflate: Deflate, Inflate: Inflate, DeflateRaw: DeflateRaw, InflateRaw: InflateRaw, BrotliCompress: BrotliCompress, BrotliDecompress: BrotliDecompress, createGzip: factory(Gzip), createGunzip: factory(Gunzip), createDeflate: factory(Deflate), createInflate: factory(Inflate), createDeflateRaw: factory(DeflateRaw), createInflateRaw: factory(InflateRaw), createBrotliCompress: factory(BrotliCompress), createBrotliDecompress: factory(BrotliDecompress), constants: { Z_OK: 0, Z_STREAM_END: 1, Z_DEFAULT_COMPRESSION: -1, Z_NO_FLUSH: 0, Z_SYNC_FLUSH: 2, Z_FINISH: 4, BROTLI_OPERATION_PROCESS: 0, BROTLI_OPERATION_FLUSH: 1, BROTLI_OPERATION_FINISH: 2 } }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "worker_threads" => Some(concat!(
            "var workerStream = require('node:stream'), WorkerReadable = workerStream.Readable, WorkerWritable = workerStream.Writable; var environmentData = globalThis.__thaw_worker_environment_data || (globalThis.__thaw_worker_environment_data = new Map()); var SHARE_ENV = Symbol.for('nodejs.worker_threads.SHARE_ENV');\n\
             function receiveMessageOnPort(port) { if (!port || !Array.isArray(port.__thawQueue)) throw new TypeError('port must be a MessagePort or BroadcastChannel'); var record = port.__thawQueue.shift(); return record ? { message: record.data } : undefined; }\n\
             function setEnvironmentData(key, value) { environmentData.set(key, structuredClone(value)); } function getEnvironmentData(key) { var value = environmentData.get(key); return value === undefined ? undefined : structuredClone(value); }\n\
             function moveMessagePortToContext(port, context) { if (!(port instanceof MessagePort)) throw new TypeError('port must be a MessagePort'); var contexts = globalThis.__thaw_vm_contexts; if (!contexts || !contexts.has(context)) throw new TypeError('contextifiedSandbox must be a vm.Context'); return port.__thawTransfer(); } var untransferable = globalThis.__thaw_untransferable_objects || (globalThis.__thaw_untransferable_objects = new WeakSet()), uncloneable = globalThis.__thaw_uncloneable_objects || (globalThis.__thaw_uncloneable_objects = new WeakSet()); function markAsUntransferable(value) { untransferable.add(value); } function markAsUncloneable(value) { uncloneable.add(value); } function isMarkedAsUntransferable(value) { return ((typeof value === 'object' && value !== null) || typeof value === 'function') && untransferable.has(value); } function cloneError(message) { return new DOMException(message, 'DataCloneError'); } function assertTransferList(transfer) { for (var value of transfer || []) if (isMarkedAsUntransferable(value)) throw cloneError('Object is marked as untransferable'); } function assertCloneable(value, seen) { if ((typeof value !== 'object' && typeof value !== 'function') || value === null) return; if (uncloneable.has(value)) throw cloneError('Object is marked as uncloneable'); if (seen.has(value)) return; seen.add(value); if (value instanceof Map) value.forEach(function(entry, key) { assertCloneable(key, seen); assertCloneable(entry, seen); }); else if (value instanceof Set) value.forEach(function(entry) { assertCloneable(entry, seen); }); else for (var key of Object.keys(value)) assertCloneable(value[key], seen); } if (!globalThis.__thaw_worker_clone_guards) { globalThis.__thaw_worker_clone_guards = true; var cloneWithoutWorkerGuards = globalThis.structuredClone; globalThis.structuredClone = function(value, options) { assertTransferList(options && options.transfer); assertCloneable(value, new WeakSet()); return cloneWithoutWorkerGuards(value, options); }; var postWithoutWorkerGuards = MessagePort.prototype.postMessage; MessagePort.prototype.postMessage = function(value, transfer) { assertTransferList(transfer); assertCloneable(value, new WeakSet()); return postWithoutWorkerGuards.call(this, value, transfer); }; }\n\
             var threadReceivers = globalThis.__thaw_worker_thread_receivers || (globalThis.__thaw_worker_thread_receivers = new Map()); function threadMessagingError(code, message, cause) { var error = new Error(message); error.code = code; if (cause !== undefined) error.cause = cause; return error; } function dispatchThreadMessage(source, target, value, transferList, timeout) { source = Number(source); target = Number(target); if (source === target) return Promise.reject(threadMessagingError('ERR_WORKER_MESSAGING_SAME_THREAD', 'Cannot send a message to the same thread')); var receiver = threadReceivers.get(target); if (!receiver) return Promise.reject(threadMessagingError('ERR_WORKER_MESSAGING_FAILED', 'The destination thread is not available')); var cloned; try { cloned = structuredClone(value, { transfer: transferList || [] }); } catch (error) { return Promise.reject(error); } return new Promise(function(resolve, reject) { var settled = false, timer; if (timeout !== undefined && Number(timeout) >= 0) timer = setTimeout(function() { if (!settled) { settled = true; reject(threadMessagingError('ERR_WORKER_MESSAGING_TIMEOUT', 'The destination thread did not process the message')); } }, Number(timeout)); queueMicrotask(function() { if (settled) return; try { if (!receiver(cloned, source)) throw threadMessagingError('ERR_WORKER_MESSAGING_FAILED', 'The destination thread has no workerMessage listener'); settled = true; if (timer) clearTimeout(timer); resolve(); } catch (error) { settled = true; if (timer) clearTimeout(timer); reject(error.code ? error : threadMessagingError('ERR_WORKER_MESSAGING_ERRORED', 'The destination thread listener threw', error)); } }); }); } function postMessageToThread(target, value, transferList, timeout) { return dispatchThreadMessage(0, target, value, transferList, timeout); } threadReceivers.set(0, function(value, source) { var process = globalThis.process; if (!process || !process.listenerCount || process.listenerCount('workerMessage') === 0) return false; process.emit('workerMessage', value, source); return true; });\n\
             module.exports = { isMainThread: true, threadId: 0, threadName: '', workerData: null, parentPort: null, resourceLimits: {}, MessageChannel: MessageChannel, MessagePort: MessagePort, BroadcastChannel: globalThis.BroadcastChannel, receiveMessageOnPort: receiveMessageOnPort, setEnvironmentData: setEnvironmentData, getEnvironmentData: getEnvironmentData, postMessageToThread: postMessageToThread, moveMessagePortToContext: moveMessagePortToContext, markAsUntransferable: markAsUntransferable, markAsUncloneable: markAsUncloneable, isMarkedAsUntransferable: isMarkedAsUntransferable, SHARE_ENV: SHARE_ENV }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
            r#"
             var nextWorkerId = globalThis.__thaw_next_worker_id || 1; function runWorkerSource(source, context) { context.globalThis = context; context.global = context; var intrinsicNames = new Set(['Error','TypeError','RangeError','ReferenceError','SyntaxError','URIError','EvalError','AggregateError','DOMException','Object','Function','Array','Number','String','Boolean','BigInt','Symbol','Date','RegExp','Map','Set','WeakMap','WeakSet','Promise','Proxy','Reflect','JSON','Math','Intl','ArrayBuffer','SharedArrayBuffer','DataView','Uint8Array','Uint16Array','Uint32Array','Int8Array','Int16Array','Int32Array','Float32Array','Float64Array','BigInt64Array','BigUint64Array','Atomics','URL','URLSearchParams','TextEncoder','TextDecoder']); var scope = new Proxy(context, { has: function(target, key) { return key !== 'scope' && key !== 'source'; }, get: function(target, key) { if (key === Symbol.unscopables) return undefined; return key in target ? target[key] : intrinsicNames.has(key) ? globalThis[key] : undefined; }, set: function(target, key, value) { target[key] = value; return true; } }); return Function('scope', 'source', 'with (scope) { return eval(source); }')(scope, String(source)); }
             function createWorkerProcess(parent) { var process = Object.create(parent), listeners = new Map(); function entries(name) { name = String(name); var list = listeners.get(name); if (!list) { list = []; listeners.set(name, list); } return list; } process.on = process.addListener = function(name, listener) { entries(name).push({ listener: listener, once: false }); return process; }; process.once = function(name, listener) { entries(name).push({ listener: listener, once: true }); return process; }; process.off = process.removeListener = function(name, listener) { listeners.set(String(name), entries(name).filter(function(entry) { return entry.listener !== listener; })); return process; }; process.emit = function(name) { var key = String(name), values = entries(key).slice(), args = Array.prototype.slice.call(arguments, 1); values.forEach(function(entry) { if (entry.once) process.off(key, entry.listener); entry.listener.apply(process, args); }); return values.length > 0; }; process.listenerCount = function(name) { return entries(name).length; }; return process; }
             var nativeWorkers = globalThis.__thaw_native_workers || (globalThis.__thaw_native_workers = new Map()), nativeWorkersByThread = globalThis.__thaw_native_workers_by_thread || (globalThis.__thaw_native_workers_by_thread = new Map()), nativeDirectPending = new Map(), nativeDirectRequest = 1, nativePorts = new Map(), nativePortSequence = 1, nativeDecodeWorker = null; function configureNativePortBridge(port, entry) { port.__thawSchedule = function() { while (port.__thawQueue.length) { var record = port.__thawQueue.shift(); if (entry.worker) __thaw_worker_port(entry.worker._nativeHandle, entry.id, __thaw_worker_encode(record.data)); } }; } function prepareNativePorts(transfer) { var entries = []; for (var port of transfer || []) { if (!(port instanceof MessagePort) || port.__thawHostPortId) continue; var id = 'p:' + (nativePortSequence++), moved = port.__thawTransfer(), entry = { id: id, receiver: moved.__thawPeer, worker: null }; port.__thawHostPortId = id; moved.__thawHostPortId = id; configureNativePortBridge(moved, entry); nativePorts.set(id, entry); entries.push(entry); } return entries; } function bindNativePorts(entries, worker) { entries.forEach(function(entry) { entry.worker = worker; }); } function decodeNativeWorkerValue(worker, payload) { var previous = nativeDecodeWorker; nativeDecodeWorker = worker; try { return __thaw_worker_decode(payload); } finally { nativeDecodeWorker = previous; } } globalThis.__thaw_create_worker_port = function(id) { id = String(id); var existing = nativePorts.get(id); if (existing) return existing.receiver; if (!nativeDecodeWorker) throw new DOMException('MessagePort has no Worker owner', 'DataCloneError'); var port = new MessagePort(), entry = { id: id, receiver: port, worker: nativeDecodeWorker }; port.__thawHostPortId = id; port.postMessage = function(value, transfer) { var entries = prepareNativePorts(transfer), payload = __thaw_worker_encode(value); detachNativeTransfers(transfer); bindNativePorts(entries, entry.worker); __thaw_worker_port(entry.worker._nativeHandle, id, payload); }; nativePorts.set(id, entry); return port; }; function enableNativeSharedEnvironment(parentProcess) { if (parentProcess.__thawSharedEnvEnabled) return; __thaw_shared_env_init(JSON.stringify(parentProcess.env || {})); var proxy = new Proxy({}, { get: function(target, key) { return typeof key === 'symbol' ? target[key] : __thaw_shared_env_get(String(key)); }, set: function(target, key, value) { if (typeof key === 'symbol') target[key] = value; else __thaw_shared_env_set(String(key), String(value)); return true; }, deleteProperty: function(target, key) { return typeof key === 'symbol' ? delete target[key] : __thaw_shared_env_delete(String(key)); }, ownKeys: function() { return JSON.parse(__thaw_shared_env_keys()); }, getOwnPropertyDescriptor: function(target, key) { if (typeof key === 'symbol') return Object.getOwnPropertyDescriptor(target, key); var value = __thaw_shared_env_get(String(key)); return value === undefined ? undefined : { value: value, writable: true, enumerable: true, configurable: true }; } }); parentProcess.env = proxy; Object.defineProperty(parentProcess, '__thawSharedEnvEnabled', { value: true, configurable: true }); } function nativeTransferListSupported(transfer) { return Array.from(transfer || []).every(function(value) { return value instanceof ArrayBuffer || value instanceof MessagePort; }); } function canUseNativeWorker(source, options) { return typeof __thaw_worker_spawn === 'function' && nativeTransferListSupported(options.transferList); } function detachNativeTransfers(transfer) { assertTransferList(transfer); var seen = new Set(); for (var value of transfer || []) { if (seen.has(value)) throw new DOMException('transfer list contains duplicate values', 'DataCloneError'); seen.add(value); if (value instanceof ArrayBuffer) __thaw_detach_array_buffer(value); } } function startNativeWorker(worker, source, options) { if (!canUseNativeWorker(source, options)) return false; worker._terminated = false; worker._exited = false; var parentProcess = globalThis.process || {}; if (options.env === SHARE_ENV) enableNativeSharedEnvironment(parentProcess); var config = { threadName: options.name === undefined ? '' : String(options.name), resourceLimits: Object.assign({}, options.resourceLimits || {}), env: Object.assign({}, (options.env === undefined || options.env === SHARE_ENV) ? (parentProcess.env || {}) : options.env), shareEnv: options.env === SHARE_ENV, argv: Array.from(parentProcess.argv || []).concat(Array.from(options.argv || [], String)), execArgv: options.execArgv === undefined ? Array.from(parentProcess.execArgv || []) : Array.from(options.execArgv || [], String), stdin: !!options.stdin, stdout: !!options.stdout, stderr: !!options.stderr }, portEntries = prepareNativePorts(options.transferList), workerDataPayload = __thaw_worker_encode(options.workerData === undefined ? null : options.workerData); detachNativeTransfers(options.transferList); worker._nativeHandle = __thaw_worker_spawn(String(globalThis.__thaw_worker_bundle_source || ''), String(source), workerDataPayload, JSON.stringify(config), worker.threadId); bindNativePorts(portEntries, worker); worker.stdin = options.stdin ? new WorkerWritable({ write: function(chunk, encoding, callback) { __thaw_worker_stdin(worker._nativeHandle, String(chunk), false); callback(); }, final: function(callback) { __thaw_worker_stdin(worker._nativeHandle, '', true); callback(); } }) : null; worker.stdout = options.stdout ? new WorkerReadable() : null; worker.stderr = options.stderr ? new WorkerReadable() : null; nativeWorkers.set(worker._nativeHandle, worker); nativeWorkersByThread.set(worker.threadId, worker); return true; } function directErrorText(code, message) { return String(code) + ':' + String(message); } function settleNativeDirect(request, errorText) { var pending = nativeDirectPending.get(Number(request)); if (!pending) return; nativeDirectPending.delete(Number(request)); if (pending.timer) clearTimeout(pending.timer); if (!errorText) pending.resolve(); else { var separator = errorText.indexOf(':'), error = new Error(separator < 0 ? errorText : errorText.slice(separator + 1)); error.code = separator < 0 ? errorText : errorText.slice(0, separator); pending.reject(error); } } function routeNativeDirect(event) { if (event.target === 0) { var errorText = ''; try { if (!process.listenerCount || process.listenerCount('workerMessage') === 0) errorText = directErrorText('ERR_WORKER_MESSAGING_FAILED', 'The destination thread has no workerMessage listener'); else process.emit('workerMessage', __thaw_worker_decode(event.payload), event.source); } catch (error) { errorText = directErrorText('ERR_WORKER_MESSAGING_ERRORED', error.message); } __thaw_worker_direct_result(event.handle, event.request, errorText); return; } var target = nativeWorkersByThread.get(event.target); if (!target || !__thaw_worker_route_direct(event.handle, target._nativeHandle, event.payload, event.source, event.request)) __thaw_worker_direct_result(event.handle, event.request, directErrorText('ERR_WORKER_MESSAGING_FAILED', 'The destination thread is not available')); } function nativePostMessageToThread(target, value, transferList, timeout) { target = Number(target); if (target === 0) return Promise.reject(threadMessagingError('ERR_WORKER_MESSAGING_SAME_THREAD', 'Cannot send a message to the same thread')); var worker = nativeWorkersByThread.get(target); if (!worker) return dispatchThreadMessage(0, target, value, transferList, timeout); var cloned; try { cloned = structuredClone(value, { transfer: transferList || [] }); } catch (error) { return Promise.reject(error); } var request = nativeDirectRequest++, payload = __thaw_worker_encode(cloned); return new Promise(function(resolve, reject) { var timer; if (timeout !== undefined && Number(timeout) >= 0) timer = setTimeout(function() { if (nativeDirectPending.delete(request)) reject(threadMessagingError('ERR_WORKER_MESSAGING_TIMEOUT', 'The destination thread did not process the message')); }, Number(timeout)); nativeDirectPending.set(request, { resolve: resolve, reject: reject, timer: timer }); if (!__thaw_worker_parent_direct(worker._nativeHandle, payload, request)) { nativeDirectPending.delete(request); if (timer) clearTimeout(timer); reject(threadMessagingError('ERR_WORKER_MESSAGING_FAILED', 'The destination thread is not available')); } }); } postMessageToThread = nativePostMessageToThread; if (!globalThis.__thaw_native_worker_poll_installed) { globalThis.__thaw_native_worker_poll_installed = true; var previousPlatformPoll = globalThis.__thaw_poll_platform_events; globalThis.__thaw_poll_platform_events = function() { if (previousPlatformPoll) previousPlatformPoll(); var events = JSON.parse(__thaw_worker_poll()); events.forEach(function(event) { var worker = nativeWorkers.get(event.handle); if (!worker) return; if (event.type === 'online') worker.emit('online'); else if (event.type === 'message') worker.emit('message', decodeNativeWorkerValue(worker, event.payload)); else if (event.type === 'stdout' && worker.stdout) worker.stdout.push(event.payload); else if (event.type === 'stderr' && worker.stderr) worker.stderr.push(event.payload); else if (event.type === 'port') { var portEntry = nativePorts.get(String(event.port)); if (portEntry) { portEntry.receiver.__thawQueue.push({ data: decodeNativeWorkerValue(worker, event.payload), ports: [] }); portEntry.receiver.__thawSchedule(); } } else if (event.type === 'direct') routeNativeDirect(event); else if (event.type === 'directResult') settleNativeDirect(event.request, event.error || ''); else if (event.type === 'error') worker.emit('error', new Error(event.error)); else if (event.type === 'exit') { nativeWorkers.delete(event.handle); nativeWorkersByThread.delete(worker.threadId); worker._finish(event.code); } }); }; }
             function Worker(filename, options) { if (!(this instanceof Worker)) return new Worker(filename, options); options = options || {}; if (!options.eval) throw new Error('Thaw Worker currently requires { eval: true }'); this._events = Object.create(null); this.threadId = nextWorkerId++; this.threadName = options.name === undefined ? '' : String(options.name); globalThis.__thaw_next_worker_id = nextWorkerId; this.resourceLimits = Object.assign({}, options.resourceLimits || {}); this.performance = { eventLoopUtilization: function() { return { idle: 0, active: 0, utilization: 0 }; } }; var childStdin = new WorkerReadable(), worker = this; this.stdin = options.stdin ? new WorkerWritable({ write: function(chunk, encoding, callback) { childStdin.push(chunk); callback(); }, final: function(callback) { childStdin.push(null); callback(); } }) : null; this.stdout = options.stdout ? new WorkerReadable() : null; this.stderr = options.stderr ? new WorkerReadable() : null; this._terminated = false; this._exited = false; var channel = new MessageChannel(); this._port = channel.port1; this._workerPort = channel.port2; this._port.on('message', function(value) { worker.emit('message', value); }); this._port.on('messageerror', function(error) { worker.emit('messageerror', error); }); this._workerPort.on('close', function() { worker._finish(0); }); var data = options.workerData === undefined ? undefined : structuredClone(options.workerData, { transfer: options.transferList || [] }); queueMicrotask(function() { if (worker._terminated) return; worker.emit('online'); var childModule = { isMainThread: false, threadId: worker.threadId, threadName: worker.threadName, workerData: data, parentPort: worker._workerPort, resourceLimits: worker.resourceLimits, MessageChannel: MessageChannel, MessagePort: MessagePort, BroadcastChannel: globalThis.BroadcastChannel, receiveMessageOnPort: receiveMessageOnPort, setEnvironmentData: setEnvironmentData, getEnvironmentData: getEnvironmentData, SHARE_ENV: SHARE_ENV }; var parentProcess = globalThis.process || {}, workerProcess = createWorkerProcess(parentProcess), parentEnvironment = parentProcess.env || {}; workerProcess.env = options.env === SHARE_ENV ? parentEnvironment : Object.assign({}, options.env === undefined ? parentEnvironment : options.env); workerProcess.argv = Array.from(parentProcess.argv || []).concat(Array.from(options.argv || [], String)); workerProcess.execArgv = options.execArgv === undefined ? Array.from(parentProcess.execArgv || []) : Array.from(options.execArgv || [], String); childModule.postMessageToThread = function(target, value, transferList, timeout) { return dispatchThreadMessage(worker.threadId, target, value, transferList, timeout); }; threadReceivers.set(worker.threadId, function(value, source) { if (workerProcess.listenerCount('workerMessage') === 0) return false; workerProcess.emit('workerMessage', value, source); return true; }); workerProcess.stdin = childStdin; workerProcess.stdout = worker.stdout ? new WorkerWritable({ write: function(chunk, encoding, callback) { worker.stdout.push(chunk); callback(); } }) : parentProcess.stdout; workerProcess.stderr = worker.stderr ? new WorkerWritable({ write: function(chunk, encoding, callback) { worker.stderr.push(chunk); callback(); } }) : parentProcess.stderr; var workerConsole = Object.create(console); function writeConsole(stream, args) { if (stream && stream.write) stream.write(Array.prototype.map.call(args, String).join(' ') + '\n'); } if (worker.stdout) workerConsole.log = workerConsole.info = function() { writeConsole(workerProcess.stdout, arguments); }; if (worker.stderr) workerConsole.warn = workerConsole.error = function() { writeConsole(workerProcess.stderr, arguments); }; var scriptModule = { exports: {} }; var context = { eval: eval, console: workerConsole, Buffer: Buffer, structuredClone: structuredClone, MessageChannel: MessageChannel, MessagePort: MessagePort, BroadcastChannel: globalThis.BroadcastChannel, setTimeout: setTimeout, clearTimeout: clearTimeout, setInterval: setInterval, clearInterval: clearInterval, queueMicrotask: queueMicrotask, process: workerProcess, module: scriptModule, exports: scriptModule.exports, __thaw_bundle_create_require: globalThis.__thaw_bundle_create_require, __thaw_worker_module: childModule, require: function(name) { if (name === 'worker_threads' || name === 'node:worker_threads') return childModule; return require(name); } }; try { runWorkerSource(String(filename), context); if (!(worker._workerPort.__thawNodeListeners.get('message') || []).length && workerProcess.listenerCount('workerMessage') === 0) worker._workerPort.close(); } catch (error) { worker.emit('error', error); worker._finish(1); } }); }
             Worker.prototype.on = Worker.prototype.addListener = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; }; Worker.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; }; Worker.prototype.prependListener = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).unshift({ listener: listener, once: false }); return this; }; Worker.prototype.prependOnceListener = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).unshift({ listener: listener, once: true }); return this; }; Worker.prototype.off = Worker.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; Worker.prototype.removeAllListeners = function(name) { if (name === undefined) this._events = Object.create(null); else delete this._events[String(name)]; return this; }; Worker.prototype.listeners = Worker.prototype.rawListeners = function(name) { return (this._events[String(name)] || []).map(function(entry) { return entry.listener; }); }; Worker.prototype.listenerCount = function(name, listener) { return (this._events[String(name)] || []).filter(function(entry) { return listener === undefined || entry.listener === listener; }).length; }; Worker.prototype.eventNames = function() { return Object.keys(this._events).filter(function(name) { return this._events[name].length; }, this); }; Worker.prototype.setMaxListeners = function(count) { this._maxListeners = Number(count); return this; }; Worker.prototype.getMaxListeners = function() { return this._maxListeners === undefined ? 10 : this._maxListeners; }; Worker.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; }; Worker.prototype.postMessage = function(value, transfer) { if (!this._terminated) this._port.postMessage(value, transfer); }; Worker.prototype._finish = function(code) { if (this._exited) return; this._exited = true; if (this.stdout) this.stdout.push(null); if (this.stderr) this.stderr.push(null); if (this.stdin && !this.stdin.writableEnded) this.stdin.end(); var worker = this; queueMicrotask(function() { worker.emit('exit', Number(code)); }); }; Worker.prototype.terminate = function() { if (!this._terminated) { this._terminated = true; this._workerPort.close(); this._port.close(); this._finish(1); } return Promise.resolve(1); }; Worker.prototype.ref = function() { this._port.ref(); return this; }; Worker.prototype.unref = function() { this._port.unref(); return this; }; Worker.prototype.getHeapSnapshot = function() { return Promise.resolve({}); };
             function exitCompletion(worker) { if (!worker._exitPromise) worker._exitPromise = new Promise(function(resolve) { worker._resolveExit = resolve; }); return worker._exitPromise; } var finishWithoutCompletion = Worker.prototype._finish; Worker.prototype._finish = function(code) { if (this._exited) return exitCompletion(this); var completion = exitCompletion(this), worker = this, exitCode = Number(code); threadReceivers.delete(worker.threadId); finishWithoutCompletion.call(this, exitCode); queueMicrotask(function() { worker._resolveExit(exitCode); }); return completion; }; Worker.prototype.terminate = function() { var completion = exitCompletion(this); if (!this._terminated && !this._exited) { this._terminated = true; this._finish(1); this._workerPort.close(); this._port.close(); } return completion; }; if (Symbol.asyncDispose) Worker.prototype[Symbol.asyncDispose] = Worker.prototype.terminate;
             var inProcessPostMessage = Worker.prototype.postMessage, inProcessTerminate = Worker.prototype.terminate, inProcessRef = Worker.prototype.ref, inProcessUnref = Worker.prototype.unref; Worker.prototype.postMessage = function(value, transfer) { if (!this._nativeHandle) return inProcessPostMessage.call(this, value, transfer); if (!nativeTransferListSupported(transfer)) throw new DOMException('value is not transferable', 'DataCloneError'); if (!this._terminated) { var portEntries = prepareNativePorts(transfer), payload = __thaw_worker_encode(value); detachNativeTransfers(transfer); bindNativePorts(portEntries, this); __thaw_worker_send(this._nativeHandle, payload); } }; Worker.prototype.terminate = function() { if (!this._nativeHandle) return inProcessTerminate.call(this); var completion = exitCompletion(this); if (!this._terminated && !this._exited) { this._terminated = true; __thaw_worker_terminate(this._nativeHandle); } return completion; }; Worker.prototype.ref = function() { return this._nativeHandle ? this : inProcessRef.call(this); }; Worker.prototype.unref = function() { return this._nativeHandle ? this : inProcessUnref.call(this); }; if (Symbol.asyncDispose) Worker.prototype[Symbol.asyncDispose] = Worker.prototype.terminate;
             function workerNotRunning() { return threadMessagingError('ERR_WORKER_NOT_RUNNING', 'Worker instance is not running'); } Worker.prototype.cpuUsage = function(previous) { if (this._exited) return Promise.reject(workerNotRunning()); var current = { user: 0, system: 0 }; if (previous) { current.user = Math.max(0, current.user - Number(previous.user || 0)); current.system = Math.max(0, current.system - Number(previous.system || 0)); } return Promise.resolve(current); }; Worker.prototype.getHeapStatistics = function() { if (this._exited) return Promise.reject(workerNotRunning()); return Promise.resolve({ total_heap_size: 0, total_heap_size_executable: 0, total_physical_size: 0, total_available_size: 0, used_heap_size: 0, heap_size_limit: Number.MAX_SAFE_INTEGER, malloced_memory: 0, peak_malloced_memory: 0, does_zap_garbage: 0, number_of_native_contexts: 1, number_of_detached_contexts: 0, total_global_handles_size: 0, used_global_handles_size: 0, external_memory: 0 }); }; Worker.prototype.getHeapSnapshot = function() { if (this._exited) return Promise.reject(workerNotRunning()); var snapshot = new WorkerReadable(); queueMicrotask(function() { snapshot.push(JSON.stringify({ snapshot: { meta: {}, node_count: 0, edge_count: 0 }, nodes: [], edges: [], strings: [] })); snapshot.push(null); }); return Promise.resolve(snapshot); }; Worker.prototype.startCpuProfile = function() { if (this._exited) return Promise.reject(workerNotRunning()); var stopped = false; return Promise.resolve({ stop: function() { if (stopped) return Promise.reject(new Error('CPU profile has already been stopped')); stopped = true; return Promise.resolve({ nodes: [], startTime: 0, endTime: 0, samples: [], timeDeltas: [] }); } }); };
             var EvalWorker = Worker; function createNativeWorker(source, options) { if (!canUseNativeWorker(source, options)) return null; var worker = Object.create(EvalWorker.prototype); worker._events = Object.create(null); worker.threadId = nextWorkerId++; worker.threadName = options.name === undefined ? '' : String(options.name); globalThis.__thaw_next_worker_id = nextWorkerId; worker.resourceLimits = Object.assign({}, options.resourceLimits || {}); worker.performance = { eventLoopUtilization: function() { return { idle: 0, active: 0, utilization: 0 }; } }; return startNativeWorker(worker, source, options) ? worker : null; } function decodeWorkerDataUrl(value) { var text = String(value); if (!text.startsWith('data:')) return null; var comma = text.indexOf(','); if (comma < 0) throw new TypeError('Invalid Worker data URL'); var metadata = text.slice(5, comma).toLowerCase(), payload = text.slice(comma + 1), parts = metadata.split(';'), mediaType = parts[0] || 'text/plain'; if (mediaType !== 'text/javascript' && mediaType !== 'application/javascript') throw new TypeError('Worker data URL must contain JavaScript'); try { return parts.indexOf('base64') >= 0 ? Buffer.from(payload, 'base64').toString('utf8') : decodeURIComponent(payload); } catch (error) { throw new TypeError('Invalid Worker data URL payload'); } } function readRuntimeWorkerFile(value) { var filename = String(value); if (filename.startsWith('file:')) { try { filename = decodeURIComponent(new URL(filename).pathname); } catch (error) { throw new TypeError('Invalid Worker file URL'); } } var source = __thaw_worker_read_source(filename), slash = filename.lastIndexOf('/'), dirname = slash < 0 ? '.' : filename.slice(0, slash), prefix = '(function(){ var __thawBuiltinRequire = require, __thawRuntimeCache = Object.create(null); function __thawNormalize(path) { var absolute = path.charAt(0) === "/", output = []; path.split("/").forEach(function(part) { if (!part || part === ".") return; if (part === "..") output.pop(); else output.push(part); }); return (absolute ? "/" : "") + output.join("/"); } function __thawLoad(base, request) { if (request.slice(0, 2) !== "./" && request.slice(0, 3) !== "../" && request.charAt(0) !== "/") return __thawBuiltinRequire(request); var target = __thawNormalize(request.charAt(0) === "/" ? request : base + "/" + request), candidates = /\\.[^/]+$/.test(target) ? [target] : [target, target + ".js", target + ".json", target + "/index.js", target + "/index.json"], loaded, filename; for (var index = 0; index < candidates.length; index++) { try { loaded = __thaw_worker_read_source(candidates[index]); filename = candidates[index]; break; } catch (error) {} } if (filename === undefined) throw new Error("Cannot find module \'" + request + "\'"); if (__thawRuntimeCache[filename]) return __thawRuntimeCache[filename].exports; var module = __thawRuntimeCache[filename] = { exports: {} }, slash = filename.lastIndexOf("/"), dirname = slash < 0 ? "." : filename.slice(0, slash), localRequire = function(name) { return __thawLoad(dirname, String(name)); }; if (/\\.json$/.test(filename)) module.exports = JSON.parse(loaded); else Function("module", "exports", "require", "__filename", "__dirname", loaded)(module, module.exports, localRequire, filename, dirname); return module.exports; } require = function(request) { return __thawLoad(__dirname, String(request)); }; })();\n'; return 'var __filename = ' + JSON.stringify(filename) + '; var __dirname = ' + JSON.stringify(dirname) + ';\n' + prefix + source; } Worker = function Worker(filename, options) { options = options || {}; var source = options.eval ? String(filename) : decodeWorkerDataUrl(filename); if (source === null) source = readRuntimeWorkerFile(filename); var nativeWorker = createNativeWorker(source, options); if (nativeWorker) return nativeWorker; var workerOptions = options.eval ? options : Object.assign({}, options, { eval: true }); return new EvalWorker(source, workerOptions); }; Worker.prototype = EvalWorker.prototype;
             var readRuntimeWorkerFileWithoutDirectoryResolution = readRuntimeWorkerFile; readRuntimeWorkerFile = function(value) { var source = readRuntimeWorkerFileWithoutDirectoryResolution(value), basicCandidates = 'candidates = /\\.[^/]+$/.test(target) ? [target] : [target, target + ".js", target + ".json", target + "/index.js", target + "/index.json"], loaded, filename;', nodeCandidates = 'candidates = /\\.[^/]+$/.test(target) ? [target] : [target, target + ".js", target + ".cjs", target + ".json", target + "/index.js", target + "/index.cjs", target + "/index.json"], loaded, filename; if (!/\\.[^/]+$/.test(target)) { try { var manifest = JSON.parse(__thaw_worker_read_source(target + "/package.json")); if (typeof manifest.main === "string" && manifest.main) return __thawLoad(target, "./" + manifest.main); } catch (error) {} }'; return source.replace(basicCandidates, nodeCandidates); };
             var readRuntimeWorkerFileWithoutBareResolution = readRuntimeWorkerFile; readRuntimeWorkerFile = function(value) { var source = readRuntimeWorkerFileWithoutBareResolution(value), builtinOnly = 'if (request.slice(0, 2) !== "./" && request.slice(0, 3) !== "../" && request.charAt(0) !== "/") return __thawBuiltinRequire(request);', nodeModulesFallback = 'if (request.slice(0, 2) !== "./" && request.slice(0, 3) !== "../" && request.charAt(0) !== "/") { try { return __thawBuiltinRequire(request); } catch (builtinError) { var parts = request.split("/"), packageParts = request.charAt(0) === "@" ? parts.slice(0, 2) : parts.slice(0, 1), packageName = packageParts.join("/"), subpath = parts.slice(packageParts.length).join("/"), directory = base; function exportTarget(value) { if (typeof value === "string") return value; if (Array.isArray(value)) { for (var item of value) { var arrayTarget = exportTarget(item); if (arrayTarget) return arrayTarget; } return null; } if (value && typeof value === "object") { for (var condition of Object.keys(value)) if (condition === "require" || condition === "node" || condition === "default") { var conditionTarget = exportTarget(value[condition]); if (conditionTarget) return conditionTarget; } } return null; } while (directory) { var packageRoot = directory + "/node_modules/" + packageName, packageText; try { packageText = __thaw_worker_read_source(packageRoot + "/package.json"); } catch (lookupError) {} if (packageText !== undefined) { var manifest = JSON.parse(packageText); if (manifest.exports !== undefined) { var exportKey = subpath ? "./" + subpath : ".", exportValue = manifest.exports; if (exportValue && typeof exportValue === "object" && !Array.isArray(exportValue) && Object.keys(exportValue).some(function(key) { return key.charAt(0) === "."; })) exportValue = exportValue[exportKey]; else if (subpath) exportValue = undefined; var resolvedExport = exportTarget(exportValue); if (!resolvedExport) { var pathError = new Error("Package subpath \'" + exportKey + "\' is not defined by exports in " + packageRoot + "/package.json"); pathError.code = "ERR_PACKAGE_PATH_NOT_EXPORTED"; throw pathError; } return __thawLoad(packageRoot, resolvedExport); } return subpath ? __thawLoad(packageRoot, "./" + subpath) : __thawLoad(directory + "/node_modules", "./" + packageName); } var slash = directory.lastIndexOf("/"); if (slash <= 0) break; directory = directory.slice(0, slash); } throw builtinError; } }'; return source.replace(builtinOnly, nodeModulesFallback); };
             var readRuntimeWorkerFileWithoutWildcardExports = readRuntimeWorkerFile; readRuntimeWorkerFile = function(value) { var source = readRuntimeWorkerFileWithoutWildcardExports(value), exactSelection = 'if (exportValue && typeof exportValue === "object" && !Array.isArray(exportValue) && Object.keys(exportValue).some(function(key) { return key.charAt(0) === "."; })) exportValue = exportValue[exportKey]; else if (subpath) exportValue = undefined;', wildcardSelection = 'var wildcardMatch; if (exportValue && typeof exportValue === "object" && !Array.isArray(exportValue) && Object.keys(exportValue).some(function(key) { return key.charAt(0) === "."; })) { var exportMap = exportValue; exportValue = exportMap[exportKey]; if (exportValue === undefined) { var wildcardKeys = Object.keys(exportMap).filter(function(key) { return key.indexOf("*") >= 0; }).sort(function(left, right) { return right.indexOf("*") - left.indexOf("*") || right.length - left.length; }); for (var wildcardKey of wildcardKeys) { var star = wildcardKey.indexOf("*"), prefix = wildcardKey.slice(0, star), suffix = wildcardKey.slice(star + 1); if (exportKey.slice(0, prefix.length) === prefix && exportKey.slice(exportKey.length - suffix.length) === suffix && exportKey.length >= prefix.length + suffix.length) { wildcardMatch = exportKey.slice(prefix.length, exportKey.length - suffix.length); exportValue = exportMap[wildcardKey]; break; } } } } else if (subpath) exportValue = undefined;', exactLoad = 'return __thawLoad(packageRoot, resolvedExport);', wildcardLoad = 'if (wildcardMatch !== undefined) resolvedExport = resolvedExport.split("*").join(wildcardMatch); return __thawLoad(packageRoot, resolvedExport);'; return source.replace(exactSelection, wildcardSelection).replace(exactLoad, wildcardLoad); };
             var readRuntimeWorkerFileWithoutPackageImports = readRuntimeWorkerFile; readRuntimeWorkerFile = function(value) { var source = readRuntimeWorkerFileWithoutPackageImports(value), loadStart = 'function __thawLoad(base, request) {', importsResolution = 'function __thawLoad(base, request) { if (request.charCodeAt(0) === 35) { var scope = base; function importTarget(value) { if (typeof value === "string") return value; if (Array.isArray(value)) { for (var item of value) { var arrayTarget = importTarget(item); if (arrayTarget) return arrayTarget; } return null; } if (value && typeof value === "object") { for (var condition of Object.keys(value)) if (condition === "require" || condition === "node" || condition === "default") { var conditionTarget = importTarget(value[condition]); if (conditionTarget) return conditionTarget; } } return null; } while (scope) { var scopeText; try { scopeText = __thaw_worker_read_source(scope + "/package.json"); } catch (scopeError) {} if (scopeText !== undefined) { var scopeManifest = JSON.parse(scopeText), imports = scopeManifest.imports || {}, importValue = imports[request], importMatch; if (importValue === undefined) { var importKeys = Object.keys(imports).filter(function(key) { return key.indexOf("*") >= 0; }).sort(function(left, right) { return right.indexOf("*") - left.indexOf("*") || right.length - left.length; }); for (var importKey of importKeys) { var importStar = importKey.indexOf("*"), importPrefix = importKey.slice(0, importStar), importSuffix = importKey.slice(importStar + 1); if (request.slice(0, importPrefix.length) === importPrefix && request.slice(request.length - importSuffix.length) === importSuffix && request.length >= importPrefix.length + importSuffix.length) { importMatch = request.slice(importPrefix.length, request.length - importSuffix.length); importValue = imports[importKey]; break; } } } var resolvedImport = importTarget(importValue); if (!resolvedImport) { var importError = new Error("Package import specifier \'" + request + "\' is not defined in " + scope + "/package.json"); importError.code = "ERR_PACKAGE_IMPORT_NOT_DEFINED"; throw importError; } if (importMatch !== undefined) resolvedImport = resolvedImport.split("*").join(importMatch); return __thawLoad(scope, resolvedImport); } var scopeSlash = scope.lastIndexOf("/"); if (scopeSlash <= 0) break; scope = scope.slice(0, scopeSlash); } var missingImport = new Error("Package import specifier \'" + request + "\' is not defined"); missingImport.code = "ERR_PACKAGE_IMPORT_NOT_DEFINED"; throw missingImport; }'; return source.replace(loadStart, importsResolution); };
             var readRuntimeWorkerFileWithoutPackageTargetGuards = readRuntimeWorkerFile; readRuntimeWorkerFile = function(value) { var source = readRuntimeWorkerFileWithoutPackageTargetGuards(value), exportLoad = 'if (wildcardMatch !== undefined) resolvedExport = resolvedExport.split("*").join(wildcardMatch); return __thawLoad(packageRoot, resolvedExport);', guardedExportLoad = 'if (wildcardMatch !== undefined) resolvedExport = resolvedExport.split("*").join(wildcardMatch); if (resolvedExport.slice(0, 2) !== "./" || resolvedExport.split("/").some(function(part) { return part === ".." || part === "node_modules"; })) { var exportTargetError = new Error("Invalid package target \'" + resolvedExport + "\' in " + packageRoot + "/package.json"); exportTargetError.code = "ERR_INVALID_PACKAGE_TARGET"; throw exportTargetError; } return __thawLoad(packageRoot, resolvedExport);', importLoad = 'return __thawLoad(scope, resolvedImport);', guardedImportLoad = 'if (resolvedImport.charAt(0) === "/" || (resolvedImport.charAt(0) === "." && resolvedImport.slice(0, 2) !== "./") || resolvedImport.split("/").some(function(part) { return part === ".." || part === "node_modules"; })) { var importTargetError = new Error("Invalid package target \'" + resolvedImport + "\' in " + scope + "/package.json"); importTargetError.code = "ERR_INVALID_PACKAGE_TARGET"; throw importTargetError; } return __thawLoad(scope, resolvedImport);'; return source.replace(exportLoad, guardedExportLoad).replace(importLoad, guardedImportLoad); };
             settleNativeDirect = function(request, errorText) { var pending = nativeDirectPending.get(Number(request)); if (!pending) return; nativeDirectPending.delete(Number(request)); if (pending.timer) clearTimeout(pending.timer); if (!errorText) { pending.resolve(); return; } var separator = errorText.indexOf(':'), code = separator < 0 ? errorText : errorText.slice(0, separator), message = separator < 0 ? errorText : errorText.slice(separator + 1), error = new Error(message); error.code = code; if (code === 'ERR_WORKER_MESSAGING_ERRORED') error.cause = new Error(message); pending.reject(error); }; module.exports.Worker = Worker; module.exports.postMessageToThread = postMessageToThread;
"#,
        )),
        _ => None,
    }
}

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
