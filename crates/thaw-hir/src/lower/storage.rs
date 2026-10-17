use swc_common::{Span, Spanned};

#[derive(Default)]
struct StoragePlan {
    dynamic: HashSet<(u32, u32)>,
}

impl StoragePlan {
    fn analyze(module: &Module) -> Result<Self, String> {
        let mut graph = StorageGraph::default();
        module.visit_with(&mut graph);
        let mut pending: Vec<_> = graph.seeds.iter().copied().collect();
        let mut dynamic = graph.seeds;
        while let Some(node) = pending.pop() {
            if let Some(neighbors) = graph.edges.get(&node) {
                for &neighbor in neighbors {
                    if dynamic.insert(neighbor) {
                        pending.push(neighbor);
                    }
                }
            }
        }
        Ok(Self { dynamic })
    }

    fn is_dynamic(&self, span: Span) -> bool {
        self.dynamic.contains(&storage_key(span))
    }
}

fn storage_key(span: Span) -> (u32, u32) {
    (span.lo.0, span.hi.0)
}

fn storage_has_any(ty: &TsType) -> bool {
    struct Any(bool);
    impl Visit for Any {
        fn visit_ts_keyword_type(&mut self, ty: &swc_ecma_ast::TsKeywordType) {
            self.0 |= ty.kind == TsKeywordTypeKind::TsAnyKeyword;
        }
    }
    let mut any = Any(false);
    ty.visit_with(&mut any);
    any.0
}

#[derive(Default)]
struct StorageGraph {
    edges: HashMap<(u32, u32), Vec<(u32, u32)>>,
    seeds: HashSet<(u32, u32)>,
    scopes: Vec<HashMap<String, Span>>,
    functions: Vec<(Span, usize)>,
    classes: Vec<Span>,
}

impl StorageGraph {
    fn link(&mut self, left: Span, right: Span) {
        if left.is_dummy() || right.is_dummy() || left == right {
            return;
        }
        self.edges.entry(storage_key(left)).or_default().push(storage_key(right));
        self.edges.entry(storage_key(right)).or_default().push(storage_key(left));
    }

    fn expose(&mut self, span: Span) {
        if !span.is_dummy() {
            self.seeds.insert(storage_key(span));
        }
    }

    fn bind(&mut self, name: &swc_ecma_ast::Ident) {
        self.scopes.last_mut().unwrap().entry(name.sym.to_string()).or_insert(name.span);
    }

    fn pattern(&mut self, pattern: &Pat) {
        struct Bindings(Vec<Span>);
        impl Visit for Bindings {
            fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
                self.0.push(binding.id.span);
            }
        }
        let mut bindings = Bindings(Vec::new());
        pattern.visit_with(&mut bindings);
        for binding in bindings.0 { self.link(pattern.span(), binding); }
        match pattern {
            Pat::Ident(binding) => self.bind(&binding.id),
            Pat::Array(array) => {
                for item in array.elems.iter().flatten() { self.pattern(item); }
            }
            Pat::Object(object) => {
                for property in &object.props {
                    match property {
                        swc_ecma_ast::ObjectPatProp::KeyValue(property) => self.pattern(&property.value),
                        swc_ecma_ast::ObjectPatProp::Assign(property) => self.bind(&property.key.id),
                        swc_ecma_ast::ObjectPatProp::Rest(rest) => self.pattern(&rest.arg),
                    }
                }
            }
            Pat::Rest(rest) => self.pattern(&rest.arg),
            Pat::Assign(assign) => self.pattern(&assign.left),
            _ => {}
        }
    }

    fn declare(&mut self, declaration: &Decl) {
        match declaration {
            Decl::Var(variable) => {
                for declaration in &variable.decls { self.pattern(&declaration.name); }
            }
            Decl::Fn(function) => self.bind(&function.ident),
            Decl::Class(class) => self.bind(&class.ident),
            _ => {}
        }
    }

    fn lookup(&self, name: &str) -> Option<(Span, usize)> {
        self.scopes.iter().enumerate().rev()
            .find_map(|(depth, scope)| scope.get(name).map(|span| (*span, depth)))
    }

    fn hoist_vars(&mut self, statements: &[Stmt]) {
        struct Hoists(Vec<swc_ecma_ast::Ident>);
        impl Visit for Hoists {
            fn visit_var_decl(&mut self, variable: &VarDecl) {
                if variable.kind != VarDeclKind::Var { return; }
                struct Names<'a>(&'a mut Vec<swc_ecma_ast::Ident>);
                impl Visit for Names<'_> {
                    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
                        self.0.push(binding.id.clone());
                    }
                    fn visit_expr(&mut self, _: &Expr) {}
                }
                for declaration in &variable.decls { declaration.name.visit_with(&mut Names(&mut self.0)); }
            }
            fn visit_function(&mut self, _: &swc_ecma_ast::Function) {}
            fn visit_arrow_expr(&mut self, _: &swc_ecma_ast::ArrowExpr) {}
            fn visit_class(&mut self, _: &swc_ecma_ast::Class) {}
        }
        let mut hoists = Hoists(Vec::new());
        for statement in statements { statement.visit_with(&mut hoists); }
        for name in hoists.0 { self.bind(&name); }
    }

    fn annotation(&mut self, span: Span, annotation: Option<&swc_ecma_ast::TsTypeAnn>) {
        if annotation.is_some_and(|annotation| storage_has_any(&annotation.type_ann)) {
            self.expose(span);
        }
    }
}

impl Visit for StorageGraph {
    fn visit_module(&mut self, module: &Module) {
        self.scopes.push(HashMap::new());
        for item in &module.body {
            match item {
                ModuleItem::Stmt(Stmt::Decl(declaration)) => self.declare(declaration),
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => self.declare(&export.decl),
                _ => {}
            }
        }
        module.visit_children_with(self);
        self.scopes.pop();
    }

    fn visit_block_stmt(&mut self, block: &swc_ecma_ast::BlockStmt) {
        self.scopes.push(HashMap::new());
        for statement in &block.stmts {
            if let Stmt::Decl(declaration) = statement { self.declare(declaration); }
        }
        block.visit_children_with(self);
        self.scopes.pop();
    }

    fn visit_function_body(&mut self, body: &swc_ecma_ast::FunctionBody) {
        self.scopes.push(HashMap::new());
        for statement in &body.stmts {
            if let Stmt::Decl(declaration) = statement { self.declare(declaration); }
        }
        body.visit_children_with(self);
        self.scopes.pop();
    }

    fn visit_ident(&mut self, identifier: &swc_ecma_ast::Ident) {
        if let Some((binding, depth)) = self.lookup(identifier.sym.as_ref()) {
            self.link(identifier.span, binding);
            if let Some(&(function, local_depth)) = self.functions.last() {
                if depth < local_depth {
                    self.link(function, binding);
                }
            }
        }
    }

    fn visit_var_declarator(&mut self, declaration: &swc_ecma_ast::VarDeclarator) {
        self.pattern(&declaration.name);
        if let Some(init) = &declaration.init { self.link(declaration.name.span(), init.span()); }
        declaration.visit_children_with(self);
    }

    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
        self.link(binding.span(), binding.id.span);
        self.annotation(binding.id.span, binding.type_ann.as_deref());
        binding.visit_children_with(self);
    }

    fn visit_fn_decl(&mut self, declaration: &FnDecl) {
        self.link(declaration.ident.span, declaration.function.span);
        if declaration.function.body.is_none() && dynamic_symbol(declaration.ident.sym.as_ref()).is_some() {
            self.expose(declaration.ident.span);
        }
        declaration.visit_children_with(self);
    }

    fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
        self.scopes.push(HashMap::new());
        self.functions.push((function.span, self.scopes.len() - 1));
        self.annotation(function.span, function.return_type.as_deref());
        for parameter in &function.params {
            self.pattern(&parameter.pat);
            self.link(function.span, parameter.pat.span());
        }
        if let Some(body) = &function.body { self.hoist_vars(&body.stmts); }
        function.visit_children_with(self);
        self.functions.pop();
        self.scopes.pop();
    }

    fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
        self.scopes.push(HashMap::new());
        self.functions.push((arrow.span, self.scopes.len() - 1));
        self.annotation(arrow.span, arrow.return_type.as_deref());
        for parameter in &arrow.params {
            self.pattern(parameter);
            self.link(arrow.span, parameter.span());
        }
        if let ArrowFunctionBody::Expr(body) = arrow.body.as_ref() { self.link(arrow.span, body.span()); }
        if let ArrowFunctionBody::FunctionBody(body) = arrow.body.as_ref() { self.hoist_vars(&body.stmts); }
        arrow.visit_children_with(self);
        self.functions.pop();
        self.scopes.pop();
    }

    fn visit_return_stmt(&mut self, statement: &swc_ecma_ast::ReturnStmt) {
        if let (Some(&(function, _)), Some(value)) = (self.functions.last(), &statement.arg) {
            self.link(function, value.span());
        }
        statement.visit_children_with(self);
    }

    fn visit_catch_clause(&mut self, catch: &swc_ecma_ast::CatchClause) {
        self.scopes.push(HashMap::new());
        if let Some(parameter) = &catch.param { self.pattern(parameter); }
        catch.visit_children_with(self);
        self.scopes.pop();
    }

    fn visit_throw_stmt(&mut self, statement: &swc_ecma_ast::ThrowStmt) {
        self.expose(statement.arg.span());
        statement.visit_children_with(self);
    }

    fn visit_getter_prop(&mut self, getter: &swc_ecma_ast::GetterProp) {
        self.link(getter.span, getter.function.span);
        getter.visit_children_with(self);
    }

    fn visit_setter_prop(&mut self, setter: &swc_ecma_ast::SetterProp) {
        self.link(setter.span, setter.function.span);
        setter.visit_children_with(self);
    }

    fn visit_class_decl(&mut self, declaration: &ClassDecl) {
        self.link(declaration.ident.span, declaration.class.span);
        self.classes.push(declaration.class.span);
        declaration.visit_children_with(self);
        self.classes.pop();
    }

    fn visit_class_method(&mut self, method: &ClassMethod) {
        if let Some(&class) = self.classes.last() { self.link(class, method.function.span); }
        method.visit_children_with(self);
    }

    fn visit_class_prop(&mut self, property: &ClassProp) {
        if let Some(&class) = self.classes.last() {
            self.link(class, property.span);
        }
        if let Some(value) = &property.value { self.link(property.span, value.span()); }
        self.annotation(property.span, property.type_ann.as_deref());
        property.visit_children_with(self);
    }

    fn visit_expr(&mut self, expression: &Expr) {
        let span = expression.span();
        match expression {
            Expr::Array(array) => {
                for element in array.elems.iter().flatten() { self.link(span, element.expr.span()); }
            }
            Expr::Object(object) => {
                for property in &object.props {
                    match property {
                        PropOrSpread::Spread(spread) => self.link(span, spread.expr.span()),
                        PropOrSpread::Prop(property) => match property.as_ref() {
                            Prop::KeyValue(property) => self.link(span, property.value.span()),
                            Prop::Shorthand(identifier) => self.link(span, identifier.span),
                            Prop::Method(method) => self.link(span, method.function.span),
                            Prop::Getter(getter) => self.link(span, getter.span),
                            Prop::Setter(setter) => self.link(span, setter.span),
                            Prop::Assign(assign) => self.link(span, assign.value.span()),
                        }
                    }
                }
            }
            Expr::Member(member) => self.link(span, member.obj.span()),
            Expr::Assign(assign) => {
                self.link(span, assign.left.span());
                self.link(span, assign.right.span());
            }
            Expr::Call(call) => {
                if let Callee::Expr(callee) = &call.callee {
                    self.link(span, callee.span());
                    for argument in &call.args { self.link(callee.span(), argument.expr.span()); }
                }
            }
            Expr::New(new) => {
                self.link(span, new.callee.span());
                if matches!(new.callee.as_ref(), Expr::Ident(name)
                    if matches!(name.sym.as_ref(), "Proxy" | "Function") && self.lookup(name.sym.as_ref()).is_none()) {
                    self.expose(span);
                }
                for argument in new.args.iter().flatten() { self.link(span, argument.expr.span()); }
            }
            Expr::Cond(conditional) => {
                self.link(span, conditional.cons.span());
                self.link(span, conditional.alt.span());
            }
            Expr::Paren(parenthesized) => self.link(span, parenthesized.expr.span()),
            Expr::TsAs(assertion) => {
                self.link(span, assertion.expr.span());
                if storage_has_any(&assertion.type_ann) { self.expose(span); }
            }
            Expr::TsTypeAssertion(assertion) => {
                self.link(span, assertion.expr.span());
                if storage_has_any(&assertion.type_ann) { self.expose(span); }
            }
            Expr::TsNonNull(assertion) => self.link(span, assertion.expr.span()),
            Expr::Await(awaited) => self.link(span, awaited.arg.span()),
            Expr::Fn(function) => self.link(span, function.function.span),
            Expr::Seq(sequence) => {
                if let Some(last) = sequence.exprs.last() { self.link(span, last.span()); }
            }
            Expr::This(_) => {
                if let Some(&class) = self.classes.last() { self.link(span, class); }
            }
            _ => {}
        }
        expression.visit_children_with(self);
    }
}
