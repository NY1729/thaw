use swc_common::{Span, Spanned};

#[derive(Default)]
struct StoragePlan {
    dynamic: HashSet<(u32, u32)>,
}

impl StoragePlan {
    fn analyze(module: &Module) -> Result<Self, String> {
        let mut graph = StorageGraph::default();
        module.visit_with(&mut graph);
        graph.connect_calls();
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
            if let Some(callable) = graph.callables.get(&node) {
                for &target in &callable.escaped {
                    if dynamic.insert(target) { pending.push(target); }
                }
            }
        }
        Ok(Self { dynamic })
    }

    fn result_is_dynamic(&self, span: Span) -> bool {
        self.dynamic.contains(&(span.lo.0, 0))
    }

    fn is_dynamic(&self, span: Span) -> bool {
        self.dynamic.contains(&storage_key(span))
    }
}

fn storage_key(span: Span) -> (u32, u32) {
    (span.lo.0, span.hi.0)
}

fn storage_has_any(ty: &TsType) -> bool {
    matches!(ty, TsType::TsKeywordType(keyword)
        if matches!(keyword.kind, TsKeywordTypeKind::TsAnyKeyword | TsKeywordTypeKind::TsUnknownKeyword))
}

struct StorageCallable {
    params: Vec<Span>,
    result: (u32, u32),
    escaped: Vec<(u32, u32)>,
}
struct StorageCall {
    callee: Span,
    arguments: Vec<Span>,
    result: Span,
}
#[derive(Default)]
struct StorageGraph {
    edges: HashMap<(u32, u32), Vec<(u32, u32)>>,
    seeds: HashSet<(u32, u32)>,
    scopes: Vec<HashMap<String, Span>>,
    functions: Vec<(Span, usize)>,
    classes: Vec<Span>,
    callables: HashMap<(u32, u32), StorageCallable>,
    calls: Vec<StorageCall>,
    captures: HashSet<((u32, u32), (u32, u32))>,
    exceptions: Vec<Span>,
}

impl StorageGraph {
    fn link(&mut self, left: Span, right: Span) {
        if left.is_dummy() || right.is_dummy() || left == right {
            return;
        }
        self.edges.entry(storage_key(left)).or_default().push(storage_key(right));
        self.edges.entry(storage_key(right)).or_default().push(storage_key(left));
    }

    fn contain(&mut self, owner: Span, value: Span) {
        if !owner.is_dummy() && !value.is_dummy() {
            self.edges.entry(storage_key(owner)).or_default().push(storage_key(value));
        }
    }

    fn link_keys(&mut self, left: (u32, u32), right: (u32, u32)) {
        self.edges.entry(left).or_default().push(right);
        self.edges.entry(right).or_default().push(left);
    }
    fn connect_calls(&mut self) {
        let mut connected = HashSet::new();
        loop {
            let mut links = Vec::new();
            for (index, call) in self.calls.iter().enumerate() {
                let mut pending = vec![storage_key(call.callee)];
                let mut seen = HashSet::new();
                while let Some(node) = pending.pop() {
                    if !seen.insert(node) { continue; }
                    if let Some(callable) = self.callables.get(&node) {
                        if connected.insert((index, node)) {
                            links.push((storage_key(call.result), callable.result));
                            for (actual, formal) in call.arguments.iter().zip(&callable.params) {
                                links.push((storage_key(*actual), storage_key(*formal)));
                            }
                        }
                    }
                    if let Some(neighbors) = self.edges.get(&node) {
                        pending.extend(neighbors.iter().copied().filter(|next| !self.captures.contains(&(node, *next))));
                    }
                }
            }
            if links.is_empty() { break; }
            for (left, right) in links { self.link_keys(left, right); }
        }
    }
    fn callable(&mut self, span: Span, parameters: &[Pat], result: Option<&TsType>) {
        let result_node = (span.lo.0, 0);
        let mut escaped = Vec::new();
        if result.is_none_or(storage_reference_type) { escaped.push(result_node); }
        for parameter in parameters {
            if matches!(parameter, Pat::Ident(binding)
                if binding.type_ann.as_ref().is_some_and(|ann| storage_reference_type(&ann.type_ann))) {
                escaped.push(storage_key(parameter.span()));
            }
        }
        self.callables.insert(storage_key(span), StorageCallable {
            params: parameters.iter().map(Spanned::span).collect(), result: result_node, escaped,
        });
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
                    self.contain(function, binding);
                    self.captures.insert((storage_key(function), storage_key(binding)));
                    self.captures.insert((storage_key(binding), storage_key(function)));
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
        self.callable(function.span,
            &function.params.iter().map(|p| p.pat.clone()).collect::<Vec<_>>(),
            function.return_type.as_ref().map(|ann| ann.type_ann.as_ref()));
        if function.return_type.as_ref().is_some_and(|ann| storage_has_any(&ann.type_ann)) {
            self.seeds.insert((function.span.lo.0, 0));
        }
        for parameter in &function.params {
            self.pattern(&parameter.pat);

        }
        if let Some(body) = &function.body { self.hoist_vars(&body.stmts); }
        function.visit_children_with(self);
        self.functions.pop();
        self.scopes.pop();
    }

    fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
        self.scopes.push(HashMap::new());
        self.functions.push((arrow.span, self.scopes.len() - 1));
        self.callable(arrow.span, &arrow.params,
            arrow.return_type.as_ref().map(|ann| ann.type_ann.as_ref()));
        if arrow.return_type.as_ref().is_some_and(|ann| storage_has_any(&ann.type_ann)) {
            self.seeds.insert((arrow.span.lo.0, 0));
        }
        for parameter in &arrow.params {
            self.pattern(parameter);

        }
        if let ArrowFunctionBody::Expr(body) = arrow.body.as_ref() { self.link_keys((arrow.span.lo.0, 0), storage_key(body.span())); }
        if let ArrowFunctionBody::FunctionBody(body) = arrow.body.as_ref() { self.hoist_vars(&body.stmts); }
        arrow.visit_children_with(self);
        self.functions.pop();
        self.scopes.pop();
    }

    fn visit_return_stmt(&mut self, statement: &swc_ecma_ast::ReturnStmt) {
        if let (Some(&(function, _)), Some(value)) = (self.functions.last(), &statement.arg) {
            self.link_keys((function.lo.0, 0), storage_key(value.span()));
        }
        statement.visit_children_with(self);
    }

    fn visit_try_stmt(&mut self, statement: &swc_ecma_ast::TryStmt) {
        let exception = statement.handler.as_ref().and_then(|catch| catch.param.as_ref()).map(Spanned::span);
        if let Some(exception) = exception { self.exceptions.push(exception); }
        statement.block.visit_with(self);
        if exception.is_some() { self.exceptions.pop(); }
        statement.handler.visit_with(self);
        statement.finalizer.visit_with(self);
    }

    fn visit_catch_clause(&mut self, catch: &swc_ecma_ast::CatchClause) {
        self.scopes.push(HashMap::new());
        if let Some(parameter) = &catch.param { self.pattern(parameter); }
        catch.visit_children_with(self);
        self.scopes.pop();
    }

    fn visit_throw_stmt(&mut self, statement: &swc_ecma_ast::ThrowStmt) {
        if let Some(&exception) = self.exceptions.last() { self.link(exception, statement.arg.span()); }
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
            self.contain(class, property.span);
        }
        if let Some(value) = &property.value { self.link(property.span, value.span()); }
        self.annotation(property.span, property.type_ann.as_deref());
        property.visit_children_with(self);
    }

    fn visit_expr(&mut self, expression: &Expr) {
        let span = expression.span();
        match expression {
            Expr::Array(array) => {
                for element in array.elems.iter().flatten() { self.contain(span, element.expr.span()); }
            }
            Expr::Object(object) => {
                for property in &object.props {
                    match property {
                        PropOrSpread::Spread(spread) => self.contain(span, spread.expr.span()),
                        PropOrSpread::Prop(property) => match property.as_ref() {
                            Prop::KeyValue(property) => self.contain(span, property.value.span()),
                            Prop::Shorthand(identifier) => self.contain(span, identifier.span),
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
                    self.calls.push(StorageCall { callee: callee.span(),
                        arguments: call.args.iter().map(|arg| arg.expr.span()).collect(), result: span });
                    if let Expr::Ident(name) = callee.as_ref() {
                        if dynamic_symbol(name.sym.as_ref()).is_some() {
                            for argument in &call.args { self.expose(argument.expr.span()); }
                        }
                    }
                    if let Expr::Member(member) = callee.as_ref() {
                        self.link(span, member.obj.span());
                        for argument in &call.args { self.contain(member.obj.span(), argument.expr.span()); }
                    }
                }
            }
            Expr::New(new) => {
                self.link(span, new.callee.span());
                if matches!(new.callee.as_ref(), Expr::Ident(name)
                    if matches!(name.sym.as_ref(), "Proxy" | "Function") && self.lookup(name.sym.as_ref()).is_none()) {
                    self.expose(span);
                }
                for argument in new.args.iter().flatten() { self.contain(span, argument.expr.span()); }
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

thread_local! {
    static ACTIVE_STORAGE_PLAN: RefCell<StoragePlan> = RefCell::new(StoragePlan::default());
}

struct StorageScope(StoragePlan);

impl StorageScope {
    fn enter(module: &Module) -> Result<Self, String> {
        let plan = StoragePlan::analyze(module)?;
        Ok(Self(ACTIVE_STORAGE_PLAN.with(|active| active.replace(plan))))
    }
}

impl Drop for StorageScope {
    fn drop(&mut self) {
        ACTIVE_STORAGE_PLAN.with(|active| active.replace(std::mem::take(&mut self.0)));
    }
}

fn has_dynamic_storage(span: Span) -> bool {
    ACTIVE_STORAGE_PLAN.with(|active| active.borrow().is_dynamic(span))
}

fn storage_type(span: Span, source: HirType) -> HirType {
    if !has_dynamic_storage(span) { return source; }
    dynamic_storage_type(source)
}

fn dynamic_storage_type(source: HirType) -> HirType {
    match source {
        HirType::Void | HirType::Dynamic => source,
        // A native class retains its authenticated identity; its mutable fields
        // are selected independently before the class layout is allocated.
        HirType::Object(_) if class_name_from_type(&source).is_some() => source,
        HirType::Promise(result) => HirType::Promise(Box::new(dynamic_storage_type(*result))),
        HirType::Function(..) | HirType::CallableFunction(..) | HirType::FunctionWithThis(..)
        | HirType::JsValue => source,
        _ => HirType::Json,
    }
}

impl FnLowerer<'_> {
    fn lower_dynamic_storage_array(&mut self, array: &swc_ecma_ast::ArrayLit) -> Result<HirExpr, String> {
        let mut parts = Vec::new();
        for element in &array.elems {
            let value = match element {
                None => HirExpr::Lit(HirLit::ArrayHole),
                Some(element) => {
                    let value = self.lower_expr(&element.expr)?;
                    if element.spread.is_some() {
                        let value = self.coerce_to_declared_aliasing(&HirType::Json, value)?;
                        parts.push(HirExpr::JsonAsNative(Box::new(value), HirType::Array(Box::new(HirType::Json))));
                        continue;
                    }
                    self.coerce_to_declared_aliasing(&HirType::Json, value)?
                }
            };
            parts.push(HirExpr::TypedClosure(
                HirType::Array(Box::new(HirType::Json)), Box::new(HirExpr::ArrayLit(vec![value])),
            ));
        }
        let native = HirExpr::ArrayConcat(parts, HirType::Json);
        let array = self.wrap_native_value_as_json(native, HirType::Array(Box::new(HirType::Json)))?;
        let arguments = self.wrap_native_value_as_json(HirExpr::ArrayLit(vec![array]), HirType::Array(Box::new(HirType::Json)))?;
        let live = HirExpr::Call(Box::new(HirExpr::Var("callDynamicValueMixedHandle".into())), vec![
            HirExpr::Call(Box::new(HirExpr::Var("getDynamicValue".into())),
                vec![HirExpr::Lit(HirLit::Str("__thaw_storage_array".into()))]),
            arguments, HirExpr::ArrayLit(Vec::new()),
        ]);
        Ok(HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_host_from_dynamic".into())), vec![live]))
    }
}

fn storage_reference_type(ty: &TsType) -> bool {
    !matches!(ty, TsType::TsKeywordType(keyword)
        if !matches!(keyword.kind, TsKeywordTypeKind::TsAnyKeyword | TsKeywordTypeKind::TsObjectKeyword | TsKeywordTypeKind::TsUnknownKeyword))
        && !matches!(ty, TsType::TsFnOrConstructorType(_))
}
fn storage_return_type(span: Span, source: HirType) -> HirType {
    if ACTIVE_STORAGE_PLAN.with(|active| active.borrow().result_is_dynamic(span)) {
        if matches!(source, HirType::Function(..) | HirType::CallableFunction(..)) {
            HirType::Dynamic
        } else { dynamic_storage_type(source) }
    } else { source }
}

impl FnLowerer<'_> {
    fn lower_dynamic_storage_constructor(&mut self, new: &swc_ecma_ast::NewExpr) -> Result<HirExpr, String> {
        let Expr::Ident(name) = new.callee.as_ref() else { unreachable!() };
        let mut arguments = Vec::new();
        for argument in new.args.iter().flatten() {
            let value = self.lower_expr(&argument.expr)?;
            if argument.spread.is_some() {
                let value = self.coerce_to_declared_aliasing(&HirType::Json, value)?;
                arguments.push(HirExpr::JsonAsNative(Box::new(value), HirType::Array(Box::new(HirType::Json))));
            } else {
                let value = self.coerce_to_declared_aliasing(&HirType::Json, value)?;
                arguments.push(HirExpr::TypedClosure(HirType::Array(Box::new(HirType::Json)), Box::new(HirExpr::ArrayLit(vec![value]))));
            }
        }
        let arguments = self.wrap_native_value_as_json(HirExpr::ArrayConcat(arguments, HirType::Json), HirType::Array(Box::new(HirType::Json)))?;
        let value = HirExpr::Call(Box::new(HirExpr::Var("constructDynamicValue".into())), vec![
            HirExpr::Call(Box::new(HirExpr::Var("getDynamicValue".into())), vec![HirExpr::Lit(HirLit::Str(name.sym.to_string()))]), arguments,
        ]);
        Ok(HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_host_from_dynamic".into())), vec![value]))
    }
}
