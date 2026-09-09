impl<'a> FnLowerer<'a> {
    fn collect_pattern_bindings(pattern: &Pat, names: &mut Vec<String>) {
        match pattern {
            Pat::Ident(binding) => names.push(binding.id.sym.to_string()),
            Pat::Array(array) => {
                for element in array.elems.iter().flatten() {
                    Self::collect_pattern_bindings(element, names);
                }
            }
            Pat::Object(object) => {
                for property in &object.props {
                    match property {
                        ObjectPatProp::KeyValue(property) => {
                            Self::collect_pattern_bindings(&property.value, names)
                        }
                        ObjectPatProp::Assign(property) => {
                            names.push(property.key.id.sym.to_string())
                        }
                        ObjectPatProp::Rest(rest) => {
                            Self::collect_pattern_bindings(&rest.arg, names)
                        }
                    }
                }
            }
            Pat::Assign(assignment) => {
                Self::collect_pattern_bindings(&assignment.left, names)
            }
            Pat::Rest(rest) => Self::collect_pattern_bindings(&rest.arg, names),
            Pat::Expr(_) | Pat::Invalid(_) => {}
        }
    }

    fn loop_closure_references(stmt: &Stmt, name: &str) -> bool {
        struct Finder<'a> {
            name: &'a str,
            closure_depth: usize,
            found: bool,
        }

        impl Visit for Finder<'_> {
            fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
                self.closure_depth += 1;
                arrow.visit_children_with(self);
                self.closure_depth -= 1;
            }

            fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
                self.closure_depth += 1;
                function.visit_children_with(self);
                self.closure_depth -= 1;
            }

            fn visit_ident(&mut self, identifier: &swc_ecma_ast::Ident) {
                self.found |= self.closure_depth > 0 && identifier.sym == self.name;
            }
        }

        let mut finder = Finder {
            name,
            closure_depth: 0,
            found: false,
        };
        stmt.visit_with(&mut finder);
        finder.found
    }

    /// Lowers a control-flow condition (`if`/`while`/`do`/`for`) with the
    /// same truthiness coercion JS itself applies there -- any value is
    /// a valid condition, not just a literal `boolean` (`truthiness_expr`
    /// already backs `!x`, `Boolean(x)`, and `console.assert(x)`; these
    /// four control-flow conditions just never routed through it, always
    /// requiring an exact `Bool` instead). Real example: `if (result.
    /// success)` against a dynamic-call result's own `Json`-typed
    /// `.success` field (`schema.safeParse(...).success`, real zod) --
    /// used to fail outright ("if condition has type Json, expected
    /// Bool") even though the exact same value printed or compared fine.
    fn lower_condition_expr(&mut self, expr: &Expr) -> Result<HirExpr, String> {
        let cond = self.lower_expr(expr)?;
        let ty = self.infer_expr_type(&cond)?;
        self.truthiness_expr(cond, &ty)
    }

    fn lower_stmt_seq(&mut self, stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
        match stmt {
            // Empty statements have no runtime effect. `debugger` only has an
            // observable effect when a JavaScript debugger is attached; a
            // native Thaw executable therefore treats it as a no-op.
            Stmt::Empty(_) | Stmt::Debugger(_) => Ok(Vec::new()),
            Stmt::Return(ret) => {
                if let Some((values, _)) = self.generator_yields.clone() {
                    let mut statements = Vec::new();
                    if let Some(value) = &ret.arg {
                        statements.push(HirStmt::Expr(self.lower_expr(value)?));
                    }
                    statements.push(HirStmt::Return(Some(HirExpr::Var(values))));
                    return Ok(statements);
                }
                if let Some(arg) = &ret.arg {
                    if self.expression_never_returns(arg) {
                        let call = self.lower_expr(arg)?;
                        return Ok(vec![
                            HirStmt::Expr(call),
                            HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                "a function declared as `never` returned".into(),
                            ))),
                        ]);
                    }
                }
                let value = match &ret.arg {
                    Some(arg) => {
                        // Hint the declared return type through, mirroring
                        // `lower_expr_with_expected_type`'s other sink
                        // points (a `let`/`const` annotation, `await`) --
                        // without it, `return c.text(...)` inside an arrow
                        // typed `(c: JsValue): JsValue => ...` (real hono
                        // handler) lowered its dynamic method call with no
                        // hint at all, defaulting to the untyped/JSON
                        // dispatch and silently snapshotting the live
                        // `Response` into JSON instead of returning it live.
                        let ret_type = self.ret_type.clone();
                        let value = self.lower_expr_with_expected_type(arg, Some(&ret_type))?;
                        if self.ret_type == HirType::Void {
                            if self.infer_expr_type(&value)? == HirType::Void
                                && contains_await(&value)
                            {
                                Some(value)
                            } else {
                                return Err("a void function cannot return a value".into());
                            }
                        } else {
                            Some(self.coerce_to_declared(&self.ret_type.clone(), value)?)
                        }
                    }
                    None => {
                        if !matches!(
                            self.ret_type,
                            HirType::Void | HirType::Dynamic | HirType::JsValue
                        ) {
                            return Err(format!(
                                "bare `return` is not valid for return type {:?}",
                                self.ret_type
                            ));
                        }
                        None
                    }
                };
                Ok(vec![HirStmt::Return(value)])
            }
            Stmt::Expr(expr_stmt) => {
                if let Expr::Yield(yield_expr) = expr_stmt.expr.as_ref() {
                    let Some((values, element)) = self.generator_yields.clone() else {
                        return Err("`yield` is only valid inside a generator function".into());
                    };
                    let value = yield_expr
                        .arg
                        .as_ref()
                        .ok_or("generator `yield` requires a value")?;
                    if yield_expr.delegate {
                        let array_type = HirType::Array(Box::new(element.clone()));
                        let value = self.lower_expr_with_expected_type(value, Some(&array_type))?;
                        let value = self.coerce_to_declared(&array_type, value)?;
                        return Ok(vec![HirStmt::Expr(HirExpr::Assign(
                            values.clone(),
                            Box::new(HirExpr::ArrayConcat(
                                vec![HirExpr::Var(values), value],
                                element,
                            )),
                        ))]);
                    }
                    let value = self.lower_expr_with_expected_type(value, Some(&element))?;
                    let value = self.coerce_to_declared(&element, value)?;
                    return Ok(vec![HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_push".into())),
                        vec![HirExpr::Var(values), value],
                    ))]);
                }
                let discarded_dynamic_call = |expr: &Expr| {
                    matches!(
                        expr,
                        Expr::Call(call)
                            if matches!(
                                &call.callee,
                                Callee::Expr(callee)
                                    if matches!(
                                        callee.as_ref(),
                                        Expr::Member(member)
                                            if self.infer_member_receiver_type(&member.obj)
                                                == Some(HirType::JsValue)
                                    )
                            )
                    )
                };
                let discarded_dynamic_result = discarded_dynamic_call(&expr_stmt.expr);
                let discarded_dynamic_await = matches!(
                        expr_stmt.expr.as_ref(),
                        Expr::Await(awaited) if discarded_dynamic_call(&awaited.arg)
                    );
                let value = if discarded_dynamic_result {
                    self.lower_expr_with_expected_type(&expr_stmt.expr, Some(&HirType::Dynamic))?
                } else if discarded_dynamic_await {
                    self.lower_expr_with_expected_type(&expr_stmt.expr, Some(&HirType::JsValue))?
                } else {
                    self.lower_expr(&expr_stmt.expr)?
                };
                let value = if matches!(self.infer_expr_type(&value)?, HirType::Promise(_)) {
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_detach_rejection".into())),
                        vec![value],
                    )
                } else {
                    value
                };
                Ok(vec![HirStmt::Expr(value)])
            }
            Stmt::Block(block) => self.lower_scoped_stmts(&block.stmts),
            Stmt::Decl(Decl::Var(var_decl)) => self.lower_var_decl(var_decl),

            Stmt::If(if_stmt) => {
                let narrowing = self.optional_undefined_narrowing(&if_stmt.test);
                let union_narrowing = self.union_narrowing(&if_stmt.test);
                let json_narrowing = self.json_typeof_narrowing(&if_stmt.test);
                let cond = self.lower_condition_expr(&if_stmt.test)?;
                let then_narrowing = narrowing
                    .as_ref()
                    .filter(|(_, _, present, _)| *present)
                    .map(|(name, payload, _, nullable)| {
                        (name.clone(), payload.clone(), *nullable)
                    });
                let else_narrowing = narrowing
                    .as_ref()
                    .filter(|(_, _, present, _)| !*present)
                    .map(|(name, payload, _, nullable)| {
                        (name.clone(), payload.clone(), *nullable)
                    });
                let branch_union = |truth: bool| {
                    union_narrowing.as_ref().map(|(targets, equal, complement)| {
                        targets
                            .iter()
                            .map(|target| UnionNarrowingTarget {
                                name: target.name.clone(),
                                matching: if truth == *equal {
                                    target.matching.clone()
                                } else if !complement {
                                    target.allowed.clone()
                                } else {
                                    target
                                        .allowed
                                        .iter()
                                        .filter(|index| !target.matching.contains(index))
                                        .copied()
                                        .collect()
                                },
                                allowed: target.allowed.clone(),
                                elements: target.elements.clone(),
                            })
                            .collect::<Vec<_>>()
                    })
                };
                let then_union = branch_union(true);
                let else_union = branch_union(false);
                let then_branch = self
                    .lower_body_with_union_narrowing(
                        &if_stmt.cons,
                        then_union.as_deref(),
                        then_narrowing.as_ref(),
                        json_narrowing
                            .as_ref()
                            .filter(|(_, _, equal)| *equal)
                            .map(|(name, ty, _)| (name.clone(), ty.clone()))
                            .as_ref(),
                    )?;
                let else_branch = match &if_stmt.alt {
                    Some(alt) => self.lower_body_with_union_narrowing(
                        alt,
                        else_union.as_deref(),
                        else_narrowing.as_ref(),
                        json_narrowing
                            .as_ref()
                            .filter(|(_, _, equal)| !*equal)
                            .map(|(name, ty, _)| (name.clone(), ty.clone()))
                            .as_ref(),
                    )?,
                    None => Vec::new(),
                };
                Ok(vec![HirStmt::If(cond, then_branch, else_branch)])
            }

            Stmt::While(while_stmt) => {
                let cond = self.lower_condition_expr(&while_stmt.test)?;
                let body = self.lower_loop_body(&while_stmt.body)?;
                Ok(vec![HirStmt::While(cond, body)])
            }

            Stmt::DoWhile(do_while) => {
                let cond = self.lower_condition_expr(&do_while.test)?;
                let guard = HirStmt::If(cond, Vec::new(), vec![HirStmt::Break]);
                let mut body = self.lower_loop_body(&do_while.body)?;
                body = inject_do_while_guard_before_continue(body, &guard);
                body.push(guard);
                Ok(vec![HirStmt::While(
                    HirExpr::Lit(HirLit::Bool(true)),
                    body,
                )])
            }

            Stmt::Break(break_stmt) => {
                if let Some(label) = &break_stmt.label {
                    let name = label.sym.as_ref();
                    let (_, target_depth, _) = self
                        .labels
                        .iter()
                        .rev()
                        .find(|(candidate, _, _)| candidate == name)
                        .ok_or_else(|| format!("unknown break label `{name}`"))?;
                    return Ok(vec![HirStmt::BreakDepth(self.loop_depth - target_depth)]);
                }
                Ok(vec![HirStmt::Break])
            }

            Stmt::Continue(continue_stmt) => {
                if let Some(label) = &continue_stmt.label {
                    let name = label.sym.as_ref();
                    let (_, target_depth, continuable) = self
                        .labels
                        .iter()
                        .rev()
                        .find(|(candidate, _, _)| candidate == name)
                        .ok_or_else(|| format!("unknown continue label `{name}`"))?;
                    if !continuable {
                        return Err(format!("continue label `{name}` does not name a loop"));
                    }
                    return Ok(vec![HirStmt::ContinueDepth(
                        self.loop_depth - target_depth,
                    )]);
                }
                Ok(vec![HirStmt::Continue])
            }

            Stmt::Labeled(labeled) => {
                let name = labeled.label.sym.to_string();
                if self.labels.iter().any(|(candidate, _, _)| candidate == &name) {
                    return Err(format!("duplicate active label `{name}`"));
                }
                let is_loop = Self::stmt_is_iteration(&labeled.body);
                let target_depth = self.loop_depth + 1;
                self.labels.push((name, target_depth, is_loop));
                let lowered = if is_loop {
                    self.lower_stmt_seq(&labeled.body)
                } else {
                    self.loop_depth += 1;
                    let body_result = self.lower_body(&labeled.body);
                    self.loop_depth -= 1;
                    body_result.map(|mut body| {
                        body.push(HirStmt::Break);
                        vec![HirStmt::While(HirExpr::Lit(HirLit::Bool(true)), body)]
                    })
                };
                self.labels.pop();
                lowered
            }

            Stmt::For(for_stmt) => {
                let saved = self.bindings.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let mut out = Vec::new();
                    if let Some(init) = &for_stmt.init {
                        match init {
                            VarDeclOrExpr::VarDecl(var_decl) => {
                                out.extend(self.lower_var_decl(var_decl)?)
                            }
                            VarDeclOrExpr::Expr(expr) => {
                                out.push(HirStmt::Expr(self.lower_expr(expr)?))
                            }
                        }
                    }

                    let iteration_sources = match &for_stmt.init {
                        Some(VarDeclOrExpr::VarDecl(declaration))
                            if declaration.kind != VarDeclKind::Var =>
                        {
                            let mut names = Vec::new();
                            for declarator in &declaration.decls {
                                Self::collect_pattern_bindings(&declarator.name, &mut names);
                            }
                            names
                                .into_iter()
                                .filter(|name| Self::loop_closure_references(&for_stmt.body, name))
                                .collect::<Vec<_>>()
                        }
                        _ => Vec::new(),
                    };

                    let cond = match &for_stmt.test {
                        Some(test) => self.lower_condition_expr(test)?,
                        None => HirExpr::Lit(HirLit::Bool(true)),
                    };

                    let loop_bindings = self.bindings.clone();
                    let mut iteration_bindings = Vec::new();
                    for source in iteration_sources {
                        let outer = self.resolve_binding(&source);
                        let ty = self.scope.get(&outer).cloned().ok_or_else(|| {
                            format!("unknown `for` iteration binding `{source}`")
                        })?;
                        let inner = self.bind_local(&source, ty.clone());
                        iteration_bindings.push((outer, inner, ty));
                    }
                    let mut body = self.lower_loop_body(&for_stmt.body)?;
                    self.bindings = loop_bindings;
                    let mut prefix = iteration_bindings
                        .iter()
                        .map(|(outer, inner, ty)| {
                            HirStmt::Let(inner.clone(), ty.clone(), HirExpr::Var(outer.clone()))
                        })
                        .collect::<Vec<_>>();
                    prefix.append(&mut body);
                    body = prefix;

                    let mut advance = iteration_bindings
                        .iter()
                        .map(|(outer, inner, _)| {
                            HirStmt::Expr(HirExpr::Assign(
                                outer.clone(),
                                Box::new(HirExpr::Var(inner.clone())),
                            ))
                        })
                        .collect::<Vec<_>>();
                    if let Some(update) = &for_stmt.update {
                        let update = self.lower_expr(update)?;
                        advance.push(HirStmt::Expr(update));
                    }
                    if !advance.is_empty() {
                        body = inject_for_advance_before_continue(body, &advance);
                        body.extend(advance);
                    }

                    out.push(HirStmt::While(cond, body));
                    Ok(out)
                })();
                self.bindings = saved;
                lowered
            }

            Stmt::ForOf(for_of) => {
                let saved = self.bindings.clone();
                let saved_scope = self.scope.clone();
                let saved_correlations = self.destructured_union_correlations.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let item_discriminants =
                        self.expression_array_element_discriminants(&for_of.right);
                    let mut values = self.lower_expr(&for_of.right)?;
                    let mut values_type = self.infer_expr_type(&values)?;
                    let mut generator_producer = None;
                    if let HirType::Function(params, result) = &values_type {
                        if params == &[HirType::I64]
                            && matches!(result.as_ref(), HirType::Array(_))
                        {
                            let result = result.as_ref().clone();
                            let producer_type = values_type.clone();
                            let producer = format!(
                                "__thaw_generator_producer_{}",
                                self.next_binding
                            );
                            self.next_binding += 1;
                            self.scope.insert(producer.clone(), producer_type.clone());
                            generator_producer = Some((producer, producer_type, values));
                            values_type = result;
                            values = HirExpr::ArrayLit(Vec::new());
                        }
                    }
                    if values_type == HirType::Str {
                        values = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_to_array".into())),
                            vec![values],
                        );
                        values_type = HirType::Array(Box::new(HirType::Str));
                    }
                    // `for...of` a `Map` yields `[key, value]` pairs and a
                    // `Set` yields its elements, matching their default
                    // iterators -- snapshot to an array up front (like
                    // `.entries()`/`.values()` do) and let the rest of
                    // this lowering treat it as an ordinary array loop.
                    if let HirType::Map(key_type, value_type) = &values_type {
                        let pair_type = HirType::Tuple(vec![
                            key_type.as_ref().clone(),
                            value_type.as_ref().clone(),
                        ]);
                        let array_type = HirType::Array(Box::new(pair_type));
                        values = HirExpr::TypedClosure(
                            array_type.clone(),
                            Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_map_snapshot_entries".into())),
                                vec![values],
                            )),
                        );
                        values_type = array_type;
                    } else if let HirType::Set(element_type) = &values_type {
                        let array_type = HirType::Array(element_type.clone());
                        values = HirExpr::TypedClosure(
                            array_type.clone(),
                            Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_map_snapshot_keys".into())),
                                vec![values],
                            )),
                        );
                        values_type = array_type;
                    }
                    let (element, json_array) = match &values_type {
                        HirType::Array(element) => (element.as_ref().clone(), false),
                        HirType::Json if !for_of.is_await => (HirType::Json, true),
                        HirType::Json => {
                            return Err("`for await...of` cannot await dynamic JSON values".into())
                        }
                        _ => return Err("`for...of` currently requires a typed array".into()),
                    };
                    let (item_type, await_item) = if for_of.is_await {
                        match &element {
                            HirType::Promise(resolved) => (resolved.as_ref().clone(), true),
                            synchronous => (synchronous.clone(), false),
                        }
                    } else {
                        (element.clone(), false)
                    };
                    let values_name = format!("__thaw_for_of_values_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(values_name.clone(), values_type.clone());
                    let index_name = format!("__thaw_for_of_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(index_name.clone(), HirType::F64);
                    let item_value = || {
                        let indexed = if generator_producer.is_some() {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_shift".into())),
                                vec![HirExpr::Var(values_name.clone())],
                            )
                        } else if json_array {
                            HirExpr::JsonIndex(
                                Box::new(HirExpr::Var(values_name.clone())),
                                Box::new(HirExpr::Var(index_name.clone())),
                            )
                        } else {
                            HirExpr::TypedIndex(
                                Box::new(HirExpr::Var(values_name.clone())),
                                Box::new(HirExpr::Var(index_name.clone())),
                                element.clone(),
                            )
                        };
                        if await_item {
                            HirExpr::AwaitPromise(Box::new(indexed), item_type.clone())
                        } else {
                            indexed
                        }
                    };
                    let item_stmts = match &for_of.left {
                        ForHead::VarDecl(decl) => {
                            let [declarator] = decl.decls.as_slice() else {
                                return Err("`for...of` requires exactly one loop binding".into());
                            };
                            if declarator.init.is_some() {
                                return Err(
                                    "`for...of` loop bindings cannot have an initializer".into(),
                                );
                            }
                            if let Pat::Ident(binding) = &declarator.name {
                                let item_ty = match &binding.type_ann {
                                    Some(annotation) => {
                                        let declared = lower_ts_type(
                                            &annotation.type_ann,
                                            self.interfaces,
                                            self.generic_interfaces,
                                        )?;
                                        if declared != item_type {
                                            return Err(format!(
                                                "`for...of` binding has type {declared:?}, expected {:?}",
                                                item_type
                                            ));
                                        }
                                        declared
                                    }
                                    None => item_type.clone(),
                                };
                                let source_name = binding.id.sym.to_string();
                                let item_name =
                                    format!("{source_name}__thaw_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(item_name.clone(), item_ty.clone());
                                self.bindings
                                    .entry(source_name)
                                    .or_default()
                                    .push(item_name.clone());
                                let declared_discriminants = binding.type_ann.as_ref().map(
                                    |annotation| {
                                        object_union_discriminants(
                                            &annotation.type_ann,
                                            self.generic_interfaces,
                                        )
                                    },
                                );
                                let discriminants = declared_discriminants
                                    .filter(|metadata| !metadata.is_empty())
                                    .or_else(|| item_discriminants.clone());
                                if let Some(discriminants) = discriminants {
                                    self.union_discriminants
                                        .insert(item_name.clone(), discriminants);
                                }
                                vec![HirStmt::Let(item_name, item_ty, item_value())]
                            } else if matches!(declarator.name, Pat::Object(_) | Pat::Array(_)) {
                                let temporary =
                                    format!("__thaw_for_of_item_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(temporary.clone(), item_type.clone());
                                let annotation = match &declarator.name {
                                    Pat::Object(pattern) => pattern.type_ann.as_ref(),
                                    Pat::Array(pattern) => pattern.type_ann.as_ref(),
                                    _ => None,
                                };
                                let declared_discriminants = annotation.map(|annotation| {
                                    object_union_discriminants(
                                        &annotation.type_ann,
                                        self.generic_interfaces,
                                    )
                                });
                                let discriminants = declared_discriminants
                                    .filter(|metadata| !metadata.is_empty())
                                    .or_else(|| item_discriminants.clone());
                                if let Some(discriminants) = discriminants {
                                    self.union_discriminants
                                        .insert(temporary.clone(), discriminants);
                                }
                                let mut statements = vec![HirStmt::Let(
                                    temporary.clone(),
                                    item_type.clone(),
                                    item_value(),
                                )];
                                self.lower_binding_pattern(
                                    &declarator.name,
                                    HirExpr::Var(temporary),
                                    &item_type,
                                    &mut statements,
                                )?;
                                statements
                            } else {
                                return Err("unsupported `for...of` binding pattern".into());
                            }
                        }
                        ForHead::Pat(pattern) => {
                            if let Pat::Ident(binding) = pattern.as_ref() {
                                let item_name = self.resolve_binding(binding.id.sym.as_ref());
                                let item_ty = self.scope.get(&item_name).cloned().ok_or_else(|| {
                                    format!("unknown `for...of` assignment target `{item_name}`")
                                })?;
                                if item_ty != item_type {
                                    return Err(format!(
                                        "`for...of` assignment target has type {item_ty:?}, expected {:?}",
                                        item_type
                                    ));
                                }
                                vec![HirStmt::Expr(HirExpr::Assign(
                                    item_name,
                                    Box::new(item_value()),
                                ))]
                            } else if matches!(pattern.as_ref(), Pat::Object(_) | Pat::Array(_)) {
                                let temporary =
                                    format!("__thaw_for_of_item_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(temporary.clone(), item_type.clone());
                                if let Some(discriminants) = item_discriminants.clone() {
                                    self.union_discriminants
                                        .insert(temporary.clone(), discriminants);
                                }
                                let mut statements = vec![HirStmt::Let(
                                    temporary.clone(),
                                    item_type.clone(),
                                    item_value(),
                                )];
                                self.lower_assignment_pattern(
                                    pattern,
                                    HirExpr::Var(temporary),
                                    &item_type,
                                    &mut statements,
                                )?;
                                statements
                            } else {
                                return Err("unsupported `for...of` assignment pattern".into());
                            }
                        }
                        ForHead::UsingDecl(_) => {
                            return Err("`using` bindings in `for...of` are not supported".into())
                        }
                    };
                    let mut body = Vec::new();
                    if let Some((producer, _, _)) = &generator_producer {
                        body.extend([
                            HirStmt::Expr(HirExpr::Assign(
                                values_name.clone(),
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var(producer.clone())),
                                    vec![HirExpr::Lit(HirLit::I64(0))],
                                )),
                            )),
                            HirStmt::If(
                                HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(
                                        values_name.clone(),
                                    )))),
                                    Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                                ),
                                vec![HirStmt::Break],
                                Vec::new(),
                            ),
                        ]);
                    }
                    body.extend(item_stmts);
                    body.extend(self.lower_loop_body(&for_of.body)?);
                    if generator_producer.is_none() {
                        let update = HirExpr::Assign(
                            index_name.clone(),
                            Box::new(HirExpr::BinOp(
                                BinOp::Add,
                                Box::new(HirExpr::Var(index_name.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                            )),
                        );
                        body = inject_for_update_before_continue(body, &update);
                        body.push(HirStmt::Expr(update));
                    }
                    let condition = if generator_producer.is_some() {
                        HirExpr::Lit(HirLit::Bool(true))
                    } else {
                        HirExpr::BinOp(
                            BinOp::Lt,
                            Box::new(HirExpr::Var(index_name.clone())),
                            Box::new(if json_array {
                                HirExpr::JsonAsNumber(Box::new(HirExpr::JsonGet(
                                    Box::new(HirExpr::Var(values_name.clone())),
                                    "length".into(),
                                )))
                            } else {
                                HirExpr::ArrayLen(Box::new(HirExpr::Var(values_name.clone())))
                            }),
                        )
                    };
                    let mut statements = vec![
                        HirStmt::Let(values_name, values_type, values),
                        HirStmt::Let(
                            index_name,
                            HirType::F64,
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ),
                        HirStmt::While(condition, body),
                    ];
                    let Some((producer, producer_type, init)) = generator_producer else {
                        return Ok(statements);
                    };
                    statements.push(HirStmt::Return(None));
                    let lambda_body = HirExpr::Block(statements);
                    let mut referenced = BTreeSet::new();
                    collect_referenced_bindings(&lambda_body, &mut referenced);
                    let captures = referenced
                        .into_iter()
                        .filter_map(|name| {
                            saved_scope
                                .get(&name)
                                .cloned()
                                .map(|ty| HirParam { name, ty })
                        })
                        .collect();
                    Ok(vec![HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Lambda(
                            captures,
                            vec![HirParam {
                                name: producer,
                                ty: producer_type,
                            }],
                            HirType::Void,
                            Box::new(lambda_body),
                        )),
                        vec![init],
                    ))])
                })();
                self.bindings = saved;
                self.scope = saved_scope;
                self.destructured_union_correlations = saved_correlations;
                lowered
            }

            Stmt::ForIn(for_in) => {
                let saved = self.bindings.clone();
                let saved_scope = self.scope.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let object = self.lower_expr(&for_in.right)?;
                    let object_type = self.infer_expr_type(&object)?;
                    let keys = match &object_type {
                        HirType::Object(fields) => fields
                            .iter()
                            .map(|(name, _)| HirExpr::Lit(HirLit::Str(name.clone())))
                            .collect::<Vec<_>>(),
                        HirType::Json | HirType::Dictionary(_) => Vec::new(),
                        _ => {
                            return Err(
                                "`for...in` requires an object, dictionary, or JSON value".into(),
                            )
                        }
                    };
                    let object_name = format!("__thaw_for_in_object_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(object_name.clone(), object_type.clone());
                    let keys_name = format!("__thaw_for_in_keys_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(
                        keys_name.clone(),
                        HirType::Array(Box::new(HirType::Str)),
                    );
                    let keys = if matches!(object_type, HirType::Json | HirType::Dictionary(_)) {
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_keys".into())),
                            vec![HirExpr::Var(object_name.clone())],
                        )
                    } else {
                        HirExpr::ArrayLit(keys)
                    };
                    let index_name = format!("__thaw_for_in_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(index_name.clone(), HirType::F64);
                    let key_value = || {
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(keys_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            HirType::Str,
                        )
                    };
                    let binding_stmt = match &for_in.left {
                        ForHead::VarDecl(decl) => {
                            let [declarator] = decl.decls.as_slice() else {
                                return Err("`for...in` requires exactly one loop binding".into());
                            };
                            if declarator.init.is_some() {
                                return Err(
                                    "`for...in` loop bindings cannot have an initializer".into(),
                                );
                            }
                            let Pat::Ident(binding) = &declarator.name else {
                                return Err(
                                    "`for...in` requires an identifier loop binding".into(),
                                );
                            };
                            if let Some(annotation) = &binding.type_ann {
                                let declared = lower_ts_type(
                                    &annotation.type_ann,
                                    self.interfaces,
                                    self.generic_interfaces,
                                )?;
                                if declared != HirType::Str {
                                    return Err(format!(
                                        "`for...in` binding must be Str, got {declared:?}"
                                    ));
                                }
                            }
                            let source_name = binding.id.sym.to_string();
                            let binding_name =
                                format!("{source_name}__thaw_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(binding_name.clone(), HirType::Str);
                            self.bindings
                                .entry(source_name)
                                .or_default()
                                .push(binding_name.clone());
                            HirStmt::Let(binding_name, HirType::Str, key_value())
                        }
                        ForHead::Pat(pattern) => {
                            let Pat::Ident(binding) = pattern.as_ref() else {
                                return Err(
                                    "`for...in` assignment requires an identifier target".into(),
                                );
                            };
                            let binding_name = self.resolve_binding(binding.id.sym.as_ref());
                            let binding_type = self.scope.get(&binding_name).ok_or_else(|| {
                                format!("unknown `for...in` assignment target `{binding_name}`")
                            })?;
                            if binding_type != &HirType::Str {
                                return Err(format!(
                                    "`for...in` assignment target must be Str, got {binding_type:?}"
                                ));
                            }
                            HirStmt::Expr(HirExpr::Assign(
                                binding_name,
                                Box::new(key_value()),
                            ))
                        }
                        ForHead::UsingDecl(_) => {
                            return Err("`using` bindings in `for...in` are not supported".into())
                        }
                    };
                    let update = HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index_name.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    );
                    let mut body = vec![binding_stmt];
                    body.extend(self.lower_loop_body(&for_in.body)?);
                    body = inject_for_update_before_continue(body, &update);
                    body.push(HirStmt::Expr(update));
                    Ok(vec![
                        HirStmt::Let(object_name, object_type, object),
                        HirStmt::Let(
                            keys_name.clone(),
                            HirType::Array(Box::new(HirType::Str)),
                            keys,
                        ),
                        HirStmt::Let(
                            index_name.clone(),
                            HirType::F64,
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ),
                        HirStmt::While(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(HirExpr::Var(index_name)),
                                Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(keys_name)))),
                            ),
                            body,
                        ),
                    ])
                })();
                self.bindings = saved;
                self.scope = saved_scope;
                lowered
            }

            Stmt::Switch(switch_stmt) => {
                let saved = self.bindings.clone();
                let saved_scope = self.scope.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let discriminant = self.lower_expr(&switch_stmt.discriminant)?;
                    let discriminant_type = self.infer_expr_type(&discriminant)?;
                    let value_name = format!("__thaw_switch_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(value_name.clone(), discriminant_type.clone());
                    let selected_name = format!("__thaw_switch_selected_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(selected_name.clone(), HirType::F64);
                    let none = HirExpr::Lit(HirLit::F64(-1.0));
                    let case_count = switch_stmt.cases.len();
                    let default_index = switch_stmt
                        .cases
                        .iter()
                        .position(|case| case.test.is_none())
                        .unwrap_or(case_count);
                    let mut out = vec![
                        HirStmt::Let(value_name.clone(), discriminant_type.clone(), discriminant),
                        HirStmt::Let(selected_name.clone(), HirType::F64, none.clone()),
                    ];

                    for (index, case) in switch_stmt.cases.iter().enumerate() {
                        let Some(test) = &case.test else {
                            continue;
                        };
                        let test = self.lower_expr(test)?;
                        self.expect_type(&discriminant_type, &test, "switch case")?;
                        out.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(selected_name.clone())),
                                Box::new(none.clone()),
                            ),
                            vec![HirStmt::If(
                                HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::Var(value_name.clone())),
                                    Box::new(test),
                                ),
                                vec![HirStmt::Expr(HirExpr::Assign(
                                    selected_name.clone(),
                                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                ))],
                                Vec::new(),
                            )],
                            Vec::new(),
                        ));
                    }
                    out.push(HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::Var(selected_name.clone())),
                            Box::new(none),
                        ),
                        vec![HirStmt::Expr(HirExpr::Assign(
                            selected_name.clone(),
                            Box::new(HirExpr::Lit(HirLit::F64(default_index as f64))),
                        ))],
                        Vec::new(),
                    ));

                    let exit = HirStmt::Expr(HirExpr::Assign(
                        selected_name.clone(),
                        Box::new(HirExpr::Lit(HirLit::F64(case_count as f64))),
                    ));
                    for (index, case) in switch_stmt.cases.iter().enumerate() {
                        let isolated_entry = index == 0
                            || Self::switch_case_prevents_fallthrough(
                                &switch_stmt.cases[index - 1].cons,
                            );
                        let case_union = if isolated_entry {
                            if let Some(test) = &case.test {
                                self.union_narrowing(&Expr::Bin(swc_ecma_ast::BinExpr {
                                    span: switch_stmt.span,
                                    op: BinaryOp::EqEqEq,
                                    left: switch_stmt.discriminant.clone(),
                                    right: test.clone(),
                                }))
                                .map(|(targets, equal, complement)| {
                                    targets
                                        .into_iter()
                                        .map(|target| UnionNarrowingTarget {
                                            matching: if equal {
                                                target.matching.clone()
                                            } else if complement {
                                                target
                                                    .allowed
                                                    .iter()
                                                    .filter(|member| {
                                                        !target.matching.contains(member)
                                                    })
                                                    .copied()
                                                    .collect()
                                            } else {
                                                target.allowed.clone()
                                            },
                                            ..target
                                        })
                                        .collect::<Vec<_>>()
                                })
                            } else {
                                let mut excluded = HashMap::<
                                    Symbol,
                                    (Vec<usize>, Vec<usize>, Vec<HirType>),
                                >::new();
                                for tested_case in &switch_stmt.cases {
                                    let Some(test) = &tested_case.test else {
                                        continue;
                                    };
                                    let Some((targets, equal, _)) = self.union_narrowing(
                                        &Expr::Bin(swc_ecma_ast::BinExpr {
                                            span: switch_stmt.span,
                                            op: BinaryOp::EqEqEq,
                                            left: switch_stmt.discriminant.clone(),
                                            right: test.clone(),
                                        }),
                                    ) else {
                                        continue;
                                    };
                                    if !equal {
                                        continue;
                                    }
                                    for target in targets {
                                        let entry = excluded.entry(target.name).or_insert_with(|| {
                                            (Vec::new(), target.allowed, target.elements)
                                        });
                                        for member in target.matching {
                                            if !entry.0.contains(&member) {
                                                entry.0.push(member);
                                            }
                                        }
                                    }
                                }
                                let targets = excluded
                                    .into_iter()
                                    .filter_map(|(name, (excluded, allowed, elements))| {
                                        let matching = allowed
                                            .iter()
                                            .filter(|member| !excluded.contains(member))
                                            .copied()
                                            .collect::<Vec<_>>();
                                        (!matching.is_empty()).then_some(UnionNarrowingTarget {
                                            name,
                                            matching,
                                            allowed,
                                            elements,
                                        })
                                    })
                                    .collect::<Vec<_>>();
                                (!targets.is_empty()).then_some(targets)
                            }
                        } else {
                            None
                        };
                        let saved_union_narrowings = self.union_narrowings.clone();
                        if let Some(targets) = &case_union {
                            for target in targets {
                                self.union_narrowings.insert(
                                    target.name.clone(),
                                    (target.matching.clone(), target.elements.clone()),
                                );
                            }
                        }
                        let lowered_case = self.lower_stmts(&case.cons);
                        self.union_narrowings = saved_union_narrowings;
                        let mut body = lowered_case?;
                        body = rewrite_switch_case_stmts(
                            body,
                            &selected_name,
                            index,
                            &exit,
                        );
                        body.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(selected_name.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            vec![HirStmt::Expr(HirExpr::Assign(
                                selected_name.clone(),
                                Box::new(HirExpr::Lit(HirLit::F64((index + 1) as f64))),
                            ))],
                            Vec::new(),
                        ));
                        out.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(selected_name.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            body,
                            Vec::new(),
                        ));
                    }
                    if default_index < case_count
                        && switch_stmt.cases.iter().all(|case| {
                            case.cons.last().is_some_and(Self::stmt_definitely_exits)
                        })
                    {
                        out.push(HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                            "unreachable exhaustive switch".into(),
                        ))));
                    }
                    Ok(out)
                })();
                self.bindings = saved;
                self.scope = saved_scope;
                lowered
            }

            Stmt::Throw(throw_stmt) => {
                // The exception channel is a single tagged string end to
                // end (see `new Error(...)`'s lowering and
                // `thaw_runtime::split_error_tag`); a thrown non-string
                // value used to be stored into that `i8*` slot as whatever
                // raw bit pattern it happened to have (an f64's bits
                // reinterpreted as a pointer for `throw 42`, say), which
                // every reader then dereferenced as a C string --
                // undefined behavior, not merely a wrong answer. Coercing
                // every thrown value to its string form here keeps that
                // one representation honest; unsupported types (a thrown
                // `Promise`, function, `Map`/`Set`, etc.) are a compile
                // error instead of memory corruption.
                let value = self.lower_expr(&throw_stmt.arg)?;
                let value_type = self.infer_expr_type(&value)?;
                // A real (Error-family) class instance also gets its raw
                // object pointer stashed in the parallel, opt-in
                // `__thaw_pending_exception_object` channel (see
                // `docs/design/exceptions.md` section 3) *in addition* to
                // the tagged string every other reader already
                // understands, so an explicit `(e as MyError).code` at a
                // `catch` site downstream can recover fields beyond
                // `message`/`name`. A plain string or fieldless Error
                // throw leaves that channel untouched (still null, or
                // stale from a previous throw already cleared at the
                // catching `catch` -- see `compile_try`).
                let error_object_name = object_type_is_error_family(&value_type).then(|| {
                    let name = format!("__thaw_thrown_object_{}", self.next_binding);
                    self.next_binding += 1;
                    name
                });
                if let Some(name) = error_object_name {
                    self.scope.insert(name.clone(), value_type.clone());
                    let object_var = HirExpr::Var(name.clone());
                    let message = self.coerce_primitive_to_string(object_var.clone())?;
                    return Ok(vec![
                        HirStmt::Let(name, value_type, value),
                        HirStmt::Expr(HirExpr::Call(
                            Box::new(HirExpr::Var(
                                "__thaw_set_pending_exception_object".to_string(),
                            )),
                            vec![object_var],
                        )),
                        HirStmt::Throw(message),
                    ]);
                }
                let value = self.coerce_primitive_to_string(value)?;
                Ok(vec![HirStmt::Throw(value)])
            }

            Stmt::Try(try_stmt) => {
                if try_stmt.handler.is_none() && try_stmt.finalizer.is_none() {
                    return Err("`try` needs a `catch` or `finally` block".into());
                }

                let mut body = self.lower_scoped_stmts(&try_stmt.block.stmts)?;
                let (catch_name, mut catch_body) = match &try_stmt.handler {
                    Some(handler) => {
                        let source_name = match &handler.param {
                            Some(Pat::Ident(binding)) => binding.id.sym.to_string(),
                            Some(_) => {
                                return Err(
                                    "only a simple identifier catch binding is supported".into(),
                                )
                            }
                            None => "_".to_string(),
                        };
                        let saved = self.bindings.clone();
                        let catch_name = self.bind_local(&source_name, HirType::Str);
                        let catch_body = self.lower_stmts(&handler.body.stmts)?;
                        self.bindings = saved;
                        (catch_name, catch_body)
                    }
                    None => {
                        let mut name = "__thaw_finally_exception".to_string();
                        while self.scope.contains_key(&name) {
                            name.push('_');
                        }
                        let name = self.bind_local(&name, HirType::Str);
                        let rethrow = HirStmt::Throw(HirExpr::Var(name.clone()));
                        (name, vec![rethrow])
                    }
                };

                let mut after_try = Vec::new();
                if let Some(finalizer) = &try_stmt.finalizer {
                    let finalizer = self.lower_scoped_stmts(&finalizer.stmts)?;
                    body = inject_finally_before_exits(body, &finalizer, false);
                    catch_body =
                        inject_finally_before_exits(catch_body, &finalizer, true);
                    after_try = finalizer;
                }
                let mut lowered = vec![HirStmt::Try(body, catch_name, catch_body)];
                lowered.extend(after_try);
                Ok(lowered)
            }

            other => Err(format!(
                "unsupported statement {other:?} (Phase 0/1/2 support return/expr/let/if/while/for/throw/try)"
            )),
        }
    }

}
