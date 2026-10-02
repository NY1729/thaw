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
    import_condition_specs: Vec<String>,
    require_condition_specs: Vec<String>,
    has_esm: bool,
    has_top_level_await: bool,
    uses_import_meta: bool,
    attribute_error: Option<String>,
    has_nonliteral_module_load: bool,
    uses_global_fetch: bool,
    _commonjs_exports: Vec<String>,
}

fn rewritten_js_source_name(source_name: &thaw_parser::common::FileName, phase: &str) -> thaw_parser::common::FileName {
    thaw_parser::common::FileName::Custom(format!("{source_name} ({phase})").into())
}

#[cfg(test)]
fn analyze_module(source: &str) -> ModuleAnalysis {
    analyze_module_named(source, &thaw_parser::common::FileName::Custom("input.js".into()))
}

fn analyze_module_named(source: &str, source_name: &thaw_parser::common::FileName) -> ModuleAnalysis {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        ArrowExpr, AssignExpr, AssignTarget, AwaitExpr, CallExpr, Callee, Expr, ForOfStmt, Function, Ident,
        ImportSpecifier, Lit, MemberExpr, MemberProp, ModuleDecl, ModuleExportName, ModuleItem,
        MetaPropKind, ObjectLit, Pat, Prop, PropName, PropOrSpread, SimpleAssignTarget, VarDeclarator,
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
        dynamic_import_specs: Vec<String>,
        require_specs: Vec<String>,
        commonjs_exports: Vec<String>,
        has_nonliteral_module_load: bool,
        attribute_error: Option<String>,
        require_functions: Vec<String>,
        create_require_functions: Vec<String>,
        module_namespaces: Vec<String>,
        uses_global_fetch: bool,
        uses_import_meta: bool,
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
        fn visit_for_of_stmt(&mut self, statement: &ForOfStmt) {
            if statement.is_await {
                self.found = true;
            }
            statement.visit_children_with(self);
        }
        fn visit_function(&mut self, _: &Function) {}
        fn visit_arrow_expr(&mut self, _: &ArrowExpr) {}
    }
    impl Visit for Calls {
        fn visit_expr(&mut self, expression: &Expr) {
            if matches!(expression, Expr::MetaProp(meta) if meta.kind == MetaPropKind::ImportMeta) {
                self.uses_import_meta = true;
            }
            expression.visit_children_with(self);
        }

        fn visit_ident(&mut self, identifier: &Ident) {
            if identifier.sym == "fetch" {
                self.uses_global_fetch = true;
            }
        }

        // A bare `fetch` reference (`visit_ident` above) misses the very
        // common cross-platform-compatible pattern of reaching it off a
        // known global object instead -- `globalThis.fetch(...)`, or
        // ky's own `globalThis.fetch.bind(globalThis)` -- because a
        // member expression's property is a distinct AST node from a
        // bound identifier reference, so `visit_ident` never sees
        // `fetch` there at all. Found via ky, whose own bundle never
        // requires `node:http` itself and never references bare
        // `fetch`, so it silently got no fetch shim and no bundled
        // `node:https`/`node:http` to back one.
        fn visit_member_expr(&mut self, member: &MemberExpr) {
            if property_name(&member.prop).as_deref() == Some("fetch")
                && matches!(member.obj.as_ref(), Expr::Ident(identifier) if matches!(identifier.sym.as_ref(), "globalThis" | "self" | "window" | "global"))
            {
                self.uses_global_fetch = true;
            }
            member.visit_children_with(self);
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
            let is_require_resolve = matches!(
                &call.callee,
                Callee::Expr(callee)
                    if matches!(callee.as_ref(), Expr::Member(member)
                        if matches!(member.obj.as_ref(), Expr::Ident(ident)
                            if self.require_functions.iter().any(|name| name == ident.sym.as_ref()))
                        && property_name(&member.prop).as_deref() == Some("resolve"))
            );
            let is_import = matches!(&call.callee, Callee::Import(_));
            if (((is_require || is_require_resolve) && call.args.len() == 1)
                || (is_import && !call.args.is_empty()))
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
                    if is_import {
                        self.dynamic_import_specs.extend(specifiers.iter().cloned());
                    } else {
                        self.require_specs.extend(specifiers.iter().cloned());
                    }
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

    let Ok((module, _)) = thaw_parser::parse_javascript_with_source_map_named(source, source_name.clone()) else {
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
        dynamic_import_specs: Vec::new(),
        require_specs: Vec::new(),
        commonjs_exports: Vec::new(),
        has_nonliteral_module_load: false,
        attribute_error: None,
        require_functions: vec!["require".to_string()],
        create_require_functions,
        module_namespaces,
        uses_global_fetch: false,
        uses_import_meta: false,
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
    let mut import_condition_specs = static_esm_specs.clone();
    for spec in calls.dynamic_import_specs {
        if !import_condition_specs.contains(&spec) {
            import_condition_specs.push(spec);
        }
    }
    ModuleAnalysis {
        specs: unique,
        static_esm_specs,
        import_condition_specs,
        require_condition_specs: calls.require_specs,
        has_esm: module
            .body
            .iter()
            .any(|item| matches!(item, ModuleItem::ModuleDecl(_))),
        has_top_level_await: top_level_await.found,
        uses_import_meta: calls.uses_import_meta,
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
#[cfg(test)]
fn rewrite_dynamic_imports(source: &str) -> Option<String> {
    rewrite_dynamic_imports_named(source, &thaw_parser::common::FileName::Custom("input.js".into()))
}

fn rewrite_dynamic_imports_named(source: &str, source_name: &thaw_parser::common::FileName) -> Option<String> {
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

    let (module, cm) = thaw_parser::parse_javascript_with_source_map_named(source, source_name.clone()).ok()?;
    let mut imports = Imports { spans: Vec::new() };
    module.visit_with(&mut imports);
    if imports.spans.is_empty() {
        return None;
    }
    // Replace only the call's wrapper. Nested import arguments remain intact
    // until their own, disjoint wrapper edits are applied from right to left.
    let mut edits = Vec::with_capacity(imports.spans.len() * 2);
    for (lo, hi, argument_lo, argument_hi) in imports.spans {
        let offset = |position| {
            cm.lookup_byte_offset(thaw_parser::common::BytePos(position))
                .pos
                .0 as usize
        };
        edits.push((offset(lo), offset(argument_lo), "requireAsync("));
        edits.push((offset(argument_hi), offset(hi), ")"));
    }
    edits.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    let mut output = source.to_string();
    for (start, end, replacement) in edits {
        output.replace_range(start..end, replacement);
    }
    Some(output)
}

fn pattern_names(pattern: &thaw_parser::ast::Pat, names: &mut std::collections::BTreeSet<String>) {
    match pattern {
        thaw_parser::ast::Pat::Ident(binding) => {
            names.insert(binding.id.sym.to_string());
        }
        thaw_parser::ast::Pat::Array(array) => {
            for element in array.elems.iter().flatten() {
                pattern_names(element, names);
            }
        }
        thaw_parser::ast::Pat::Object(object) => {
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
        thaw_parser::ast::Pat::Assign(assign) => pattern_names(&assign.left, names),
        thaw_parser::ast::Pat::Rest(rest) => pattern_names(&rest.arg, names),
        thaw_parser::ast::Pat::Expr(_) | thaw_parser::ast::Pat::Invalid(_) => {}
    }
}

/// Choose one unused number range for every name introduced by ESM lowering.
/// The reference pass and declaration pass must use the same offset.
fn esm_synthetic_offset_named(source: &str, source_name: &thaw_parser::common::FileName) -> Option<usize> {
    use std::collections::BTreeSet;
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{Ident, ModuleDecl, ModuleItem};

    #[derive(Default)]
    struct Identifiers(BTreeSet<String>);
    impl Visit for Identifiers {
        fn visit_ident(&mut self, ident: &Ident) {
            self.0.insert(ident.sym.to_string());
        }
    }

    let (module, _) = thaw_parser::parse_javascript_with_source_map_named(source, source_name.clone()).ok()?;
    let mut identifiers = Identifiers::default();
    module.visit_with(&mut identifiers);
    let count = module.body.iter().filter(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::Import(_) | ModuleDecl::ExportAll(_)) => true,
        ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) => export.src.is_some(),
        _ => false,
    }).count();
    let unused = |name: &str| !identifiers.0.contains(name) && !source.contains(name);
    (0..).find(|offset| {
        unused(&format!("__thaw_esm_key_{offset}"))
            && unused(&format!("__thaw_esm_origin_{offset}"))
            && unused(&format!("__thaw_esm_resolution_{offset}"))
            && (0..count).all(|index| {
                let index = offset + index;
                ["import", "reexport", "reexport_all"].iter().all(|kind| {
                    unused(&format!("__thaw_esm_{kind}_{index}"))
                })
            })
    })
}

fn rewrite_live_import_references_named(source: &str, synthetic_offset: usize, source_name: &thaw_parser::common::FileName) -> Option<String> {
    use std::collections::{BTreeMap, BTreeSet};
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        ArrowExpr, ArrowFunctionBody, BlockStmt, CallExpr, Callee, CatchClause, Class, Decl, Expr,
        FnExpr, ForHead, ForInStmt, ForOfStmt, ForStmt, Function, ImportSpecifier, ModuleDecl,
        ModuleExportName, ModuleItem, OptChainBase, OptChainExpr, Prop, StaticBlock, Stmt,
        SwitchStmt, TaggedTpl, VarDecl,
        VarDeclKind, VarDeclOrExpr,
    };
    use thaw_parser::common::Spanned;


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

    fn variable_names(declaration: &VarDecl) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for declarator in &declaration.decls {
            pattern_names(&declarator.name, &mut names);
        }
        names
    }

    fn hoisted_var_bindings(statements: &[Stmt]) -> BTreeSet<String> {
        struct HoistedVars(BTreeSet<String>);
        impl Visit for HoistedVars {
            fn visit_var_decl(&mut self, declaration: &VarDecl) {
                if declaration.kind == VarDeclKind::Var {
                    self.0.extend(variable_names(declaration));
                }
                declaration.visit_children_with(self);
            }
            fn visit_function(&mut self, _: &Function) {}
            fn visit_arrow_expr(&mut self, _: &ArrowExpr) {}
            fn visit_class(&mut self, _: &Class) {}
        }
        let mut collector = HoistedVars(BTreeSet::new());
        statements.visit_with(&mut collector);
        collector.0
    }

    let (module, cm) = thaw_parser::parse_javascript_with_source_map_named(source, source_name.clone()).ok()?;
    let export_name = |name: &ModuleExportName| match name {
        ModuleExportName::Ident(identifier) => identifier.sym.to_string(),
        ModuleExportName::Str(value) => value.value.to_string_lossy().into_owned(),
    };
    let mut bindings = BTreeMap::<String, String>::new();
    let mut named_bindings = BTreeSet::new();
    let origin_name = format!("__thaw_esm_origin_{synthetic_offset}");
    let mut synthetic_count = synthetic_offset;
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                let module_name = format!("__thaw_esm_import_{synthetic_count}");
                synthetic_count += 1;
                for specifier in &import.specifiers {
                    match specifier {
                        ImportSpecifier::Named(named) => {
                            let local = named.local.sym.to_string();
                            named_bindings.insert(local.clone());
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
                                    "((typeof {origin_name} !== 'undefined') ? {origin_name}.defaultImport({}, {module_name}) : (({module_name} && {module_name}.__esModule) ? {module_name}.default : {module_name}))",
                                    js_string_literal(&import.src.value.to_string_lossy())
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
        named_bindings: &'a BTreeSet<String>,
        shadowed: Vec<BTreeSet<String>>,
        replacements: Vec<(u32, u32, String)>,
    }
    impl References<'_> {
        fn is_shadowed(&self, name: &str) -> bool {
            self.shadowed.iter().rev().any(|scope| scope.contains(name))
        }
        fn detach_named_import(&mut self, expression: &Expr) -> bool {
            let identifier = match expression {
                Expr::Ident(identifier) => identifier,
                Expr::Paren(parenthesized) => return self.detach_named_import(&parenthesized.expr),
                _ => return false,
            };
            let name = identifier.sym.as_str();
            if self.is_shadowed(name) || !self.named_bindings.contains(name) {
                return false;
            }
            let Some(replacement) = self.bindings.get(name) else {
                return false;
            };
            let span = identifier.span();
            self.replacements.push((span.lo.0, span.hi.0, format!("(0, {replacement})")));
            true
        }
        fn push_function_scope(&mut self, function: &Function) {
            let mut names = BTreeSet::new();
            for parameter in &function.params {
                pattern_names(&parameter.pat, &mut names);
            }
            self.shadowed.push(names);
            function.params.visit_with(self);
            function.decorators.visit_with(self);
            if let Some(body) = &function.body {
                let mut names = direct_bindings(&body.stmts);
                names.extend(hoisted_var_bindings(&body.stmts));
                self.shadowed.push(names);
                body.stmts.visit_with(self);
                self.shadowed.pop();
            }
            self.shadowed.pop();
        }
    }
    impl Visit for References<'_> {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if self.detach_named_import(callee) {
                    call.args.visit_with(self);
                    return;
                }
            }
            call.visit_children_with(self);
        }

        fn visit_opt_chain_expr(&mut self, chain: &OptChainExpr) {
            if let OptChainBase::Call(call) = chain.base.as_ref() {
                if self.detach_named_import(&call.callee) {
                    call.args.visit_with(self);
                    return;
                }
            }
            chain.visit_children_with(self);
        }

        fn visit_tagged_tpl(&mut self, tagged: &TaggedTpl) {
            if self.detach_named_import(&tagged.tag) {
                tagged.tpl.visit_with(self);
                return;
            }
            tagged.visit_children_with(self);
        }

        fn visit_fn_expr(&mut self, expression: &FnExpr) {
            let mut names = BTreeSet::new();
            if let Some(name) = &expression.ident {
                names.insert(name.sym.to_string());
            }
            self.shadowed.push(names);
            expression.function.visit_with(self);
            self.shadowed.pop();
        }

        fn visit_function(&mut self, function: &Function) {
            self.push_function_scope(function);
        }

        fn visit_arrow_expr(&mut self, arrow: &ArrowExpr) {
            let mut names = BTreeSet::new();
            for parameter in &arrow.params {
                pattern_names(parameter, &mut names);
            }
            self.shadowed.push(names);
            arrow.params.visit_with(self);
            if let ArrowFunctionBody::FunctionBody(body) = arrow.body.as_ref() {
                let mut body_names = direct_bindings(&body.stmts);
                body_names.extend(hoisted_var_bindings(&body.stmts));
                self.shadowed.push(body_names);
            }
            arrow.body.visit_with(self);
            if matches!(arrow.body.as_ref(), ArrowFunctionBody::FunctionBody(_)) {
                self.shadowed.pop();
            }
            self.shadowed.pop();
        }

        fn visit_for_stmt(&mut self, statement: &ForStmt) {
            let names = match statement.init.as_ref() {
                Some(VarDeclOrExpr::VarDecl(declaration))
                    if declaration.kind != VarDeclKind::Var => variable_names(declaration),
                _ => BTreeSet::new(),
            };
            self.shadowed.push(names);
            statement.visit_children_with(self);
            self.shadowed.pop();
        }

        fn visit_for_in_stmt(&mut self, statement: &ForInStmt) {
            let names = match &statement.left {
                ForHead::VarDecl(declaration) if declaration.kind != VarDeclKind::Var => {
                    variable_names(declaration)
                }
                _ => BTreeSet::new(),
            };
            self.shadowed.push(names);
            statement.visit_children_with(self);
            self.shadowed.pop();
        }

        fn visit_for_of_stmt(&mut self, statement: &ForOfStmt) {
            let names = match &statement.left {
                ForHead::VarDecl(declaration) if declaration.kind != VarDeclKind::Var => {
                    variable_names(declaration)
                }
                _ => BTreeSet::new(),
            };
            self.shadowed.push(names);
            statement.visit_children_with(self);
            self.shadowed.pop();
        }

        fn visit_switch_stmt(&mut self, statement: &SwitchStmt) {
            statement.discriminant.visit_with(self);
            let mut names = BTreeSet::new();
            for case in &statement.cases {
                names.extend(direct_bindings(&case.cons));
            }
            self.shadowed.push(names);
            statement.cases.visit_with(self);
            self.shadowed.pop();
        }

        fn visit_static_block(&mut self, block: &StaticBlock) {
            self.shadowed.push(hoisted_var_bindings(&block.body.stmts));
            block.body.visit_with(self);
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
        named_bindings: &named_bindings,
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

fn rewrite_import_meta_urls_named(source: &str, source_name: &thaw_parser::common::FileName) -> String {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{Expr, Ident, MetaPropKind};
    use thaw_parser::common::Spanned;

    // `import.meta` itself (bare, or as the object of `.url`/`.resolve`/
    // any other member access) is rejected outright by QuickJS's
    // script-mode parser -- it's only valid in real ESM module code,
    // and this bundler's CJS-rewritten output is evaluated as a plain
    // script. A real package's source can reference it even on a path
    // this build never actually executes (e.g. yargs's `.config()`
    // "extends" feature uses `import.meta.resolve(...)`) -- since
    // QuickJS parses the whole file eagerly, an unreached reference
    // still blocks every other statement in the bundle from loading at
    // all. All references in a module must share one object, including
    // properties added by its own code. Keep its declaration inside the
    // module factory and initialize it on first access.
    #[derive(Default)]
    struct ImportMetaExprs {
        spans: Vec<(u32, u32)>,
        identifiers: std::collections::BTreeSet<String>,
    }
    impl Visit for ImportMetaExprs {
        fn visit_ident(&mut self, ident: &Ident) {
            self.identifiers.insert(ident.sym.to_string());
        }

        fn visit_expr(&mut self, expr: &Expr) {
            if let Expr::MetaProp(meta) = expr {
                if meta.kind == MetaPropKind::ImportMeta {
                    let span = meta.span();
                    self.spans.push((span.lo.0, span.hi.0));
                    return;
                }
            }
            expr.visit_children_with(self);
        }
    }

    let Ok((module, source_map)) = thaw_parser::parse_javascript_with_source_map_named(source, source_name.clone()) else {
        return source.to_string();
    };
    let mut metas = ImportMetaExprs::default();
    module.visit_with(&mut metas);
    if metas.spans.is_empty() {
        return source.to_string();
    }
    // Check parsed names as well as source text: a Unicode-escaped
    // identifier can bind the same name without containing its spelling.
    let binding = (0..)
        .map(|index| format!("__thaw_import_meta_{index}"))
        .find(|name| !metas.identifiers.contains(name) && !source.contains(name.as_str()))
        .expect("an unused import.meta binding name exists");
    let meta = format!(
        "({binding} || ({binding} = {{url: ('file://' + __filename), resolve: function() {{ throw new Error('import.meta.resolve is not supported'); }}}}))"
    );
    let mut output = source.to_string();
    for (lo, hi) in metas.spans.into_iter().rev() {
        let lo = source_map
            .lookup_byte_offset(thaw_parser::common::BytePos(lo))
            .pos
            .0 as usize;
        let hi = source_map
            .lookup_byte_offset(thaw_parser::common::BytePos(hi))
            .pos
            .0 as usize;
        output.replace_range(lo..hi, &meta);
    }
    // `var` is hoisted, so this placement preserves leading directives
    // while every replacement still sees the same factory-local slot.
    output.push_str(&format!("\nvar {binding};"));
    output
}

/// Each module is wrapped as `function(module, exports, require,
/// requireAsync, __filename, __dirname) { <body> }` (see
/// `bundle/render.rs`) so a real CommonJS-style `module`/`exports`/
/// `require` and Node's own `__filename`/`__dirname` globals are
/// available without any import. A common real-world ESM idiom
/// re-derives exactly these same names at the top of a file as local
/// `const`/`let` bindings (`const __dirname = fileURLToPath(dirname(
/// import.meta.url));`, `const require = createRequire(import.meta.
/// url);`) as an ESM/CJS-dual-package shim -- since ESM has no such
/// globals natively. Once wrapped, that redeclaration collides with
/// the wrapper's own parameter of the identical name, which QuickJS
/// (like real JS) rejects outright ("invalid redefinition of parameter
/// name") -- even though the value it computes is equivalent to what
/// the wrapper already supplies.
///
/// Rather than reason about whether the shim's own computation matches
/// the wrapper's value, just drop the `const`/`let` keyword from a
/// top-level (module-body-scope, not nested in any block or function)
/// declaration whose sole binding is exactly one of these reserved
/// names, turning it into a plain reassignment of the existing
/// parameter -- same runtime effect the shim always intended, no new
/// lexical binding, no collision.
fn strip_reserved_wrapper_redeclarations_named(source: &str, source_name: &thaw_parser::common::FileName) -> String {
    use thaw_parser::ast::{Decl, ModuleItem, Pat, Stmt, VarDeclKind};
    use thaw_parser::common::Spanned;

    const RESERVED: [&str; 6] = [
        "module",
        "exports",
        "require",
        "requireAsync",
        "__filename",
        "__dirname",
    ];

    let Ok((module, source_map)) = thaw_parser::parse_javascript_with_source_map_named(source, source_name.clone()) else {
        return source.to_string();
    };
    let mut edits: Vec<(u32, u32)> = Vec::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Var(var_decl))) = item else {
            continue;
        };
        if !matches!(var_decl.kind, VarDeclKind::Const | VarDeclKind::Let) {
            continue;
        }
        let [declarator] = var_decl.decls.as_slice() else {
            continue;
        };
        if declarator.init.is_none() {
            continue;
        }
        let Pat::Ident(ident) = &declarator.name else {
            continue;
        };
        if !RESERVED.contains(&ident.id.sym.as_str()) {
            continue;
        }
        edits.push((var_decl.span().lo.0, ident.id.span().lo.0));
    }
    if edits.is_empty() {
        return source.to_string();
    }
    let mut output = source.to_string();
    for (lo, hi) in edits.into_iter().rev() {
        let lo = source_map
            .lookup_byte_offset(thaw_parser::common::BytePos(lo))
            .pos
            .0 as usize;
        let hi = source_map
            .lookup_byte_offset(thaw_parser::common::BytePos(hi))
            .pos
            .0 as usize;
        output.replace_range(lo..hi, "");
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
    // This test-only convenience has no dependency graph. A star needs the
    // bundle's binding-origin graph to distinguish a diamond from ambiguity.
    if esm_export_graph(source)
        .and_then(|graph| graph.get("stars").and_then(|stars| stars.as_array()).cloned())
        .is_some_and(|stars| !stars.is_empty())
    {
        return None;
    }
    rewrite_esm_to_commonjs_mode_named(source, false, &thaw_parser::common::FileName::Custom("input.js".into()))
}

#[cfg(test)]
fn rewrite_esm_to_commonjs_mode(source: &str, await_imports: bool) -> Option<String> {
    rewrite_esm_to_commonjs_mode_named(source, await_imports, &thaw_parser::common::FileName::Custom("input.js".into()))
}

fn rewrite_esm_to_commonjs_mode_named(source: &str, await_imports: bool, source_name: &thaw_parser::common::FileName) -> Option<String> {
    use std::collections::BTreeSet;
    use thaw_parser::ast::{
        Decl, DefaultDecl, ExportSpecifier, ImportSpecifier, ModuleDecl, ModuleExportName,
        ModuleItem,
    };
    use thaw_parser::common::{SourceMapper, Spanned};

    let dynamic_source = rewrite_dynamic_imports_named(source, source_name);
    let dynamic_name = dynamic_source.as_ref().filter(|rewritten| rewritten.as_str() != source)
        .map(|_| rewritten_js_source_name(source_name, "dynamic import rewrite"));
    let source = dynamic_source.as_deref().unwrap_or(source);
    let source_name = dynamic_name.as_ref().unwrap_or(source_name);
    let synthetic_offset = esm_synthetic_offset_named(source, source_name)?;
    let live_source = rewrite_live_import_references_named(source, synthetic_offset, source_name);
    let live_name = live_source.as_ref().filter(|rewritten| rewritten.as_str() != source)
        .map(|_| rewritten_js_source_name(source_name, "live import rewrite"));
    let source = live_source.as_deref().unwrap_or(source);
    let source_name = live_name.as_ref().unwrap_or(source_name);
    let (module, cm) = thaw_parser::parse_javascript_with_source_map_named(source, source_name.clone()).ok()?;
    let has_esm_syntax = module
        .body
        .iter()
        .any(|item| matches!(item, ModuleItem::ModuleDecl(_)));
    if !has_esm_syntax {
        let rewritten = rewrite_import_meta_urls_named(source, source_name);
        return if rewritten == source {
            live_source.or(dynamic_source)
        } else {
            Some(rewritten)
        };
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
            Decl::Var(v) => {
                let mut names = BTreeSet::new();
                for declarator in &v.decls {
                    pattern_names(&declarator.name, &mut names);
                }
                names.into_iter().collect()
            }
            _ => Vec::new(),
        }
    };

    let mut prologue = String::new();
    let mut local_export_prologue = String::new();
    let mut rest = String::new();
    let mut synthetic_count = synthetic_offset;
    let mut imported_bindings = BTreeMap::<String, String>::new();
    let origin_name = format!("__thaw_esm_origin_{synthetic_offset}");
    let mut binding_counter = synthetic_offset;
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
                                    "((typeof {origin_name} !== 'undefined') ? {origin_name}.defaultImport({}, {module_name}) : (({module_name} && {module_name}.__esModule) ? {module_name}.default : {module_name}))",
                                    js_string_literal(&import.src.value.to_string_lossy())
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

    let resolution_name = format!("__thaw_esm_resolution_{synthetic_offset}");
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
                prologue.push_str(&format!("if (typeof {origin_name} !== 'undefined') {origin_name}.edge({}, {var_name});\n", js_string_literal(&spec)));
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
                let (text, local_name) = match &default_decl.decl {
                    DefaultDecl::Fn(f) => (snippet(f.span()), f.ident.as_ref().map(|id| id.sym.to_string())),
                    DefaultDecl::Class(c) => (snippet(c.span()), c.ident.as_ref().map(|id| id.sym.to_string())),
                    DefaultDecl::TsInterfaceDecl(_) => (None, None),
                };
                if let Some(text) = text {
                    if let Some(name) = local_name {
                        rest.push_str(&format!("{text}\n"));
                        local_export_prologue.push_str(&format!(
                            "Object.defineProperty(exports, \"default\", {{ enumerable: true, get: function() {{ return {name}; }} }});\n"
                        ));
                    } else {
                        rest.push_str(&format!("module.exports.default = {text};\n"));
                    }
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
                prologue.push_str(&format!("if (typeof {origin_name} !== 'undefined') {origin_name}.edge({}, {var_name});\n", js_string_literal(&spec)));
                    for spec in &named.specifiers {
                        if let ExportSpecifier::Named(n) = spec {
                            let orig = export_name(&n.orig);
                            let exported = n
                                .exported
                                .as_ref()
                                .map(&export_name)
                                .unwrap_or_else(|| orig.clone());
                            local_export_prologue.push_str(&format!(
                                "Object.defineProperty(exports, {}, {{ enumerable: true, get: function() {{ if (typeof {var_name} === 'undefined') throw new ReferenceError('Export is not initialized'); return {var_name}[{}]; }} }});\n",
                                js_string_literal(&exported),
                                js_string_literal(&orig)
                            ));
                        } else if let ExportSpecifier::Namespace(namespace) = spec {
                            local_export_prologue.push_str(&format!(
                                "Object.defineProperty(exports, {}, {{ enumerable: true, get: function() {{ if (typeof {var_name} === 'undefined') throw new ReferenceError('Export is not initialized'); return {var_name}; }} }});\n",
                                js_string_literal(&export_name(&namespace.name))
                            ));
                        }
                        // `export v from './y'` remains unsupported.
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
                prologue.push_str(&format!("if (typeof {origin_name} !== 'undefined') {origin_name}.edge({}, {var_name});\n", js_string_literal(&spec)));
            }
            // `import foo = require(...)`/`export = foo`/`export as
            // namespace`: TS-only forms that shouldn't appear in real
            // runtime `.js` files; skip gracefully rather than crashing
            // if one somehow does.
            ModuleItem::ModuleDecl(_) => {}
        }
    }

    // Publish statically known ESM star names before a cyclic dependency can
    // observe this partially initialized namespace. Opaque CJS/native names
    // remain deferred until their loader records an edge.
    let star_setup = if module.body.iter().any(|item| matches!(item, ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) if !export.type_only)) {
        format!("if (typeof {origin_name} === 'undefined') throw new TypeError('export * requires bundle origin graph');\n{origin_name}.track(exports);\n")
    } else {
        String::new()
    };
    let generated = format!("module.exports.__esModule = true;\n{local_export_prologue}{star_setup}{prologue}{rest}");
    let generated_name = rewritten_js_source_name(source_name, "CommonJS generation");
    let meta_rewritten = rewrite_import_meta_urls_named(&generated, &generated_name);
    let meta_name = if meta_rewritten != generated {
        rewritten_js_source_name(&generated_name, "import.meta rewrite")
    } else {
        generated_name
    };
    Some(strip_reserved_wrapper_redeclarations_named(&meta_rewritten, &meta_name))
}

/// The original ESM export edges, kept separately from the generated CJS
/// source so star resolution can follow bindings through diamond/cyclic
/// reexports before either module has finished evaluating.
#[cfg(test)]
fn esm_export_graph(source: &str) -> Option<serde_json::Value> {
    esm_export_graph_named(source, &thaw_parser::common::FileName::Custom("input.js".into()))
}

fn esm_export_graph_named(source: &str, source_name: &thaw_parser::common::FileName) -> Option<serde_json::Value> {
    use std::collections::{BTreeMap, BTreeSet};
    use thaw_parser::ast::{Decl, DefaultDecl, ExportSpecifier, ImportSpecifier, ModuleDecl, ModuleExportName, ModuleItem};

    let (module, _) = thaw_parser::parse_javascript_with_source_map_named(source, source_name.clone()).ok()?;
    let name = |value: &ModuleExportName| match value {
        ModuleExportName::Ident(id) => id.sym.to_string(),
        ModuleExportName::Str(text) => text.value.to_string_lossy().into_owned(),
    };
    let mut imported = BTreeMap::<String, (String, Option<String>, bool)>::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(decl)) = item else { continue };
        let source = decl.src.value.to_string_lossy().into_owned();
        for specifier in &decl.specifiers {
            match specifier {
                ImportSpecifier::Named(named) => {
                    let local = named.local.sym.to_string();
                    let remote = named.imported.as_ref().map(&name).unwrap_or_else(|| local.clone());
                    imported.insert(local, (source.clone(), Some(remote), false));
                }
                ImportSpecifier::Default(default) => {
                    imported.insert(default.local.sym.to_string(), (source.clone(), Some("default".to_string()), true));
                }
                ImportSpecifier::Namespace(namespace) => {
                    imported.insert(namespace.local.sym.to_string(), (source.clone(), None, false));
                }
            }
        }
    }
    let mut local = serde_json::Map::new();
    let mut indirect = serde_json::Map::new();
    let mut stars = Vec::new();
    let mut has_esm = false;
    for item in &module.body {
        let ModuleItem::ModuleDecl(decl) = item else { continue };
        has_esm = true;
        match decl {
            ModuleDecl::ExportDecl(export) => {
                let mut names = BTreeSet::new();
                match &export.decl {
                    Decl::Fn(function) => { names.insert(function.ident.sym.to_string()); }
                    Decl::Class(class) => { names.insert(class.ident.sym.to_string()); }
                    Decl::Var(vars) => for variable in &vars.decls { pattern_names(&variable.name, &mut names); },
                    _ => {}
                }
                for binding in names { local.insert(binding.clone(), binding.into()); }
            }
            ModuleDecl::ExportDefaultDecl(default) => {
                let binding = match &default.decl {
                    DefaultDecl::Fn(function) => function.ident.as_ref().map(|id| id.sym.to_string()),
                    DefaultDecl::Class(class) => class.ident.as_ref().map(|id| id.sym.to_string()),
                    DefaultDecl::TsInterfaceDecl(_) => None,
                }.unwrap_or_else(|| "*default*".to_string());
                local.insert("default".to_string(), binding.into());
            }
            ModuleDecl::ExportDefaultExpr(_) => {
                local.insert("default".to_string(), "*default*".into());
            }
            ModuleDecl::ExportNamed(export) if !export.type_only => {
                for specifier in &export.specifiers {
                    match specifier {
                        ExportSpecifier::Named(named) if !named.is_type_only => {
                            let original = name(&named.orig);
                            let exported = named.exported.as_ref().map(&name).unwrap_or_else(|| original.clone());
                            let target = export.src.as_ref().map(|source| (source.value.to_string_lossy().into_owned(), Some(original.clone()), false))
                                .or_else(|| imported.get(&original).cloned());
                            if let Some((source, remote, default_interop)) = target {
                                indirect.insert(exported, serde_json::json!([source, remote, default_interop]));
                            } else {
                                local.insert(exported, original.into());
                            }
                        }
                        ExportSpecifier::Namespace(namespace) => {
                            if let Some(source) = &export.src {
                                indirect.insert(name(&namespace.name), serde_json::json!([source.value.to_string_lossy(), null, false]));
                            }
                        }
                        _ => {}
                    }
                }
            }
            ModuleDecl::ExportAll(export) if !export.type_only => {
                stars.push(serde_json::Value::String(export.src.value.to_string_lossy().into_owned()));
            }
            _ => {}
        }
    }
    has_esm.then(|| serde_json::json!({ "local": local, "indirect": indirect, "stars": stars }))
}

fn esm_origin_parameter_named(source: &str, source_name: &thaw_parser::common::FileName) -> Option<String> {
    esm_export_graph_named(source, source_name)?;
    let dynamic_source = rewrite_dynamic_imports_named(source, source_name);
    let dynamic_name = dynamic_source.as_ref().filter(|rewritten| rewritten.as_str() != source)
        .map(|_| rewritten_js_source_name(source_name, "dynamic import rewrite"));
    let source = dynamic_source.as_deref().unwrap_or(source);
    Some(format!("__thaw_esm_origin_{}", esm_synthetic_offset_named(source, dynamic_name.as_ref().unwrap_or(source_name))?))
}
