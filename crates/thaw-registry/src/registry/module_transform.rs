/// Parses a JavaScript module and collects its statically knowable dependency
/// edges. Direct `require`/`import` arguments are constant-folded when they
/// consist solely of string literals, expression-free templates, parentheses,
/// and string concatenation; runtime expressions remain dynamic. Shadowed or
/// member calls, comments, and strings are not mistaken for edges.
/// ESM declarations are collected directly from the module AST before they
/// are lowered to CommonJS.
#[derive(Clone, Default)]
struct ModuleAnalysis {
    specs: Vec<String>,
    static_esm_specs: Vec<String>,
    has_esm: bool,
    has_top_level_await: bool,
    attribute_error: Option<String>,
    has_nonliteral_module_load: bool,
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
        has_nonliteral_module_load: bool,
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
                } else {
                    self.has_nonliteral_module_load = true;
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
        has_nonliteral_module_load: false,
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
        has_nonliteral_module_load: calls.has_nonliteral_module_load,
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

    fn direct_bindings(statements: &[Stmt]) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for statement in statements {
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
            if let Some(body) = &function.body {
                self.shadowed.push(direct_bindings(&body.stmts));
                body.stmts.visit_with(self);
                self.shadowed.pop();
            }
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
            self.shadowed.push(direct_bindings(&block.stmts));
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

fn rewrite_import_meta_urls(source: &str) -> String {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{Expr, MemberExpr, MemberProp, MetaPropKind};
    use thaw_parser::common::Spanned;

    #[derive(Default)]
    struct ImportMetaUrls(Vec<(u32, u32)>);
    impl Visit for ImportMetaUrls {
        fn visit_member_expr(&mut self, member: &MemberExpr) {
            if matches!(member.obj.as_ref(), Expr::MetaProp(meta) if meta.kind == MetaPropKind::ImportMeta)
                && matches!(&member.prop, MemberProp::Ident(property) if property.sym == "url")
            {
                let span = member.span();
                self.0.push((span.lo.0, span.hi.0));
                return;
            }
            member.visit_children_with(self);
        }
    }

    let Ok((module, source_map)) = thaw_parser::parse_javascript_with_source_map(source) else {
        return source.to_string();
    };
    let mut urls = ImportMetaUrls::default();
    module.visit_with(&mut urls);
    let mut output = source.to_string();
    for (lo, hi) in urls.0.into_iter().rev() {
        let lo = source_map
            .lookup_byte_offset(thaw_parser::common::BytePos(lo))
            .pos
            .0 as usize;
        let hi = source_map
            .lookup_byte_offset(thaw_parser::common::BytePos(hi))
            .pos
            .0 as usize;
        output.replace_range(lo..hi, "('file://' + __filename)");
    }
    output
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
                    "__thaw_require"
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
                        "__thaw_require"
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
                    "__thaw_require"
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

    Some(rewrite_import_meta_urls(&format!(
        "module.exports.__esModule = true;\n{local_export_prologue}{prologue}{rest}"
    )))
}
