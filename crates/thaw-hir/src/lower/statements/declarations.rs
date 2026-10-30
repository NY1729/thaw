impl<'a> FnLowerer<'a> {
    fn initializer_callback_references(expression: &Expr, name: &str) -> bool {
        use swc_ecma_visit::{Visit, VisitWith};

        struct Finder<'a> {
            name: &'a str,
            found: bool,
        }

        impl Visit for Finder<'_> {
            fn visit_ident(&mut self, identifier: &swc_ecma_ast::Ident) {
                self.found |= identifier.sym == self.name;
            }
        }

        let Expr::Call(call) = expression else {
            return false;
        };
        call.args.iter().any(|argument| {
            if !matches!(argument.expr.as_ref(), Expr::Arrow(_) | Expr::Fn(_)) {
                return false;
            }
            let mut finder = Finder { name, found: false };
            argument.expr.visit_with(&mut finder);
            finder.found
        })
    }

    fn initializer_closure_references(expression: &Expr, name: &str) -> bool {
        use swc_ecma_visit::{Visit, VisitWith};

        fn pattern_binds(pattern: &Pat, name: &str) -> bool {
            let mut names = Vec::new();
            FnLowerer::collect_pattern_bindings(pattern, &mut names);
            names.iter().any(|bound| bound == name)
        }
        fn declarations_shadow(statements: &[Stmt], name: &str) -> bool {
            statements.iter().any(|statement| match statement {
                Stmt::Decl(Decl::Var(declaration)) => declaration.decls.iter()
                    .any(|item| pattern_binds(&item.name, name)),
                Stmt::Decl(Decl::Using(declaration)) => declaration.decls.iter()
                    .any(|item| pattern_binds(&item.name, name)),
                Stmt::Decl(Decl::Fn(function)) => function.ident.sym == name,
                Stmt::Decl(Decl::Class(class)) => class.ident.sym == name,
                _ => false,
            })
        }
        struct Finder<'a> { name: &'a str, found: bool }
        impl Visit for Finder<'_> {
            fn visit_expr(&mut self, expression: &Expr) {
                if let Expr::Ident(identifier) = expression {
                    self.found |= identifier.sym == self.name;
                } else {
                    expression.visit_children_with(self);
                }
            }
            fn visit_simple_assign_target(&mut self, target: &swc_ecma_ast::SimpleAssignTarget) {
                if let swc_ecma_ast::SimpleAssignTarget::Ident(binding) = target {
                    self.found |= binding.id.sym == self.name;
                } else {
                    target.visit_children_with(self);
                }
            }
            fn visit_prop(&mut self, property: &swc_ecma_ast::Prop) {
                if let swc_ecma_ast::Prop::Shorthand(identifier) = property {
                    self.found |= identifier.sym == self.name;
                } else {
                    property.visit_children_with(self);
                }
            }
            fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
                if !arrow.params.iter().any(|item| pattern_binds(item, self.name)) {
                    arrow.body.visit_with(self);
                }
            }
            fn visit_fn_expr(&mut self, function: &swc_ecma_ast::FnExpr) {
                if function.ident.as_ref().is_some_and(|id| id.sym == self.name) { return; }
                function.function.visit_with(self);
            }
            fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
                if !function.params.iter().any(|item| pattern_binds(&item.pat, self.name)) {
                    function.body.visit_with(self);
                }
            }
            fn visit_block_stmt(&mut self, block: &swc_ecma_ast::BlockStmt) {
                if !declarations_shadow(&block.stmts, self.name) {
                    block.visit_children_with(self);
                }
            }
            fn visit_catch_clause(&mut self, catch: &swc_ecma_ast::CatchClause) {
                if !catch.param.as_ref().is_some_and(|item| pattern_binds(item, self.name)) {
                    catch.visit_children_with(self);
                }
            }
            fn visit_for_stmt(&mut self, statement: &swc_ecma_ast::ForStmt) {
                if !matches!(statement.init.as_ref(), Some(swc_ecma_ast::VarDeclOrExpr::VarDecl(declaration))
                    if declaration.decls.iter().any(|item| pattern_binds(&item.name, self.name))) {
                    statement.visit_children_with(self);
                }
            }
            fn visit_for_in_stmt(&mut self, statement: &swc_ecma_ast::ForInStmt) {
                if matches!(&statement.left, swc_ecma_ast::ForHead::VarDecl(declaration)
                    if declaration.decls.iter().any(|item| pattern_binds(&item.name, self.name))) {
                    statement.right.visit_with(self);
                } else { statement.visit_children_with(self); }
            }
            fn visit_for_of_stmt(&mut self, statement: &swc_ecma_ast::ForOfStmt) {
                if matches!(&statement.left, swc_ecma_ast::ForHead::VarDecl(declaration)
                    if declaration.decls.iter().any(|item| pattern_binds(&item.name, self.name))) {
                    statement.right.visit_with(self);
                } else { statement.visit_children_with(self); }
            }
        }
        let mut finder = Finder { name, found: false };
        match expression {
            Expr::Arrow(arrow) => {
                if arrow.params.iter().any(|item| pattern_binds(item, name)) { return false; }
                arrow.body.visit_with(&mut finder);
            }
            Expr::Fn(function) => {
                if function.ident.as_ref().is_some_and(|id| id.sym == name)
                    || function.function.params.iter().any(|item| pattern_binds(&item.pat, name)) {
                    return false;
                }
                function.function.body.visit_with(&mut finder);
            }
            _ => return false,
        }
        finder.found
    }

    /// `using x = expr;` / `await using x = expr;` bind `x` like a `const`;
    /// the enclosing block's disposal (see `lower_stmts`) is what runs
    /// `x[Symbol.dispose]()` / `await x[Symbol.asyncDispose]()`.
    fn lower_using_decl(&mut self, using_decl: &UsingDecl) -> Result<Vec<HirStmt>, String> {
        let synthetic = VarDecl {
            span: using_decl.span,
            ctxt: swc_common::SyntaxContext::empty(),
            kind: VarDeclKind::Const,
            declare: false,
            decls: using_decl.decls.clone(),
        };
        self.lower_var_decl(&synthetic)
    }

    /// The statement that disposes a `using` binding: `x[Symbol.dispose]()`
    /// for `using`, `await x[Symbol.asyncDispose]()` for `await using`.
    fn lower_using_disposal(
        &mut self,
        source_name: &str,
        is_await: bool,
    ) -> Result<HirStmt, String> {
        let span = swc_common::DUMMY_SP;
        let symbol_name = if is_await { "asyncDispose" } else { "dispose" };
        let member = MemberExpr {
            span,
            obj: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                source_name.into(),
                span,
            ))),
            prop: MemberProp::Computed(ComputedPropName {
                span,
                expr: Box::new(Expr::Member(MemberExpr {
                    span,
                    obj: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                        "Symbol".into(),
                        span,
                    ))),
                    prop: MemberProp::Ident(IdentName::new(symbol_name.into(), span)),
                })),
            }),
        };
        let call = Expr::Call(CallExpr {
            span,
            ctxt: swc_common::SyntaxContext::empty(),
            callee: Callee::Expr(Box::new(Expr::Member(member))),
            args: Vec::new(),
            type_args: None,
        });
        // `using x = null` / `undefined` is allowed and disposes nothing.
        let binding = self.resolve_binding(source_name);
        let nullish = |ty: &HirType| matches!(ty, HirType::Null | HirType::Undefined);
        match self.scope.get(&binding).cloned() {
            Some(ty) if nullish(&ty) => {
                return Ok(HirStmt::Expr(HirExpr::Lit(HirLit::Undefined)));
            }
            Some(ty @ (HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_))) => {
                // The comparison form each absence wrapper narrows on.
                let (op, absent) = match ty {
                    HirType::Optional(_) => (
                        BinaryOp::NotEqEq,
                        Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt("undefined".into(), span)),
                    ),
                    HirType::Nullable(_) => (
                        BinaryOp::NotEqEq,
                        Expr::Lit(swc_ecma_ast::Lit::Null(swc_ecma_ast::Null { span })),
                    ),
                    _ => (
                        BinaryOp::NotEq,
                        Expr::Lit(swc_ecma_ast::Lit::Null(swc_ecma_ast::Null { span })),
                    ),
                };
                let statement = Stmt::If(swc_ecma_ast::IfStmt {
                    span,
                    test: Box::new(Expr::Bin(swc_ecma_ast::BinExpr {
                        span,
                        op,
                        left: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                            source_name.into(),
                            span,
                        ))),
                        right: Box::new(absent),
                    })),
                    cons: Box::new(Stmt::Expr(swc_ecma_ast::ExprStmt {
                        span,
                        expr: Box::new(if is_await {
                            Expr::Await(AwaitExpr { span, arg: Box::new(call) })
                        } else {
                            call
                        }),
                    })),
                    alt: None,
                });
                let statements = self.lower_stmt_seq(&statement)?;
                return Ok(HirStmt::If(
                    HirExpr::Lit(HirLit::Bool(true)),
                    statements,
                    Vec::new(),
                ));
            }
            _ => {}
        }
        let value = if is_await {
            self.lower_expr(&Expr::Await(AwaitExpr {
                span,
                arg: Box::new(call),
            }))?
        } else {
            self.lower_expr(&call)?
        };
        Ok(HirStmt::Expr(value))
    }

    /// The HIR binding a repeated `var name` declaration must reuse: `var`
    /// is function-scoped, so a second declaration of the same name in the
    /// same function is the same variable (an initializer assigns to it).
    fn reusable_var_binding(&self, var_decl: &VarDecl, pattern: &Pat) -> Option<Symbol> {
        let Pat::Ident(binding) = pattern else { return None };
        if var_decl.kind != VarDeclKind::Var || binding.type_ann.is_some() {
            return None;
        }
        let name = binding.id.sym.to_string();
        let owned = self.function_var_names.get(&name)?;
        (self.resolve_binding(&name) == *owned && self.scope.contains_key(owned))
            .then(|| owned.clone())
    }

    fn lower_var_decl(&mut self, var_decl: &VarDecl) -> Result<Vec<HirStmt>, String> {
        if var_decl.kind != VarDeclKind::Var {
            return self.lower_var_decl_inner(var_decl);
        }
        let mut statements = Vec::new();
        for declarator in &var_decl.decls {
            if let Some(owned) = self.reusable_var_binding(var_decl, &declarator.name) {
                if let Some(init) = &declarator.init {
                    let value = self.lower_expr(init)?;
                    let declared = self.scope.get(&owned).cloned().ok_or("unknown `var` binding")?;
                    let value = self.coerce_to_declared(&declared, value)?;
                    self.record_binding_write(&owned);
                    statements.push(HirStmt::Expr(HirExpr::Assign(owned, Box::new(value))));
                }
                continue;
            }
            let mut single = var_decl.clone();
            single.decls = vec![declarator.clone()];
            statements.extend(self.lower_var_decl_inner(&single)?);
            if let Pat::Ident(binding) = &declarator.name {
                let name = binding.id.sym.to_string();
                let owned = self.resolve_binding(&name);
                self.function_var_names.insert(name, owned);
            }
        }
        Ok(statements)
    }

    fn deferred_generic_bind_call<'b>(
        &self,
        expression: &'b Expr,
    ) -> Option<(&'b swc_ecma_ast::CallExpr, GenericCallableTemplate)> {
        let Expr::Call(call) = expression else { return None };
        let Callee::Expr(callee) = &call.callee else { return None };
        let Expr::Member(member) = callee.as_ref() else { return None };
        let Expr::Ident(template) = member.obj.as_ref() else { return None };
        let MemberProp::Ident(method) = &member.prop else { return None };
        if method.sym != *"bind" { return None; }
        self.generic_callable_template(template.sym.as_ref()).map(|target| (call, target))
    }

    fn lower_deferred_generic_bind(
        &mut self,
        name: &str,
        call: &swc_ecma_ast::CallExpr,
        target: GenericCallableTemplate,
    ) -> Result<Vec<HirStmt>, String> {
        if let GenericCallableTemplate::Named { symbol } = &target {
            if self.signatures.get(symbol).is_some_and(|signature| signature.generic_this_pattern.is_some()) {
                return Err(format!(
                    "generic function .bind() cannot specialize explicit generic this for `{symbol}`"
                ));
            }
        }
        let mut statements = Vec::new();
        let (this_value, leading) = match call.args.split_first() {
            Some((this_arg, leading)) => {
                if this_arg.spread.is_some() {
                    return Err("function .bind() cannot spread its thisArg".into());
                }
                (self.lower_expr(&this_arg.expr)?, leading)
            }
            None => (HirExpr::Lit(HirLit::Undefined), &[][..]),
        };
        let this_type = self.infer_expr_type(&this_value)?;
        let this_name = format!("__thaw_generic_bind_capture_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(this_name.clone(), this_type.clone());
        statements.push(HirStmt::Let(this_name.clone(), this_type.clone(), this_value));
        let this_capture = GenericBindCapture { name: this_name, ty: this_type, source_span: None };

        let (values, bindings) =
            self.lower_native_spread_values_with_snapshots(leading, "function .bind()")?;
        for (capture_name, ty, value) in bindings {
            statements.push(HirStmt::Let(capture_name, ty, value));
        }
        let mut leading_captures = Vec::with_capacity(values.len());
        for (index, value) in values.into_iter().enumerate() {
            let HirExpr::Var(capture_name) = &value else { unreachable!() };
            let ty = self.infer_expr_type(&value)?;
            let source_span = if leading.iter().all(|argument| argument.spread.is_none()) {
                leading.get(index).map(|argument| argument.expr.span())
            } else { None };
            if let Some(argument) = leading.get(index).filter(|_| source_span.is_some()) {
                if let Some(hint) = self.source_value_hint(&argument.expr)
                    .and_then(|hint| self.infer_expr_type(&hint).ok())
                    .filter(|hint| !matches!(hint, HirType::Json | HirType::Dynamic | HirType::JsValue))
                {
                    self.source_scope.insert(capture_name.clone(), hint);
                }
                let static_string = match argument.expr.as_ref() {
                    Expr::Lit(Lit::Str(value)) => Some(value.value.to_string_lossy().into_owned()),
                    Expr::Ident(identifier) => {
                        let source = self.resolve_binding(identifier.sym.as_ref());
                        self.immutable_bindings.contains(&source)
                            .then(|| self.static_string_bindings.get(&source).cloned())
                            .flatten()
                    }
                    _ => None,
                };
                if let Some(value) = static_string {
                    self.static_string_bindings.insert(capture_name.clone(), value);
                }
            }
            leading_captures.push(GenericBindCapture { name: capture_name.clone(), ty, source_span });
        }

        let bound = self.bind_decl_local(name, HirType::Dynamic, true);
        self.deferred_generic_binds.insert(bound.clone(), DeferredGenericBind {
            target,
            this_capture,
            leading_captures,
        });
        Ok(statements)
    }

    fn lower_var_decl_inner(&mut self, var_decl: &VarDecl) -> Result<Vec<HirStmt>, String> {
        let mut statements = Vec::new();
        for decl in &var_decl.decls {
            if let Pat::Ident(binding) = &decl.name {
                let name = binding.id.sym.to_string();
                // A declaration with no initializer at all (`let record;`,
                // or `let record: SomeType;`) used to be rejected
                // outright, regardless of its declared type -- a real,
                // common pattern (real example: csv-parse's own streaming
                // API, `let record; while ((record = parser.read()) !==
                // null) { ... }`, mirroring Node's own `Readable` docs).
                // Supported here for exactly the cases where a `let`'s
                // real-JS initial value (`undefined`) has a well-defined
                // representation in the declared native type: no
                // annotation at all (defaults to `Json`, matching how a
                // bare `any`/`unknown` annotation already lowers -- an
                // untyped local is exactly that in spirit), or an
                // annotation whose type already has an existing "absent"
                // encoding (`Json`'s NAPI-undefined sentinel, `JsValue`,
                // `Optional`/`Nullable`/`Nullish`, a union with an
                // `undefined` member -- all via the same `coerce_to_
                // declared` machinery an ordinary `let x: T = someValue`
                // already funnels through, just fed a literal `undefined`
                // instead). A concrete scalar/aggregate annotation with no
                // such encoding (`let x: number;`) still can't produce a
                // real default value and is rejected -- honestly matching
                // real, strict TypeScript's own "variable is used before
                // being assigned" diagnostic for the same shape.
                if decl.init.is_none() {
                    let annotated = binding
                        .type_ann
                        .as_ref()
                        .map(|annotation| {
                            lower_ts_type(
                                &annotation.type_ann,
                                self.interfaces,
                                self.generic_interfaces,
                            )
                        })
                        .transpose()?;
                    let ty = annotated.unwrap_or(HirType::Json);
                    // `let x: number;` with a later assignment: start from the
                    // type's zero value (ponytail: a read before the first
                    // assignment sees 0/""/false, not `undefined`).
                    let zero = if self.assigned_later.contains(&name) {
                        match ty {
                            HirType::F64 => Some(HirLit::F64(0.0)),
                            HirType::I64 => Some(HirLit::I64(0)),
                            HirType::Bool => Some(HirLit::Bool(false)),
                            HirType::Str => Some(HirLit::Str(String::new())),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let value = match zero {
                        Some(lit) => Ok(HirExpr::Lit(lit)),
                        None => self.coerce_to_declared(&ty, HirExpr::Lit(HirLit::Undefined)),
                    }
                    .map_err(|error| {
                            format!(
                                "`{name}` needs an initializer (its declared type `{ty:?}` has \
                                 no default value: {error})"
                            )
                        })?;
                    let hir_name = self.bind_decl_local(&name, ty.clone(), var_decl.kind == VarDeclKind::Const);
                    statements.push(HirStmt::Let(hir_name, ty, value));
                    continue;
                }
                let init = decl
                    .init
                    .as_deref()
                    .ok_or_else(|| format!("`{name}` needs an initializer"))?;
                if binding.type_ann.is_none() {
                    if let Some((call, target)) = self.deferred_generic_bind_call(init) {
                        if var_decl.kind != VarDeclKind::Const {
                            return Err("generic bound function templates currently require an immutable binding".into());
                        }
                        statements.extend(self.lower_deferred_generic_bind(&name, call, target)?);
                        continue;
                    }
                }
                // `const proxy = new Proxy(target, handler)`, when
                // `target` is a simple identifier already declared
                // `Object`/`Dictionary`-shaped in this function: retain
                // its *current* value as an independent, live QuickJS
                // handle (`retainDynamicJson`, via the existing
                // `coerce_to_declared` path an unrelated earlier fix
                // already built for a `JsValue`-declared target with an
                // `Object`/`Dictionary`-typed source -- see that path's
                // own doc comment) and bind it as a real, function-scoped
                // `let` -- emitted here, *before* `init` itself is
                // lowered below, since only a statement-level `let` (not
                // an expression-level one-shot IIFE binding) survives
                // past this one declaration for later statements to read.
                // `proxy_target_live_handles` records the mapping so (a)
                // the `Expr::New` Proxy lowering just below reuses this
                // exact handle as its constructor argument instead of a
                // fresh snapshot, and (b) every later read of the `target`
                // identifier itself (`Expr::Ident`'s own check of the
                // same table) re-reads this handle instead of `target`'s
                // own native storage -- making a `set` trap's mutation
                // (which only ever reaches this handle's QuickJS object)
                // observable from `target` directly, not just through the
                // proxy. Scoped deliberately narrowly: only a bare
                // identifier target, not an object literal or other
                // expression, and only `Object`/`Dictionary`-shaped (the
                // only two static shapes an existing `JsValue`-target
                // coercion already supports).
                if let Expr::New(new_expr) = init {
                    if let Expr::Ident(class) = new_expr.callee.as_ref() {
                        if class.sym == *"Proxy" {
                            if let Some([target_arg, _]) =
                                new_expr.args.as_deref().and_then(|args| {
                                    <&[_; 2]>::try_from(args).ok()
                                })
                            {
                                if target_arg.spread.is_none() {
                                    if let Expr::Ident(target_ident) = target_arg.expr.as_ref() {
                                        let target_name =
                                            self.resolve_binding(target_ident.sym.as_ref());
                                        if let Some(declared_ty) =
                                            self.scope.get(&target_name).cloned()
                                        {
                                            if matches!(
                                                declared_ty,
                                                HirType::Object(_) | HirType::Dictionary(_)
                                            ) {
                                                let live_value = self.coerce_to_declared(
                                                    &HirType::JsValue,
                                                    HirExpr::Var(target_name.clone()),
                                                )?;
                                                let handle_name = format!(
                                                    "__thaw_proxy_target_live_{}",
                                                    self.next_binding
                                                );
                                                self.next_binding += 1;
                                                self.scope.insert(
                                                    handle_name.clone(),
                                                    HirType::JsValue,
                                                );
                                                statements.push(HirStmt::Let(
                                                    handle_name.clone(),
                                                    HirType::JsValue,
                                                    live_value,
                                                ));
                                                self.proxy_target_live_handles.insert(
                                                    target_name,
                                                    (handle_name, declared_ty),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                let sparse_array = self.expression_may_be_sparse_array(init);
                let sparse_array_function = match init {
                    Expr::Arrow(arrow) => arrow_returns_sparse_array(arrow),
                    Expr::Fn(function) => function_returns_sparse_array(&function.function),
                    Expr::Ident(identifier) => self
                        .sparse_array_functions
                        .contains(&self.resolve_binding(identifier.sym.as_ref())),
                    _ => false,
                };
                if let Expr::Yield(yield_expr) = init {
                    let Some((values, element, input, input_type, returns, _)) =
                        self.generator_yields.clone()
                    else {
                        return Err("`yield` is only valid inside a generator function".into());
                    };
                    let (emission, delegated) = self.lower_generator_yield_emission(
                        yield_expr,
                        &values,
                        &element,
                        &input,
                        &returns,
                    )?;
                    let declared = binding
                        .type_ann
                        .as_ref()
                        .map(|annotation| {
                            lower_ts_type(
                                &annotation.type_ann,
                                self.interfaces,
                                self.generic_interfaces,
                            )
                        })
                        .transpose()?
                        .unwrap_or_else(|| {
                            delegated
                                .as_ref()
                                .map(|(_, ty)| ty.clone())
                                .unwrap_or_else(|| input_type.clone())
                        });
                    let resumed = delegated
                        .map(|(value, _)| value)
                        .unwrap_or_else(|| HirExpr::Var(input));
                    let resumed = self.coerce_to_declared(&declared, resumed)?;
                    let hir_name = self.bind_decl_local(&name, declared.clone(), var_decl.kind == VarDeclKind::Const);
                    statements.extend(emission);
                    statements.push(HirStmt::Let(hir_name, declared, resumed));
                    continue;
                }
                let correlated_alias_source = self.expression_identifier_alias_source(init);
                if let (Expr::Arrow(arrow), Some(annotation)) = (init, binding.type_ann.as_ref()) {
                    if arrow.type_params.is_some() {
                        if let Some((expected, callable_name)) =
                            self.generic_callable_annotation_signature(&annotation.type_ann)?
                        {
                            let actual = self.generic_arrow_signature(arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_decl_local(&name, HirType::Dynamic, var_decl.kind == VarDeclKind::Const);
                            self.generic_arrows.insert(hir_name, arrow.clone());
                            continue;
                        }
                    }
                }
                if let (Expr::Fn(function), Some(annotation)) = (init, binding.type_ann.as_ref()) {
                    if function.function.type_params.is_some() {
                        if let Some((expected, callable_name)) =
                            self.generic_callable_annotation_signature(&annotation.type_ann)?
                        {
                            let arrow = function_expression_as_arrow(function)?;
                            let actual = self.generic_arrow_signature(&arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_decl_local(&name, HirType::Dynamic, var_decl.kind == VarDeclKind::Const);
                            self.generic_arrows.insert(hir_name.clone(), arrow);
                            self.generic_non_arrow_names.insert(hir_name.clone());
                            self.store_generic_function_expression_receiver(&hir_name, function)?;
                            if named_function_is_recursive(function) {
                                self.generic_arrow_self_names.insert(
                                    hir_name,
                                    function.ident.as_ref().unwrap().sym.to_string(),
                                );
                            }
                            continue;
                        }
                    }
                }
                if let (Expr::Ident(identifier), Some(annotation)) =
                    (init, binding.type_ann.as_ref())
                {
                    if let Some((expected, callable_name)) =
                        self.generic_callable_annotation_signature(&annotation.type_ann)?
                    {
                        if let Some(actual) = self.signatures.get(identifier.sym.as_ref()) {
                            if !actual.generic_type_params.is_empty() {
                                self.validate_generic_callable_shape(
                                    &expected,
                                    actual,
                                    &callable_name,
                                )?;
                                let hir_name = self.bind_decl_local(&name, HirType::Dynamic, var_decl.kind == VarDeclKind::Const);
                                self.generic_named_templates
                                    .insert(hir_name, identifier.sym.to_string());
                                continue;
                            }
                        }
                    }
                }
                if let (Expr::Ident(identifier), Some(annotation)) =
                    (init, binding.type_ann.as_ref())
                {
                    if let Some((expected, callable_name)) =
                        self.generic_callable_annotation_signature(&annotation.type_ann)?
                    {
                        let source_name = self.resolve_binding(identifier.sym.as_ref());
                        if let Some(arrow) = self.generic_arrows.get(&source_name).cloned() {
                            let actual = self.generic_arrow_signature(&arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_decl_local(&name, HirType::Dynamic, var_decl.kind == VarDeclKind::Const);
                            self.generic_arrows.insert(hir_name.clone(), arrow);
                            if self.generic_non_arrow_names.contains(&source_name) {
                                self.generic_non_arrow_names.insert(hir_name.clone());
                                if let Some(receiver) = self.generic_non_arrow_receivers.get(&source_name).cloned() {
                                    self.generic_non_arrow_receivers.insert(hir_name.clone(), receiver);
                                }
                                if let Some(template) = self.generic_non_arrow_receiver_templates.get(&source_name).cloned() {
                                    self.generic_non_arrow_receiver_templates.insert(hir_name.clone(), template);
                                }
                            }
                            if let Some(self_name) =
                                self.generic_arrow_self_names.get(&source_name).cloned()
                            {
                                self.generic_arrow_self_names.insert(hir_name, self_name);
                            }
                            continue;
                        }
                        if let Some(target) =
                            self.generic_named_templates.get(&source_name).cloned()
                        {
                            let actual = self
                                .signatures
                                .get(&target)
                                .expect("named generic template target");
                            self.validate_generic_callable_shape(
                                &expected,
                                actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_decl_local(&name, HirType::Dynamic, var_decl.kind == VarDeclKind::Const);
                            self.generic_named_templates.insert(hir_name, target);
                            continue;
                        }
                    }
                }
                if let (Expr::Ident(identifier), None) = (init, binding.type_ann.as_ref()) {
                    let source_name = self.resolve_binding(identifier.sym.as_ref());
                    if let Some(bind) = self.deferred_generic_binds.get(&source_name).cloned() {
                        if var_decl.kind != VarDeclKind::Const {
                            return Err("generic bound function templates currently require an immutable binding".into());
                        }
                        let alias = self.bind_decl_local(&name, HirType::Dynamic, true);
                        self.deferred_generic_binds.insert(alias, bind);
                        continue;
                    }
                    if let Some(arrow) = self.generic_arrows.get(&source_name).cloned() {
                        let hir_name = self.bind_decl_local(&name, HirType::Dynamic, var_decl.kind == VarDeclKind::Const);
                        self.generic_arrows.insert(hir_name.clone(), arrow);
                        if self.generic_non_arrow_names.contains(&source_name) {
                            self.generic_non_arrow_names.insert(hir_name.clone());
                            if let Some(receiver) = self.generic_non_arrow_receivers.get(&source_name).cloned() {
                                self.generic_non_arrow_receivers.insert(hir_name.clone(), receiver);
                            }
                            if let Some(template) = self.generic_non_arrow_receiver_templates.get(&source_name).cloned() {
                                self.generic_non_arrow_receiver_templates.insert(hir_name.clone(), template);
                            }
                        }
                        if let Some(self_name) =
                            self.generic_arrow_self_names.get(&source_name).cloned()
                        {
                            self.generic_arrow_self_names.insert(hir_name, self_name);
                        }
                        continue;
                    }
                    if let Some(GenericCallableTemplate::Named { symbol }) =
                        self.generic_callable_template(identifier.sym.as_ref())
                    {
                        if var_decl.kind != VarDeclKind::Const {
                            return Err("generic function template aliases currently require an immutable binding".into());
                        }
                        let hir_name = self.bind_decl_local(&name, HirType::Dynamic, true);
                        self.generic_named_templates.insert(hir_name, symbol);
                        continue;
                    }
                }
                let annotated = binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        lower_ts_type(
                            &annotation.type_ann,
                            self.interfaces,
                            self.generic_interfaces,
                        )
                    })
                    .transpose()?;
                let source_annotation = annotated.clone();
                let mut source_hint = self.source_value_hint(init).and_then(|hint| self.infer_expr_type(&hint).ok());
                if has_dynamic_storage(binding.id.span) || has_dynamic_storage(init.span()) {
                    if let Some(source) = &source_annotation { self.validate_storage_source(init, source)?; }
                }
                let annotated = annotated.map(|ty| storage_type(binding.id.span, ty));
                let recursive_binding_type = if Self::initializer_callback_references(init, &name)
                    || (annotated.is_some() && Self::initializer_closure_references(init, &name))
                {
                    annotated.clone().or_else(|| {
                        let Expr::Call(call) = init else {
                            return None;
                        };
                        let Callee::Expr(callee) = &call.callee else {
                            return None;
                        };
                        let Expr::Ident(callee) = callee.as_ref() else {
                            return None;
                        };
                        self.signatures
                            .get(&self.resolve_binding(callee.sym.as_ref()))
                            .map(|signature| signature.ret.clone())
                            .or_else(|| {
                                matches!(
                                    callee.sym.as_ref(),
                                    "setTimeout"
                                        | "setInterval"
                                        | "setImmediate"
                                )
                                .then_some(HirType::JsValue)
                            })
                    })
                } else {
                    None
                };
                let recursive_binding = recursive_binding_type
                    .as_ref()
                    .map(|ty| self.bind_decl_local(&name, ty.clone(), var_decl.kind == VarDeclKind::Const));
                if let Expr::Fn(function) = init {
                    if let Some((hir_name, ty, value)) = self.lower_recursive_function_expression(
                        &name,
                        function,
                        annotated.as_ref(),
                        var_decl.kind == VarDeclKind::Const,
                    )? {
                        statements.push(HirStmt::Let(hir_name, ty, value));
                        continue;
                    }
                }
                if annotated.is_none()
                    && (matches!(init, Expr::Arrow(arrow) if arrow.type_params.is_some())
                        || matches!(init, Expr::Fn(function) if function.function.type_params.is_some()))
                {
                    let recursive_self = match init {
                        Expr::Fn(function) if named_function_is_recursive(function) => {
                            function.ident.as_ref().map(|name| name.sym.to_string())
                        }
                        _ => None,
                    };
                    let arrow = match init {
                        Expr::Arrow(arrow) => arrow.clone(),
                        Expr::Fn(function) => function_expression_as_arrow(function)?,
                        _ => unreachable!(),
                    };
                    if arrow.is_async && !arrow.is_generator {
                        return Err(
                            "async generic arrow variables are not supported".into(),
                        );
                    }
                    let hir_name = self.bind_decl_local(&name, HirType::Dynamic, var_decl.kind == VarDeclKind::Const);
                    self.generic_arrows.insert(hir_name.clone(), arrow);
                    if let Expr::Fn(function) = init {
                        self.generic_non_arrow_names.insert(hir_name.clone());
                        self.store_generic_function_expression_receiver(&hir_name, function)?;
                    }
                    if let Some(self_name) = recursive_self {
                        self.generic_arrow_self_names.insert(hir_name, self_name);
                    }
                    continue;
                }
                let native_method_value = self.native_instance_method_value(init).or_else(|| {
                    let Expr::Ident(identifier) = init else {
                        return None;
                    };
                    self.native_method_values
                        .get(&self.resolve_binding(identifier.sym.as_ref()))
                        .cloned()
                });
                let propagated_discriminants = self.expression_union_discriminants(init);
                let propagated_array_discriminants =
                    self.expression_array_element_discriminants(init);
                let propagated_nested_array_discriminants =
                    self.expression_nested_array_discriminants(init);
                let propagated_object_array_property_discriminants =
                    self.expression_object_array_property_discriminants(init);
                let propagated_object_function_property_discriminants =
                    self.expression_object_function_property_discriminants(init);
                let propagated_function_discriminants =
                    self.expression_function_discriminants(init);
                let propagated_function_array_discriminants =
                    self.expression_function_array_discriminants(init);
                let propagated_function_nested_array_discriminants =
                    self.expression_function_nested_array_discriminants(init);
                let propagated_function_object_array_property_discriminants =
                    self.expression_function_object_array_property_discriminants(init);
                let propagated_function_object_function_property_discriminants =
                    self.expression_function_object_function_property_discriminants(init);
                let dynamic_call = matches!(
                    init,
                    Expr::Call(call)
                        if matches!(
                            &call.callee,
                            Callee::Expr(callee)
                                if match callee.as_ref() {
                                    Expr::Ident(identifier) => self.scope
                                        .get(&self.resolve_binding(identifier.sym.as_ref()))
                                        == Some(&HirType::JsValue),
                                    Expr::Member(member) => self
                                        .infer_member_receiver_type(&member.obj)
                                        == Some(HirType::JsValue),
                                    _ => false,
                                }
                        )
                );
                // `const r = await fetch(u)` whose `r` is used as a receiver (`r.status`) is a Response.
                let awaited_fetch = matches!(init, Expr::Await(a)
                    if matches!(a.arg.as_ref(), Expr::Call(c)
                        if matches!(&c.callee, Callee::Expr(e)
                            if matches!(e.as_ref(), Expr::Ident(i) if &*i.sym == "fetch"))));
                let inferred_expected = if annotated.is_none()
                    && awaited_fetch
                    && self.member_receiver_bindings.contains(&name)
                {
                    Some(HirType::JsValue)
                } else if annotated.is_none() && dynamic_call {
                    if self.awaited_bindings.contains(&name) {
                        Some(HirType::Dynamic)
                    } else if self.member_receiver_bindings.contains(&name) {
                        Some(HirType::JsValue)
                    } else {
                        None
                    }
                } else {
                    None
                };
                let static_string = (var_decl.kind == VarDeclKind::Const)
                    .then(|| self.static_property_name(init))
                    .flatten();
                let value = match (init, annotated.as_ref()) {
                    (Expr::Arrow(arrow), Some(HirType::Function(params, ret))) => {
                        self.stored_arrow_initializer = true;
                        let lowered = self.lower_contextual_arrow(arrow, params, Some(ret));
                        self.stored_arrow_initializer = false;
                        lowered?
                    }
                    (Expr::Arrow(arrow), Some(HirType::CallableFunction(params, _, rest, ret))) => {
                        let mut abi_params = params.clone();
                        if let Some(rest) = rest {
                            abi_params.push(HirType::Array(rest.clone()));
                        }
                        self.stored_arrow_initializer = true;
                        let lowered = self.lower_contextual_arrow(arrow, &abi_params, Some(ret));
                        self.stored_arrow_initializer = false;
                        lowered?
                    }
                    (Expr::Fn(function), Some(HirType::Function(params, ret))) => {
                        self.stored_arrow_initializer = true;
                        let lowered = self.lower_function_expression(function, Some((params, Some(ret))));
                        self.stored_arrow_initializer = false;
                        lowered?
                    }
                    (Expr::Fn(function), Some(HirType::CallableFunction(params, _, rest, ret))) => {
                        let mut abi_params = params.clone();
                        if let Some(rest) = rest {
                            abi_params.push(HirType::Array(rest.clone()));
                        }
                        self.stored_arrow_initializer = true;
                        let lowered = self.lower_function_expression(function, Some((&abi_params, Some(ret))));
                        self.stored_arrow_initializer = false;
                        lowered?
                    }
                    (
                        Expr::Ident(identifier),
                        Some(HirType::Function(_, _) | HirType::CallableFunction(..)),
                    ) => {
                        let source = self.resolve_binding(identifier.sym.as_ref());
                        if matches!(
                            self.generic_callable_template(&source),
                            Some(GenericCallableTemplate::Named { .. })
                        ) {
                            let (params, ret) = match annotated.as_ref().unwrap() {
                                HirType::Function(params, ret) => (params.clone(), ret.as_ref().clone()),
                                HirType::CallableFunction(params, _, rest, ret) => {
                                    let mut params = params.clone();
                                    if let Some(rest) = rest {
                                        params.push(HirType::Array(rest.clone()));
                                    }
                                    (params, ret.as_ref().clone())
                                }
                                _ => unreachable!("function type matched above"),
                            };
                            self.lower_promise_callback(init, &params, Some(&ret))?
                        } else if self.scope.contains_key(&source) {
                            self.lower_expr(init)?
                        } else {
                            let signature = self.signatures.get(&source).ok_or_else(|| {
                                format!("unknown function value `{}`", identifier.sym)
                            })?;
                            if signature.is_extern
                                && ordinary_quickjs_typed_symbol(&source).is_some()
                                && signature.generic_type_params.is_empty()
                                && is_quickjs_function_source(signature.source_range.0)
                            {
                                self.lower_expr(init)?
                            } else if signature.is_extern
                                || !signature.generic_type_params.is_empty()
                            {
                                return Err(format!(
                                    "function value `{}` needs a monomorphic implementation",
                                    identifier.sym
                                ));
                            } else {
                                let ret = if signature.is_async {
                                    HirType::Promise(Box::new(signature.ret.clone()))
                                } else {
                                    signature.ret.clone()
                                };
                                HirExpr::FunctionRef(source, signature.params.clone(), ret)
                            }
                        }
                    }
                    _ => {
                        self.stored_arrow_initializer = matches!(init, Expr::Arrow(_));
                        self.expected_source_return_hint =
                            matches!(init, Expr::Call(_) | Expr::Await(_))
                                .then(|| source_annotation.clone())
                                .flatten();
                        let lowered = self.lower_expr_with_expected_type(
                            init,
                            annotated.as_ref().or(inferred_expected.as_ref()),
                        );
                        self.expected_source_return_hint = None;
                        self.stored_arrow_initializer = false;
                        lowered?
                    }
                };

                let actual_type = self.infer_expr_type_inner(&value).map_err(|error| {
                    format!(
                        "cannot infer the type of `{name}`: {error} \
                         (add an explicit type annotation)"
                    )
                })?;
                if source_hint.is_none() {
                    let (source_start, is_async) = match init {
                        Expr::Arrow(arrow) => (Some(arrow.span.lo.0), arrow.is_async),
                        Expr::Fn(function) => {
                            (Some(function.function.span.lo.0), function.function.is_async)
                        }
                        _ => (None, false),
                    };
                    if let Some(source_start) = source_start {
                        source_hint = storage_source_return_type(source_start).map(|mut source| {
                            if is_async && !matches!(source, HirType::Promise(_)) {
                                source = HirType::Promise(Box::new(source));
                            }
                            match &actual_type {
                                HirType::Function(params, _) => {
                                    HirType::Function(
                                        storage_source_callable_parameters(source_start, params),
                                        Box::new(source),
                                    )
                                }
                                HirType::CallableFunction(params, optional, rest, _) => {
                                    let mut source_slots = params.clone();
                                    if let Some(rest) = rest {
                                        source_slots.push(HirType::Array(rest.clone()));
                                    }
                                    let mut source_slots =
                                        storage_source_callable_parameters(source_start, &source_slots);
                                    let source_rest = if rest.is_some() {
                                        match source_slots.pop() {
                                            Some(HirType::Array(element)) => Some(element),
                                            _ => rest.clone(),
                                        }
                                    } else {
                                        None
                                    };
                                    HirType::CallableFunction(
                                        source_slots,
                                        optional.clone(),
                                        source_rest,
                                        Box::new(source),
                                    )
                                }
                                _ => actual_type.clone(),
                            }
                        });
                    }
                }
                let ty = match annotated {
                    Some(ty) => ty,
                    // A container that receives a sink read keeps `undefined` elements.
                    None if is_sink_slot(binding.id.span) => widen_sink_type(actual_type.clone()),
                    None => actual_type.clone(),
                };
                // A fresh empty `new Map()`/`new Set()` simply takes the widened type.
                let value = match value {
                    HirExpr::TypedClosure(inner_type, allocation)
                        if is_sink_slot(binding.id.span)
                            && matches!(inner_type, HirType::Map(..) | HirType::Set(_))
                            && matches!(allocation.as_ref(), HirExpr::Call(callee, args)
                                if args.is_empty() && matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_map_new")) =>
                    {
                        HirExpr::TypedClosure(widen_sink_type(inner_type), allocation)
                    }
                    other => other,
                };
                // `const a: number[] = stack` / `const m: Map<K, V> | undefined = registry`: the
                // object stays the subclass instance (it prints, tests `instanceof` and dispatches
                // as one), so an immutable binding keeps the instance type for the base member.
                let ty = match native_base_type(&actual_type) {
                    Some(base) if var_decl.kind == VarDeclKind::Const && binding.type_ann.is_some() => {
                        let retain = |member: &HirType| if member == base { actual_type.clone() } else { member.clone() };
                        match &ty {
                            HirType::Optional(inner) => HirType::Optional(Box::new(retain(inner))),
                            HirType::Nullable(inner) => HirType::Nullable(Box::new(retain(inner))),
                            HirType::Nullish(inner) => HirType::Nullish(Box::new(retain(inner))),
                            HirType::Union(members) => HirType::Union(members.iter().map(retain).collect()),
                            other => retain(other),
                        }
                    }
                    _ => ty,
                };
                // `const err: any = <caught error>`: an `any`/`Dynamic` or
                // bare-`Json` annotation on a *caught-exception string*
                // must stay `Str`. Coercing it to `Json` (the default an
                // `any` annotation otherwise gets for a string) would turn
                // `err.name`/`err.message` into plain JSON key lookups on a
                // JSON string -- both `undefined` -- and lose the
                // `__thaw_error_name`/`__thaw_error_message` routing a
                // `Str` value gets (real trigger: `catch (e) { const err:
                // any = e; err.name }` vs. the working `String(e)`).
                // A reassignable `let x: any = "s"` must stay `any`: only an immutable
                // alias or a direct copy of a caught error keeps the `Str` storage.
                let keeps_str = var_decl.kind == VarDeclKind::Const
                    || matches!(&value, HirExpr::Var(n)
                        if self.catch_bindings.contains(n) || self.promise_catch_bindings.contains(n));
                let ty = if actual_type == HirType::Str
                    && keeps_str
                    && matches!(ty, HirType::Dynamic | HirType::Json)
                {
                    HirType::Str
                } else {
                    ty
                };
                // An `any`/`Json`-annotated local whose initializer is a
                // live `JsValue` (a class token/prototype, an npm object,
                // ...) keeps the live handle as its storage type instead of
                // the `Json` handle-placeholder snapshot -- otherwise a
                // dynamically-obtained method (`const p: any =
                // A.prototype; p.greet`) reads back `undefined`, since the
                // placeholder object only carries the handle id.
                let ty = if matches!(ty, HirType::Dynamic | HirType::Json)
                    && actual_type == HirType::JsValue
                {
                    HirType::JsValue
                } else {
                    ty
                };
                let private_receiver_alias = if binding.type_ann.is_none() {
                    match &value {
                        HirExpr::NonArrowThis {
                            payload,
                            declared,
                            access: crate::NonArrowThisAccess::Value,
                        } => Some((payload.as_ref().clone(), declared.clone())),
                        _ => None,
                    }
                } else {
                    None
                };
                let sparse_index = self.is_primitive_array_index(init, &ty);
                let mut value = if let Some((payload, _)) = &private_receiver_alias {
                    payload.clone()
                } else if binding.type_ann.is_some() && sparse_index {
                    self.coerce_primitive_array_argument(value, &ty)?
                } else {
                    self.class_method_view_made = false;
                    self.coerce_to_declared_aliasing(&ty, value)?
                };
                let method_view_made = std::mem::take(&mut self.class_method_view_made);

                let storage_type = if let Some((_, receiver)) = &private_receiver_alias {
                    HirType::NonArrowReceiver(Box::new(receiver.clone()))
                } else if class_name_from_type(&actual_type).is_some()
                    && matches!(&ty, HirType::Object(fields) if fields.iter().any(|(_, field)| *field == HirType::Dynamic))
                {
                    actual_type.clone()
                } else if ty == HirType::Dynamic && actual_type == HirType::Str && keeps_str {
                    // `const err: any = <caught error>` -- an `any`-annotated
                    // copy of a caught-exception string. Storing it as
                    // `Dynamic` would drop the `.name`/`.message`/`.code`
                    // property routing a `Str` value gets (real trigger:
                    // `catch (e) { const err: any = e; err.name }` came back
                    // `undefined` while `String(e)` still worked). Keep the
                    // concrete `Str` storage type; an `any` annotation never
                    // requires a value to actually be widened.
                    HirType::Str
                } else {
                    ty.clone()
                };
                let hir_name = if let Some(hir_name) = recursive_binding {
                    self.scope.insert(hir_name.clone(), storage_type.clone());
                    value = HirExpr::RecursiveClosure(
                        hir_name.clone(),
                        storage_type.clone(),
                        Box::new(value),
                    );
                    hir_name
                } else {
                    self.bind_decl_local(&name, storage_type.clone(), var_decl.kind == VarDeclKind::Const)
                };
                if let Some(source) = source_annotation.or(source_hint) { self.source_scope.insert(hir_name.clone(), source); }
                if let Some(value) = static_string {
                    self.static_string_bindings.insert(hir_name.clone(), value);
                }
                // `any` holds a Json snapshot, not the native object: an alias
                // would dispatch methods on a pointer that is not a class instance.
                // A `Map`/`Set` subclass bound to its collection type was projected to the
                // wrapped collection: the binding holds the native collection itself.
                if !method_view_made && class_name_from_type(&actual_type).is_some() && actual_type != storage_type
                    && native_base_type(&actual_type) != Some(&storage_type)
                    && !matches!(storage_type, HirType::Dynamic | HirType::Json | HirType::JsValue) {
                    self.native_class_aliases
                        .insert(hir_name.clone(), actual_type);
                }
                if let Some(source) = correlated_alias_source {
                    self.propagate_destructured_union_alias(&source, &hir_name);
                }
                if let Some(annotation) = &binding.type_ann {
                    self.register_object_union_discriminants(&hir_name, &annotation.type_ann);
                    let element_discriminants = array_element_union_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !element_discriminants.is_empty() {
                        self.array_element_discriminants
                            .insert(hir_name.clone(), element_discriminants);
                    }
                    let nested_discriminants = nested_array_union_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !nested_discriminants.is_empty() {
                        self.nested_array_discriminants
                            .insert(hir_name.clone(), nested_discriminants);
                    }
                    let property_discriminants = object_array_property_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !property_discriminants.is_empty() {
                        self.object_array_property_discriminants
                            .insert(hir_name.clone(), property_discriminants);
                    }
                    let function_property_discriminants = object_function_property_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !function_property_discriminants.is_empty() {
                        self.object_function_property_discriminants
                            .insert(hir_name.clone(), function_property_discriminants);
                    }
                    let return_discriminants = function_return_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !return_discriminants.is_empty() {
                        self.function_value_discriminants
                            .insert(hir_name.clone(), return_discriminants);
                    }
                    let return_array_discriminants = function_return_array_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !return_array_discriminants.is_empty() {
                        self.function_value_array_discriminants
                            .insert(hir_name.clone(), return_array_discriminants);
                    }
                    let return_nested_discriminants = function_return_nested_array_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !return_nested_discriminants.is_empty() {
                        self.function_value_nested_array_discriminants
                            .insert(hir_name.clone(), return_nested_discriminants);
                    }
                    let return_property_discriminants =
                        function_return_object_array_property_discriminants(
                            &annotation.type_ann,
                            self.generic_interfaces,
                        );
                    if !return_property_discriminants.is_empty() {
                        self.function_value_object_array_property_discriminants
                            .insert(hir_name.clone(), return_property_discriminants);
                    }
                    let return_function_property_discriminants =
                        function_return_object_function_property_discriminants(
                            &annotation.type_ann,
                            self.generic_interfaces,
                        );
                    if !return_function_property_discriminants.is_empty() {
                        self.function_value_object_function_property_discriminants
                            .insert(hir_name.clone(), return_function_property_discriminants);
                    }
                } else if let Some(discriminants) = propagated_discriminants {
                    self.union_discriminants
                        .insert(hir_name.clone(), discriminants);
                }
                if binding.type_ann.is_none() {
                    if let Some(discriminants) = propagated_array_discriminants {
                        self.array_element_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_nested_array_discriminants {
                        self.nested_array_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_object_array_property_discriminants {
                        self.object_array_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_object_function_property_discriminants {
                        self.object_function_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                }
                if binding.type_ann.is_none() {
                    if let Some(discriminants) = propagated_function_discriminants {
                        self.function_value_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_function_array_discriminants {
                        self.function_value_array_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_function_nested_array_discriminants {
                        self.function_value_nested_array_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) =
                        propagated_function_object_array_property_discriminants
                    {
                        self.function_value_object_array_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) =
                        propagated_function_object_function_property_discriminants
                    {
                        self.function_value_object_function_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                }
                if let Some(method) = native_method_value {
                    self.native_method_values.insert(hir_name.clone(), method);
                }
                if sparse_array {
                    self.sparse_arrays.insert(hir_name.clone());
                }
                if sparse_array_function {
                    self.sparse_array_functions.insert(hir_name.clone());
                }
                statements.push(HirStmt::Let(hir_name, storage_type, value));
                continue;
            }

            let init = decl
                .init
                .as_deref()
                .ok_or("destructuring declarations need an initializer")?;
            let propagated_discriminants = self.expression_union_discriminants(init);
            let propagated_property_discriminants =
                self.expression_object_array_property_discriminants(init);
            let propagated_function_property_discriminants =
                self.expression_object_function_property_discriminants(init);
            let mut value = self.lower_expr(init)?;
            let annotation = match &decl.name {
                Pat::Array(pattern) => pattern.type_ann.as_ref(),
                Pat::Object(pattern) => pattern.type_ann.as_ref(),
                _ => None,
            };
            let ty = if let Some(annotation) = annotation {
                let ty = lower_ts_type(
                    &annotation.type_ann,
                    self.interfaces,
                    self.generic_interfaces,
                )?;
                value = self.coerce_to_declared(&ty, value)?;
                ty
            } else if matches!(&decl.name, Pat::Array(_)) {
                if let HirExpr::ArrayLit(elements) = &value {
                    HirType::Tuple(
                        elements
                            .iter()
                            .map(|element| self.infer_expr_type_inner(element))
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                } else {
                    self.infer_expr_type(&value)?
                }
            } else {
                self.infer_expr_type(&value)?
            };
            // `const [first] = map` / `= set`: destructure the snapshot of its entries / values.
            let (value, ty) = if matches!(&decl.name, Pat::Array(_))
                && matches!(&ty, HirType::Map(..) | HirType::Set(_))
            {
                let value = self.native_collection_entries(value)?;
                let ty = self.infer_expr_type(&value)?;
                (value, ty)
            } else {
                (value, ty)
            };
            // `const [a, b] = buffer`: the elements are read from a widened copy.
            let value = if matches!(&decl.name, Pat::Array(_)) {
                self.bytes_to_f64_array(value)?
            } else {
                value
            };
            let destructurable_union = matches!(
                &ty,
                HirType::Union(elements)
                    if (matches!(&decl.name, Pat::Object(_))
                        && elements.iter().all(|element| matches!(element, HirType::Object(_))))
                        || (matches!(&decl.name, Pat::Array(_))
                            && elements.iter().all(|element| matches!(element, HirType::Tuple(_) | HirType::Array(_))))
            );
            let destructurable_dictionary =
                matches!((&decl.name, &ty), (Pat::Object(_), HirType::Dictionary(_)));
            // `const {a} = anyValue;` for a bare dynamic `Json` object --
            // same story as `destructurable_array` below, and the same
            // fix shape: `lower_binding_pattern` (reached just below)
            // already supports `Pat::Object` against `Json` by reusing
            // `Dictionary`'s own machinery (every property reads back as
            // `Json`, exactly `Dictionary(Json)`'s own contract), this
            // gate just never let a `let`/`const` declaration reach it.
            let destructurable_json_object =
                matches!((&decl.name, &ty), (Pat::Object(_), HirType::Json));
            // `const [a, b] = arr;` for a homogeneous `T[]` (dynamic
            // length, unlike a fixed-arity `Tuple`) or a bare dynamic
            // `Json` array -- `lower_binding_pattern` (reached just below)
            // already supports both for `Pat::Array` (originally added
            // for a different caller, e.g. a `for`-`of` binding), this
            // check just never let a `let`/`const` declaration reach it.
            let destructurable_array = matches!(&decl.name, Pat::Array(_))
                && matches!(&ty, HirType::Array(_) | HirType::Json);
            let destructurable_string = ty == HirType::Str
                && matches!(&decl.name, Pat::Array(_) | Pat::Object(_));
            if !matches!(ty, HirType::Object(_) | HirType::Tuple(_))
                && !destructurable_string
                && !destructurable_union
                && !destructurable_dictionary
                && !destructurable_json_object
                && !destructurable_array
            {
                return Err(format!(
                    "destructuring requires a fixed-shape object or tuple, got {ty:?}"
                ));
            }
            let temporary = format!("__thaw_destructure_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(temporary.clone(), ty.clone());
            let declared_discriminants = annotation.and_then(|annotation| {
                let discriminants =
                    object_union_discriminants(&annotation.type_ann, self.generic_interfaces);
                (!discriminants.is_empty()).then_some(discriminants)
            });
            if let Some(discriminants) = declared_discriminants.or(propagated_discriminants) {
                self.union_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            let declared_property_discriminants = annotation.and_then(|annotation| {
                let discriminants = object_array_property_discriminants(
                    &annotation.type_ann,
                    self.generic_interfaces,
                );
                (!discriminants.is_empty()).then_some(discriminants)
            });
            if let Some(discriminants) =
                declared_property_discriminants.or(propagated_property_discriminants)
            {
                self.object_array_property_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            let declared_function_property_discriminants = annotation.and_then(|annotation| {
                let discriminants = object_function_property_discriminants(
                    &annotation.type_ann,
                    self.generic_interfaces,
                );
                (!discriminants.is_empty()).then_some(discriminants)
            });
            if let Some(discriminants) = declared_function_property_discriminants
                .or(propagated_function_property_discriminants)
            {
                self.object_function_property_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            statements.push(HirStmt::Let(temporary.clone(), ty.clone(), value));
            self.lower_binding_pattern_impl(
                &decl.name,
                HirExpr::Var(temporary),
                &ty,
                &mut statements,
                var_decl.kind == VarDeclKind::Const,
            )?;
        }
        Ok(statements)
    }
}
