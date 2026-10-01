fn constructor_pat_names(pat: &thaw_parser::ast::Pat, names: &mut std::collections::HashSet<String>) {
    use thaw_parser::ast::{ObjectPatProp, Pat};
    match pat {
        Pat::Ident(binding) => { names.insert(binding.id.sym.to_string()); }
        Pat::Array(array) => {
            for element in array.elems.iter().flatten() { constructor_pat_names(element, names); }
        }
        Pat::Object(object) => {
            for property in &object.props {
                match property {
                    ObjectPatProp::Assign(assign) => { names.insert(assign.key.sym.to_string()); }
                    ObjectPatProp::KeyValue(value) => constructor_pat_names(&value.value, names),
                    ObjectPatProp::Rest(rest) => constructor_pat_names(&rest.arg, names),
                }
            }
        }
        Pat::Assign(assign) => constructor_pat_names(&assign.left, names),
        Pat::Rest(rest) => constructor_pat_names(&rest.arg, names),
        _ => {}
    }
}

fn constructor_decl_names(
    decl: &thaw_parser::ast::Decl,
    names: &mut std::collections::HashSet<String>,
) {
    use thaw_parser::ast::Decl;
    match decl {
        Decl::Var(vars) => {
            for var in &vars.decls { constructor_pat_names(&var.name, names); }
        }
        Decl::Fn(function) => { names.insert(function.ident.sym.to_string()); }
        Decl::Class(class) => { names.insert(class.ident.sym.to_string()); }
        _ => {}
    }
}

#[derive(Default)]
struct ConstructorHoistedVars(std::collections::HashSet<String>);

impl swc_ecma_visit::Visit for ConstructorHoistedVars {
    fn visit_var_decl(&mut self, declaration: &thaw_parser::ast::VarDecl) {
        if declaration.kind == thaw_parser::ast::VarDeclKind::Var {
            for binding in &declaration.decls {
                constructor_pat_names(&binding.name, &mut self.0);
            }
        }
    }

    fn visit_function(&mut self, _function: &thaw_parser::ast::Function) {}
    fn visit_arrow_expr(&mut self, _arrow: &thaw_parser::ast::ArrowExpr) {}
    fn visit_class(&mut self, _class: &thaw_parser::ast::Class) {}
}

fn rewrite_external_class_constructors(
    source: &str,
    classes: &[ClassConstructorRewrite],
    package_qualifiers: &std::collections::HashMap<String, String>,
) -> Result<String, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        ArrowExpr, BlockStmt, CatchClause, ClassExpr, Constructor, Decl, DefaultDecl, Expr,
        FnExpr, ForHead, ForInStmt, ForOfStmt, ForStmt, Function, MemberProp, ModuleDecl,
        ModuleItem, NewExpr, ParamOrTsParamProp, Stmt, SwitchStmt, TsParamPropParam,
        VarDeclKind, VarDeclOrExpr,
    };
    use thaw_parser::common::Spanned;

    if classes.is_empty() { return Ok(source.to_string()); }
    let module = thaw_parser::parse_typescript(source)?;
    let (named_imports, namespace_imports, imported_names) =
        constructor_imports(&module, package_qualifiers);
    let mut top_level = std::collections::HashSet::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(decl)) => constructor_decl_names(decl, &mut top_level),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) =>
                constructor_decl_names(&export.decl, &mut top_level),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => match &export.decl {
                DefaultDecl::Class(class) => {
                    if let Some(name) = &class.ident { top_level.insert(name.sym.to_string()); }
                }
                DefaultDecl::Fn(function) => {
                    if let Some(name) = &function.ident { top_level.insert(name.sym.to_string()); }
                }
                _ => {}
            },
            _ => {}
        }
    }
    let mut hoisted = ConstructorHoistedVars::default();
    module.visit_with(&mut hoisted);
    top_level.extend(hoisted.0);
    struct Finder<'a> {
        classes: &'a [ClassConstructorRewrite],
        named_imports: std::collections::HashMap<String, (String, String)>,
        namespace_imports: std::collections::HashMap<String, String>,
        imported_names: std::collections::HashSet<String>,
        shadowed: Vec<std::collections::HashSet<String>>,
        replacements: Vec<(u32, u32, String)>,
    }
    impl Finder<'_> {
        fn is_shadowed(&self, name: &str) -> bool {
            self.shadowed.iter().any(|scope| scope.contains(name))
        }
        fn find_class(&self, qualifier: &str, name: &str) -> Option<&ClassConstructorRewrite> {
            self.classes.iter().find(|(package, class, _)| package == qualifier && class == name)
        }
        fn class_for_callee(&self, callee: &Expr) -> Option<&ClassConstructorRewrite> {
            match callee {
                Expr::Ident(name) => {
                    let local = name.sym.as_str();
                    if self.is_shadowed(local) { return None; }
                    if let Some((package, exported)) = self.named_imports.get(local) {
                        return self.find_class(package, exported);
                    }
                    if let Some(package) = self.namespace_imports.get(local) {
                        return self.find_class(package, "__namespace_root__");
                    }
                    if self.imported_names.contains(local) { return None; }
                    let mut candidates = self.classes.iter().filter(|(_, class, _)| class == local);
                    let only = candidates.next()?;
                    candidates.next().is_none().then_some(only)
                }
                Expr::Member(member) => {
                    let mut names = Vec::new();
                    let mut current = member;
                    let root = loop {
                        let MemberProp::Ident(property) = &current.prop else { return None; };
                        names.push(property.sym.to_string());
                        match current.obj.as_ref() {
                            Expr::Ident(root) => break root.sym.as_str(),
                            Expr::Member(parent) => current = parent,
                            _ => return None,
                        }
                    };
                    names.reverse();
                    let path = names.join(".");
                    if self.is_shadowed(root) { return None; }
                    let unimported = !self.namespace_imports.contains_key(root)
                        && !self.named_imports.contains_key(root);
                    let (package, name) = if let Some(package) = self.namespace_imports.get(root) {
                        (package.as_str(), path)
                    } else if let Some((package, exported)) = self.named_imports.get(root) {
                        (package.as_str(), format!("{exported}.{path}"))
                    } else if self.imported_names.contains(root) {
                        return None;
                    } else {
                        (root, path)
                    };
                    self.find_class(package, &name).or_else(|| {
                        unimported.then(|| {
                            self.classes.iter().find(|(_, class, _)| class == &format!("{root}.{name}"))
                        }).flatten()
                    })
                }
                _ => None,
            }
        }
    }
    impl Visit for Finder<'_> {
        fn visit_switch_stmt(&mut self, statement: &SwitchStmt) {
            statement.discriminant.visit_with(self);
            let mut names = std::collections::HashSet::new();
            for case in &statement.cases {
                for statement in &case.cons {
                    match statement {
                        Stmt::Decl(Decl::Var(vars)) if vars.kind != VarDeclKind::Var => {
                            for var in &vars.decls {
                                constructor_pat_names(&var.name, &mut names);
                            }
                        }
                        Stmt::Decl(decl @ (Decl::Class(_) | Decl::Fn(_))) =>
                            constructor_decl_names(decl, &mut names),
                        _ => {}
                    }
                }
            }
            self.shadowed.push(names);
            statement.cases.visit_with(self);
            self.shadowed.pop();
        }
        fn visit_block_stmt(&mut self, block: &BlockStmt) {
            let mut names = std::collections::HashSet::new();
            for statement in &block.stmts {
                if let Stmt::Decl(decl) = statement { constructor_decl_names(decl, &mut names); }
            }
            self.shadowed.push(names);
            block.visit_children_with(self);
            self.shadowed.pop();
        }
        fn visit_function(&mut self, function: &Function) {
            let mut names = std::collections::HashSet::new();
            for parameter in &function.params { constructor_pat_names(&parameter.pat, &mut names); }
            if let Some(body) = &function.body {
                let mut hoisted = ConstructorHoistedVars::default();
                body.visit_with(&mut hoisted);
                names.extend(hoisted.0);
            }
            self.shadowed.push(names);
            function.visit_children_with(self);
            self.shadowed.pop();
        }
        fn visit_fn_expr(&mut self, expression: &FnExpr) {
            let mut names = std::collections::HashSet::new();
            if let Some(name) = &expression.ident { names.insert(name.sym.to_string()); }
            self.shadowed.push(names);
            expression.visit_children_with(self);
            self.shadowed.pop();
        }
        fn visit_arrow_expr(&mut self, arrow: &ArrowExpr) {
            let mut names = std::collections::HashSet::new();
            for parameter in &arrow.params { constructor_pat_names(parameter, &mut names); }
            let mut hoisted = ConstructorHoistedVars::default();
            arrow.body.visit_with(&mut hoisted);
            names.extend(hoisted.0);
            self.shadowed.push(names);
            arrow.visit_children_with(self);
            self.shadowed.pop();
        }
        fn visit_catch_clause(&mut self, clause: &CatchClause) {
            let mut names = std::collections::HashSet::new();
            if let Some(parameter) = &clause.param { constructor_pat_names(parameter, &mut names); }
            self.shadowed.push(names);
            clause.visit_children_with(self);
            self.shadowed.pop();
        }
        fn visit_class_expr(&mut self, expression: &ClassExpr) {
            let mut names = std::collections::HashSet::new();
            if let Some(name) = &expression.ident { names.insert(name.sym.to_string()); }
            self.shadowed.push(names);
            expression.visit_children_with(self);
            self.shadowed.pop();
        }
        fn visit_constructor(&mut self, constructor: &Constructor) {
            let mut names = std::collections::HashSet::new();
            for parameter in &constructor.params {
                match parameter {
                    ParamOrTsParamProp::Param(param) => constructor_pat_names(&param.pat, &mut names),
                    ParamOrTsParamProp::TsParamProp(property) => match &property.param {
                        TsParamPropParam::Ident(binding) => { names.insert(binding.id.sym.to_string()); }
                        TsParamPropParam::Assign(assign) => constructor_pat_names(&assign.left, &mut names),
                    },
                }
            }
            if let Some(body) = &constructor.body {
                let mut hoisted = ConstructorHoistedVars::default();
                body.visit_with(&mut hoisted);
                names.extend(hoisted.0);
            }
            self.shadowed.push(names);
            constructor.visit_children_with(self);
            self.shadowed.pop();
        }
        fn visit_for_stmt(&mut self, statement: &ForStmt) {
            let mut names = std::collections::HashSet::new();
            if let Some(VarDeclOrExpr::VarDecl(vars)) = &statement.init {
                for var in &vars.decls { constructor_pat_names(&var.name, &mut names); }
            }
            self.shadowed.push(names);
            statement.visit_children_with(self);
            self.shadowed.pop();
        }
        fn visit_for_in_stmt(&mut self, statement: &ForInStmt) {
            let mut names = std::collections::HashSet::new();
            if let ForHead::VarDecl(vars) = &statement.left {
                for var in &vars.decls { constructor_pat_names(&var.name, &mut names); }
            }
            self.shadowed.push(names);
            statement.visit_children_with(self);
            self.shadowed.pop();
        }
        fn visit_for_of_stmt(&mut self, statement: &ForOfStmt) {
            let mut names = std::collections::HashSet::new();
            if let ForHead::VarDecl(vars) = &statement.left {
                for var in &vars.decls { constructor_pat_names(&var.name, &mut names); }
            }
            self.shadowed.push(names);
            statement.visit_children_with(self);
            self.shadowed.pop();
        }
        fn visit_new_expr(&mut self, expression: &NewExpr) {
            let argument_count = expression.args.as_ref().map_or(0, Vec::len);
            let helper = self.class_for_callee(&expression.callee).and_then(|(_, _, helpers)| {
                let mut candidates = helpers.iter().filter(|(arity, _, _)| *arity == argument_count);
                let (_, helper, _) = candidates.next()?;
                candidates.next().is_none().then(|| helper.clone())
            });
            if let Some(helper) = helper {
                self.replacements.push((
                    expression.span().lo.0,
                    expression.type_args.as_ref().map_or_else(
                        || expression.callee.span().hi.0,
                        |args| args.span().hi.0,
                    ),
                    helper,
                ));
            }
            expression.visit_children_with(self);
        }
    }
    let mut finder = Finder {
        classes,
        named_imports,
        namespace_imports,
        imported_names,
        shadowed: vec![top_level],
        replacements: Vec::new(),
    };
    module.visit_with(&mut finder);
    let mut output = source.to_string();
    for (start, end, helper) in finder.replacements.into_iter().rev() {
        output.replace_range((start - 1) as usize..(end - 1) as usize, &helper);
    }
    Ok(output)
}

/// Rewrites `pkg.name(...)` call expressions in a user's own `.ts` source
/// to the package-qualified alias identifier `generate_registry_shims`'s
/// collision resolution actually emits for a colliding name (e.g.
/// `qs.stringify(x)` -> `qs_stringify(x)`), so a user can keep writing
/// the familiar namespace-qualified form even though Thaw has no real
/// object/member-call support backing it -- this is resolved *entirely*
/// as source-level syntax sugar, at this preprocessing step, not by the
/// compiler. `rewrites` is empty when there were no collisions at all,
/// in which case this returns `source` untouched without even parsing it.
///
/// Real default/namespace imports are resolved against their source package
/// too, so overload selection sees the package-qualified alias before the
/// module graph removes the import. Named imports remain excluded because a
/// member access on one of those is an ordinary value operation, not package
/// qualification.
///
/// Uses `swc_ecma_visit`'s `Visit` to walk the whole AST (a call can be
/// nested arbitrarily deep in an expression), unlike the top-level-only
/// walks elsewhere in this project (thaw-registry also uses a full AST walk
/// for JavaScript dependency discovery). Matched
/// spans are collected first and applied as one pass of text
/// substitution over the original source afterward, copying everything
/// else verbatim -- this project carries no general JS/TS code
/// generator, so re-printing from the AST isn't an option.
fn rewrite_qualified_calls(
    source: &str,
    rewrites: &[QualifiedCallRewrite],
    imported_overload_aliases: &std::collections::HashSet<String>,
) -> Result<String, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{CallExpr, Callee, Expr, ImportSpecifier, MemberProp, ModuleDecl, ModuleItem};
    use thaw_parser::common::Spanned;

    if rewrites.is_empty() {
        return Ok(source.to_string());
    }

    struct Finder<'a> {
        rewrites: &'a [(String, String, String)],
        imported_names: std::collections::HashSet<String>,
        imported_packages: std::collections::HashMap<String, String>,
        imported_overload_aliases: &'a std::collections::HashSet<String>,
        matches: Vec<(u32, u32, String)>,
    }
    impl Visit for Finder<'_> {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if let Expr::Member(member) = &**callee {
                    let mut names = Vec::new();
                    let mut base = callee.as_ref();
                    while let Expr::Member(segment) = base {
                        let MemberProp::Ident(property) = &segment.prop else { break };
                        names.push(property.sym.to_string());
                        base = segment.obj.as_ref();
                    }
                    if let Expr::Ident(obj) = base {
                        names.reverse();
                        let path = names.join(".");
                        let package = self
                            .imported_packages
                            .get(obj.sym.as_str())
                            .map(String::as_str)
                            .unwrap_or(obj.sym.as_str());
                        if let Some((_, _, alias)) =
                            self.rewrites.iter().find(|(pkg, name, _)| {
                                pkg == package && name == &path
                            })
                        {
                            let imported = self.imported_packages.contains_key(obj.sym.as_str());
                            if (!self.imported_names.contains(obj.sym.as_str()) || imported)
                                && (!imported || self.imported_overload_aliases.contains(alias))
                            {
                                let span = member.span();
                                self.matches.push((span.lo.0, span.hi.0, alias.clone()));
                            }
                        }
                    }
                }
            }
            call.visit_children_with(self);
        }
    }

    let (module, cm) = thaw_parser::parse_typescript_with_source_map(source)?;
    let mut imported_names = std::collections::HashSet::new();
    let mut imported_packages = std::collections::HashMap::new();
    for item in &module.body {
        if let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item {
            let package = import.src.value.as_str().unwrap_or_default().to_string();
            for specifier in &import.specifiers {
                let local = match specifier {
                    ImportSpecifier::Named(named) => &named.local,
                    ImportSpecifier::Default(default) => {
                        imported_packages.insert(default.local.sym.to_string(), package.clone());
                        &default.local
                    }
                    ImportSpecifier::Namespace(namespace) => {
                        imported_packages
                            .insert(namespace.local.sym.to_string(), package.clone());
                        &namespace.local
                    }
                };
                imported_names.insert(local.sym.to_string());
            }
        }
    }
    let mut finder = Finder {
        rewrites,
        imported_names,
        imported_packages,
        imported_overload_aliases,
        matches: Vec::new(),
    };
    module.visit_with(&mut finder);

    if finder.matches.is_empty() {
        return Ok(source.to_string());
    }
    finder.matches.sort_by_key(|(lo, ..)| *lo);

    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (lo, hi, alias) in &finder.matches {
        let lo = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(*lo))
            .pos
            .0 as usize;
        let hi = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(*hi))
            .pos
            .0 as usize;
        out.push_str(&source[cursor..lo]);
        out.push_str(alias);
        cursor = hi;
    }
    out.push_str(&source[cursor..]);
    Ok(out)
}
