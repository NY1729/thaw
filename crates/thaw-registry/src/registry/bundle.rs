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

include!("module_transform.rs");

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

include!("builtins.rs");

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
