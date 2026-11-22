//! Scope-aware rewrite of references to imported bindings, shared by the registry's QuickJS ESM
//! transform and thaw-js's native ESM conversion: every reference to an imported local becomes the
//! replacement expression the caller supplies (a live read of the dependency's export).

use std::collections::{BTreeMap, BTreeSet};

pub fn pattern_names(
    pattern: &thaw_parser::ast::Pat,
    names: &mut std::collections::BTreeSet<String>,
) {
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

/// `source` with every reference to a name of `bindings` (local name -> replacement expression) replaced,
/// skipping references shadowed by parameters, locals, catch/for/switch/class scopes. A named import in
/// `named_bindings` that is called or tagged is detached (`(0, repl)(..)`: no `this`). `assign_targets`
/// also rewrites `x = ..` targets. `None`: nothing to replace. `module`/`cm` are the parse of `source`.
pub fn rewrite_import_references(
    source: &str,
    module: &thaw_parser::ast::Module,
    cm: &thaw_parser::common::SourceMap,
    bindings: &BTreeMap<String, String>,
    named_bindings: &BTreeSet<String>,
    assign_targets: bool,
) -> Option<String> {
    use std::collections::{BTreeMap, BTreeSet};
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        ArrowExpr, ArrowFunctionBody, AssignExpr, AssignTarget, BlockStmt, CallExpr, Callee,
        CatchClause, Class, Decl, Expr, FnExpr, ForHead, ForInStmt, ForOfStmt, ForStmt, Function,
        ModuleDecl, ModuleItem, OptChainBase, OptChainExpr, Prop, SimpleAssignTarget, StaticBlock,
        Stmt, SwitchStmt, TaggedTpl, VarDecl, VarDeclKind, VarDeclOrExpr,
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

    struct References<'a> {
        bindings: &'a BTreeMap<String, String>,
        named_bindings: &'a BTreeSet<String>,
        assign_targets: bool,
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
            self.replacements
                .push((span.lo.0, span.hi.0, format!("(0, {replacement})")));
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
        // `x = ..` / `x += ..` on an import: a member write, which throws on a getter-only export.
        fn visit_assign_expr(&mut self, assign: &AssignExpr) {
            if self.assign_targets {
                if let AssignTarget::Simple(SimpleAssignTarget::Ident(target)) = &assign.left {
                    let name = target.id.sym.as_str();
                    if !self.is_shadowed(name) {
                        if let Some(replacement) = self.bindings.get(name) {
                            let span = target.id.span();
                            self.replacements
                                .push((span.lo.0, span.hi.0, replacement.clone()));
                            assign.right.visit_with(self);
                            return;
                        }
                    }
                }
            }
            assign.visit_children_with(self);
        }

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
                    if declaration.kind != VarDeclKind::Var =>
                {
                    variable_names(declaration)
                }
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
        bindings,
        named_bindings,
        assign_targets,
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
