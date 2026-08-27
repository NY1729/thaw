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

