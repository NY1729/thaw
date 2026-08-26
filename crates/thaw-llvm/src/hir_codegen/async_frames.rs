impl<'ctx> HirCompiler<'ctx> {
    fn frame_await_plan(&self, func: &HirFunction) -> Result<Option<FrameAsyncPlan>, String> {
        if !self.frame_async_functions.contains_key(&func.name) {
            return Ok(None);
        }
        let normalized_body = self.flatten_async_finally_only_tries(&func.body)?;
        let mut segments = vec![AsyncSegment {
            stmts: Vec::new(),
            awaited: None,
            await_guard: None,
            await_next: None,
            resume_target: None,
            rejection_handler: None,
        }];
        let mut found = false;
        let mut extra_locals = Vec::new();
        let mut guarded_rethrow_handlers = HashMap::new();
        let mut next_temporary = 0usize;
        let mut next_guard = 0usize;
        for stmt in &normalized_body {
            if let HirStmt::Try(try_body, catch_name, catch_body) = stmt {
                let frame_names = self
                    .frame_async_functions
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                if try_body
                    .iter()
                    .chain(catch_body)
                    .any(|nested| Self::stmt_awaits_frame_source(nested, &frame_names))
                {
                    found = true;
                    let try_guard = format!("__thaw_try_{next_guard}");
                    let catch_guard = format!("__thaw_catch_{next_guard}");
                    next_guard += 1;
                    extra_locals.push((catch_name.clone(), HirType::Str));
                    let current = segments.last_mut().unwrap();
                    current.stmts.push(HirStmt::Let(
                        try_guard.clone(),
                        HirType::Bool,
                        HirExpr::Lit(HirLit::Bool(true)),
                    ));
                    current.stmts.push(HirStmt::Let(
                        catch_guard.clone(),
                        HirType::Bool,
                        HirExpr::Lit(HirLit::Bool(false)),
                    ));
                    let outer_handler = AsyncRejectionHandler {
                        try_guard: try_guard.clone(),
                        catch_guard: catch_guard.clone(),
                        catch_binding: catch_name.clone(),
                        disable_guards: Vec::new(),
                    };
                    for nested in try_body {
                        if let HirStmt::Try(inner_body, inner_catch_name, inner_catch_body) = nested
                        {
                            let inner_try_guard = format!("__thaw_try_{next_guard}");
                            let inner_catch_guard = format!("__thaw_catch_{next_guard}");
                            next_guard += 1;
                            extra_locals.push((inner_catch_name.clone(), HirType::Str));
                            let current = segments.last_mut().unwrap();
                            current.stmts.push(HirStmt::Let(
                                inner_try_guard.clone(),
                                HirType::Bool,
                                HirExpr::Lit(HirLit::Bool(false)),
                            ));
                            current.stmts.push(HirStmt::Let(
                                inner_catch_guard.clone(),
                                HirType::Bool,
                                HirExpr::Lit(HirLit::Bool(false)),
                            ));
                            current.stmts.push(HirStmt::If(
                                HirExpr::Var(try_guard.clone()),
                                vec![HirStmt::Expr(HirExpr::Assign(
                                    inner_try_guard.clone(),
                                    Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                ))],
                                Vec::new(),
                            ));
                            let inner_handler = AsyncRejectionHandler {
                                try_guard: inner_try_guard.clone(),
                                catch_guard: inner_catch_guard.clone(),
                                catch_binding: inner_catch_name.clone(),
                                disable_guards: Vec::new(),
                            };
                            for inner_stmt in inner_body {
                                if let HirStmt::Try(
                                    nested_try_body,
                                    nested_catch_name,
                                    nested_catch_body,
                                ) = inner_stmt
                                {
                                    self.append_nested_async_try(
                                        &mut segments,
                                        nested_try_body,
                                        nested_catch_name,
                                        nested_catch_body,
                                        &inner_try_guard,
                                        Some(inner_handler.clone()),
                                        &frame_names,
                                        &mut extra_locals,
                                        &mut guarded_rethrow_handlers,
                                        &mut next_temporary,
                                        &mut next_guard,
                                    )?;
                                    continue;
                                }
                                if let HirStmt::Throw(error) = inner_stmt {
                                    segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                        HirExpr::Var(inner_try_guard.clone()),
                                        vec![
                                            HirStmt::Expr(HirExpr::Assign(
                                                inner_catch_name.clone(),
                                                Box::new(error.clone()),
                                            )),
                                            HirStmt::Expr(HirExpr::Assign(
                                                inner_try_guard.clone(),
                                                Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                            )),
                                            HirStmt::Expr(HirExpr::Assign(
                                                inner_catch_guard.clone(),
                                                Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                            )),
                                        ],
                                        Vec::new(),
                                    ));
                                } else {
                                    let first_new = segments.len();
                                    self.append_guarded_async_stmt(
                                        &mut segments,
                                        inner_stmt,
                                        &inner_try_guard,
                                        true,
                                        &[],
                                        &frame_names,
                                        &mut extra_locals,
                                        &mut guarded_rethrow_handlers,
                                        Some(inner_handler.clone()),
                                        &mut next_temporary,
                                        &mut next_guard,
                                    )?;
                                    for segment in &mut segments[first_new..] {
                                        if segment.rejection_handler.is_none() {
                                            segment.rejection_handler = Some(inner_handler.clone());
                                        }
                                    }
                                }
                            }
                            for inner_stmt in inner_catch_body {
                                if let HirStmt::Try(
                                    nested_try_body,
                                    nested_catch_name,
                                    nested_catch_body,
                                ) = inner_stmt
                                {
                                    let mut enclosing = outer_handler.clone();
                                    enclosing.disable_guards.push(inner_catch_guard.clone());
                                    self.append_nested_async_try(
                                        &mut segments,
                                        nested_try_body,
                                        nested_catch_name,
                                        nested_catch_body,
                                        &inner_catch_guard,
                                        Some(enclosing),
                                        &frame_names,
                                        &mut extra_locals,
                                        &mut guarded_rethrow_handlers,
                                        &mut next_temporary,
                                        &mut next_guard,
                                    )?;
                                    continue;
                                }
                                let first_new = segments.len();
                                if let HirStmt::Throw(error) = inner_stmt {
                                    let mut handler = outer_handler.clone();
                                    handler.disable_guards.push(inner_catch_guard.clone());
                                    guarded_rethrow_handlers
                                        .insert(inner_catch_guard.clone(), handler);
                                    segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                        HirExpr::Var(inner_catch_guard.clone()),
                                        vec![HirStmt::Throw(error.clone())],
                                        Vec::new(),
                                    ));
                                } else {
                                    self.append_guarded_async_stmt(
                                        &mut segments,
                                        inner_stmt,
                                        &inner_catch_guard,
                                        true,
                                        &[],
                                        &frame_names,
                                        &mut extra_locals,
                                        &mut guarded_rethrow_handlers,
                                        Some({
                                            let mut handler = outer_handler.clone();
                                            handler.disable_guards.push(inner_catch_guard.clone());
                                            handler
                                        }),
                                        &mut next_temporary,
                                        &mut next_guard,
                                    )?;
                                }
                                for segment in &mut segments[first_new..] {
                                    let mut handler = outer_handler.clone();
                                    handler.disable_guards.push(inner_catch_guard.clone());
                                    if segment.rejection_handler.is_none() {
                                        segment.rejection_handler = Some(handler);
                                    }
                                }
                            }
                            continue;
                        }
                        if let HirStmt::Throw(error) = nested {
                            segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                HirExpr::Var(try_guard.clone()),
                                vec![
                                    HirStmt::Expr(HirExpr::Assign(
                                        catch_name.clone(),
                                        Box::new(error.clone()),
                                    )),
                                    HirStmt::Expr(HirExpr::Assign(
                                        try_guard.clone(),
                                        Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                    )),
                                    HirStmt::Expr(HirExpr::Assign(
                                        catch_guard.clone(),
                                        Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                    )),
                                ],
                                Vec::new(),
                            ));
                        } else {
                            let first_new_segment = segments.len();
                            self.append_guarded_async_stmt(
                                &mut segments,
                                nested,
                                &try_guard,
                                true,
                                &[],
                                &frame_names,
                                &mut extra_locals,
                                &mut guarded_rethrow_handlers,
                                Some(outer_handler.clone()),
                                &mut next_temporary,
                                &mut next_guard,
                            )?;
                            for segment in &mut segments[first_new_segment..] {
                                if segment.rejection_handler.is_none() {
                                    segment.rejection_handler = Some(outer_handler.clone());
                                }
                            }
                        }
                    }
                    for nested in catch_body {
                        if let HirStmt::Throw(error) = nested {
                            segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                HirExpr::Var(catch_guard.clone()),
                                vec![HirStmt::Throw(error.clone())],
                                Vec::new(),
                            ));
                        } else {
                            self.append_guarded_async_stmt(
                                &mut segments,
                                nested,
                                &catch_guard,
                                true,
                                &[],
                                &frame_names,
                                &mut extra_locals,
                                &mut guarded_rethrow_handlers,
                                None,
                                &mut next_temporary,
                                &mut next_guard,
                            )?;
                        }
                    }
                    continue;
                }
            }
            if let HirStmt::While(cond, body) = stmt {
                let frame_names = self
                    .frame_async_functions
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                let condition_awaits = Self::expr_awaits_frame_source(cond, &frame_names);
                let body_awaits = body
                    .iter()
                    .any(|nested| Self::stmt_awaits_frame_source(nested, &frame_names));
                if condition_awaits || body_awaits {
                    let sleep_zero = || {
                        HirExpr::Call(
                            Box::new(HirExpr::Var("sleep".to_string())),
                            vec![HirExpr::Lit(HirLit::F64(0.0))],
                        )
                    };
                    found = true;
                    segments.last_mut().unwrap().awaited = Some(sleep_zero());
                    let condition_state = segments.len();
                    let guard = format!("__thaw_loop_{next_guard}");
                    let body_guard = format!("__thaw_loop_body_{next_guard}");
                    next_guard += 1;
                    segments.push(AsyncSegment {
                        stmts: Vec::new(),
                        awaited: None,
                        await_guard: None,
                        await_next: None,
                        resume_target: None,
                        rejection_handler: None,
                    });
                    let mut rewritten_condition = cond.clone();
                    loop {
                        let temporary = format!("__thaw_await_{next_temporary}");
                        let Some((awaited, ty)) =
                            self.extract_first_frame_await(&mut rewritten_condition, &temporary)?
                        else {
                            break;
                        };
                        next_temporary += 1;
                        segments.last_mut().unwrap().awaited = Some(awaited);
                        segments.push(AsyncSegment {
                            stmts: Vec::new(),
                            awaited: None,
                            await_guard: None,
                            await_next: None,
                            resume_target: Some((temporary, ty)),
                            rejection_handler: None,
                        });
                    }
                    segments.last_mut().unwrap().stmts.push(HirStmt::Let(
                        guard.clone(),
                        HirType::Bool,
                        rewritten_condition,
                    ));
                    segments.last_mut().unwrap().stmts.push(HirStmt::Let(
                        body_guard.clone(),
                        HirType::Bool,
                        HirExpr::Var(guard.clone()),
                    ));

                    for nested in body {
                        if matches!(
                            nested,
                            HirStmt::Break
                                | HirStmt::Continue
                                | HirStmt::BreakDepth(_)
                                | HirStmt::ContinueDepth(_)
                        ) {
                            let exits = Self::async_loop_exit_stmts(
                                nested,
                                &[(body_guard.clone(), guard.clone())],
                            )?;
                            segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                HirExpr::Var(body_guard.clone()),
                                exits,
                                Vec::new(),
                            ));
                            continue;
                        }
                        if let HirStmt::If(cond, then_body, else_body) = nested {
                            self.append_nested_async_if(
                                &mut segments,
                                cond,
                                then_body,
                                else_body,
                                &body_guard,
                                true,
                                &[(body_guard.clone(), guard.clone())],
                                &frame_names,
                                &mut extra_locals,
                                &mut guarded_rethrow_handlers,
                                None,
                                &mut next_temporary,
                                &mut next_guard,
                            )?;
                            continue;
                        }
                        if let HirStmt::While(cond, nested_body) = nested {
                            self.append_nested_async_while(
                                &mut segments,
                                cond,
                                nested_body,
                                &body_guard,
                                true,
                                &[(body_guard.clone(), guard.clone())],
                                &frame_names,
                                &mut extra_locals,
                                &mut guarded_rethrow_handlers,
                                None,
                                &mut next_temporary,
                                &mut next_guard,
                            )?;
                            continue;
                        }
                        if matches!(nested, HirStmt::Try(..))
                            || Self::stmt_contains_frame_unsupported(nested)
                        {
                            self.append_guarded_async_stmt(
                                &mut segments,
                                nested,
                                &body_guard,
                                true,
                                &[(body_guard.clone(), guard.clone())],
                                &frame_names,
                                &mut extra_locals,
                                &mut guarded_rethrow_handlers,
                                None,
                                &mut next_temporary,
                                &mut next_guard,
                            )?;
                            continue;
                        }
                        if let HirStmt::Expr(HirExpr::Await(inner)) = nested {
                            if self.is_frame_await_source(inner) {
                                segments.last_mut().unwrap().awaited = Some(inner.as_ref().clone());
                                segments.last_mut().unwrap().await_guard =
                                    Some((body_guard.clone(), true));
                                segments.push(AsyncSegment {
                                    stmts: Vec::new(),
                                    awaited: None,
                                    await_guard: None,
                                    await_next: None,
                                    resume_target: None,
                                    rejection_handler: None,
                                });
                                continue;
                            }
                        }
                        let mut rewritten = nested.clone();
                        loop {
                            let temporary = format!("__thaw_await_{next_temporary}");
                            let Some((awaited, ty)) = self
                                .extract_first_frame_await_from_stmt(&mut rewritten, &temporary)?
                            else {
                                break;
                            };
                            next_temporary += 1;
                            segments.last_mut().unwrap().awaited = Some(awaited);
                            segments.last_mut().unwrap().await_guard =
                                Some((body_guard.clone(), true));
                            segments.push(AsyncSegment {
                                stmts: Vec::new(),
                                awaited: None,
                                await_guard: None,
                                await_next: None,
                                resume_target: Some((temporary, ty)),
                                rejection_handler: None,
                            });
                        }
                        segments.last_mut().unwrap().stmts.push(HirStmt::If(
                            HirExpr::Var(body_guard.clone()),
                            vec![rewritten],
                            Vec::new(),
                        ));
                    }

                    segments.last_mut().unwrap().awaited = Some(sleep_zero());
                    segments.last_mut().unwrap().await_guard = Some((guard, true));
                    segments.last_mut().unwrap().await_next = Some(condition_state);
                    segments.push(AsyncSegment {
                        stmts: Vec::new(),
                        awaited: None,
                        await_guard: None,
                        await_next: None,
                        resume_target: None,
                        rejection_handler: None,
                    });
                    continue;
                }
            }
            if let HirStmt::If(cond, then_body, else_body) = stmt {
                let frame_names = self
                    .frame_async_functions
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                let branches_await = then_body
                    .iter()
                    .chain(else_body)
                    .any(|nested| Self::stmt_awaits_frame_source(nested, &frame_names));
                if branches_await {
                    found = true;
                    let mut rewritten_condition = cond.clone();
                    loop {
                        let temporary = format!("__thaw_await_{next_temporary}");
                        let Some((awaited, ty)) =
                            self.extract_first_frame_await(&mut rewritten_condition, &temporary)?
                        else {
                            break;
                        };
                        next_temporary += 1;
                        segments.last_mut().unwrap().awaited = Some(awaited);
                        segments.push(AsyncSegment {
                            stmts: Vec::new(),
                            awaited: None,
                            await_guard: None,
                            await_next: None,
                            resume_target: Some((temporary, ty)),
                            rejection_handler: None,
                        });
                    }
                    let guard = format!("__thaw_branch_{next_guard}");
                    next_guard += 1;
                    segments.last_mut().unwrap().stmts.push(HirStmt::Let(
                        guard.clone(),
                        HirType::Bool,
                        rewritten_condition,
                    ));
                    for (body, expected) in [(then_body, true), (else_body, false)] {
                        for nested in body {
                            if let HirStmt::If(cond, then_body, else_body) = nested {
                                self.append_nested_async_if(
                                    &mut segments,
                                    cond,
                                    then_body,
                                    else_body,
                                    &guard,
                                    expected,
                                    &[],
                                    &frame_names,
                                    &mut extra_locals,
                                    &mut guarded_rethrow_handlers,
                                    None,
                                    &mut next_temporary,
                                    &mut next_guard,
                                )?;
                                continue;
                            }
                            if matches!(nested, HirStmt::While(..) | HirStmt::Try(..))
                                || Self::stmt_contains_frame_unsupported(nested)
                            {
                                self.append_guarded_async_stmt(
                                    &mut segments,
                                    nested,
                                    &guard,
                                    expected,
                                    &[],
                                    &frame_names,
                                    &mut extra_locals,
                                    &mut guarded_rethrow_handlers,
                                    None,
                                    &mut next_temporary,
                                    &mut next_guard,
                                )?;
                                continue;
                            }
                            if let HirStmt::Expr(HirExpr::Await(inner)) = nested {
                                if self.is_frame_await_source(inner) {
                                    found = true;
                                    segments.last_mut().unwrap().awaited =
                                        Some(inner.as_ref().clone());
                                    segments.last_mut().unwrap().await_guard =
                                        Some((guard.clone(), expected));
                                    segments.push(AsyncSegment {
                                        stmts: Vec::new(),
                                        awaited: None,
                                        await_guard: None,
                                        await_next: None,
                                        resume_target: None,
                                        rejection_handler: None,
                                    });
                                    continue;
                                }
                            }
                            let mut rewritten = nested.clone();
                            loop {
                                let temporary = format!("__thaw_await_{next_temporary}");
                                let Some((awaited, ty)) = self
                                    .extract_first_frame_await_from_stmt(
                                        &mut rewritten,
                                        &temporary,
                                    )?
                                else {
                                    break;
                                };
                                next_temporary += 1;
                                found = true;
                                segments.last_mut().unwrap().awaited = Some(awaited);
                                segments.last_mut().unwrap().await_guard =
                                    Some((guard.clone(), expected));
                                segments.push(AsyncSegment {
                                    stmts: Vec::new(),
                                    awaited: None,
                                    await_guard: None,
                                    await_next: None,
                                    resume_target: Some((temporary, ty)),
                                    rejection_handler: None,
                                });
                            }
                            segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                HirExpr::Var(guard.clone()),
                                if expected {
                                    vec![rewritten.clone()]
                                } else {
                                    Vec::new()
                                },
                                if expected {
                                    Vec::new()
                                } else {
                                    vec![rewritten]
                                },
                            ));
                        }
                    }
                    continue;
                }
            }
            let boundary = match stmt {
                HirStmt::Expr(HirExpr::Await(inner)) if self.is_frame_await_source(inner) => {
                    Some((inner.as_ref().clone(), None))
                }
                HirStmt::Let(name, ty, HirExpr::Await(inner))
                    if self.is_frame_await_source(inner) =>
                {
                    Some((inner.as_ref().clone(), Some((name.clone(), ty.clone()))))
                }
                _ => None,
            };
            if let Some((awaited, target)) = boundary {
                found = true;
                segments.last_mut().unwrap().awaited = Some(awaited);
                segments.push(AsyncSegment {
                    stmts: Vec::new(),
                    awaited: None,
                    await_guard: None,
                    await_next: None,
                    resume_target: target,
                    rejection_handler: None,
                });
            } else {
                let frame_names = self
                    .frame_async_functions
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                if Self::stmt_awaits_frame_source(stmt, &frame_names) {
                    match stmt {
                        HirStmt::If(_, then_body, else_body)
                            if then_body.iter().chain(else_body).any(|nested| {
                                Self::stmt_awaits_frame_source(nested, &frame_names)
                            }) =>
                        {
                            return Err(
                                "frame-split `await` inside an if branch is not supported yet"
                                    .to_string(),
                            );
                        }
                        HirStmt::While(..) => {
                            return Err("frame-split `await` inside a loop is not supported yet"
                                .to_string());
                        }
                        HirStmt::Try(..) => {
                            return Err(
                                "frame-split `await` inside try/catch is not supported yet"
                                    .to_string(),
                            );
                        }
                        _ => {}
                    }
                    let mut rewritten = stmt.clone();
                    loop {
                        let temporary = format!("__thaw_await_{next_temporary}");
                        let Some((awaited, ty)) =
                            self.extract_first_frame_await_from_stmt(&mut rewritten, &temporary)?
                        else {
                            break;
                        };
                        next_temporary += 1;
                        found = true;
                        segments.last_mut().unwrap().awaited = Some(awaited);
                        segments.push(AsyncSegment {
                            stmts: Vec::new(),
                            awaited: None,
                            await_guard: None,
                            await_next: None,
                            resume_target: Some((temporary, ty)),
                            rejection_handler: None,
                        });
                    }
                    segments.last_mut().unwrap().stmts.push(rewritten);
                    continue;
                }
                segments.last_mut().unwrap().stmts.push(stmt.clone());
            }
        }
        if !func.is_async {
            return if found {
                Err("frame-split await requires an async function".to_string())
            } else {
                Ok(None)
            };
        }
        if func.ret != HirType::Void {
            self.basic_type(&func.ret)
                .map_err(|e| format!("async frame result: {e}"))?;
        }
        let mut locals = func
            .params
            .iter()
            .map(|param| {
                self.basic_type(&param.ty)
                    .map_err(|e| format!("async frame parameter `{}`: {e}", param.name))?;
                Ok((param.name.clone(), param.ty.clone()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        locals.extend(extra_locals);
        for stmt in segments.iter().flat_map(|segment| &segment.stmts) {
            self.collect_async_frame_locals(stmt, &mut locals)?;
        }
        for (name, ty) in segments
            .iter()
            .filter_map(|segment| segment.resume_target.as_ref())
        {
            if locals.iter().any(|(existing, _)| existing == name) {
                return Err(format!("duplicate async frame local `{name}`"));
            }
            self.basic_type(ty)
                .map_err(|e| format!("async frame local `{name}`: {e}"))?;
            locals.push((name.clone(), ty.clone()));
        }
        Ok(Some(FrameAsyncPlan {
            segments,
            locals,
            ret: func.ret.clone(),
            guarded_rethrow_handlers,
        }))
    }

    fn collect_async_frame_locals(
        &self,
        stmt: &HirStmt,
        locals: &mut Vec<(String, HirType)>,
    ) -> Result<(), String> {
        match stmt {
            HirStmt::Let(name, ty, _) => {
                if locals.iter().any(|(existing, _)| existing == name) {
                    return Err(format!("duplicate async frame local `{name}`"));
                }
                self.basic_type(ty)
                    .map_err(|e| format!("async frame local `{name}`: {e}"))?;
                locals.push((name.clone(), ty.clone()));
            }
            HirStmt::If(_, then_body, else_body) => {
                for nested in then_body.iter().chain(else_body) {
                    self.collect_async_frame_locals(nested, locals)?;
                }
            }
            HirStmt::While(_, body) => {
                for nested in body {
                    self.collect_async_frame_locals(nested, locals)?;
                }
            }
            HirStmt::Try(body, _, catch_body) => {
                for nested in body.iter().chain(catch_body) {
                    self.collect_async_frame_locals(nested, locals)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn flatten_async_finally_only_tries(&self, body: &[HirStmt]) -> Result<Vec<HirStmt>, String> {
        let mut flattened = Vec::new();
        for stmt in body {
            let HirStmt::Try(try_body, catch_name, catch_body) = stmt else {
                flattened.push(stmt.clone());
                continue;
            };
            let synthetic_finally_catch = catch_name.starts_with("__thaw_finally_exception")
                && matches!(
                    catch_body.last(),
                    Some(HirStmt::Throw(HirExpr::Var(name))) if name == catch_name
                );
            let contains_frame_await = try_body.iter().any(|stmt| {
                let names = self
                    .frame_async_functions
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                Self::stmt_awaits_frame_source(stmt, &names)
            });
            if !contains_frame_await {
                flattened.push(stmt.clone());
                continue;
            }
            if !synthetic_finally_catch {
                flattened.push(stmt.clone());
                continue;
            }
            let frame_names = self
                .frame_async_functions
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            let may_reject = try_body.iter().any(Self::stmt_contains_throw)
                || try_body
                    .iter()
                    .any(|stmt| Self::stmt_awaits_named_async(stmt, &frame_names));
            if may_reject {
                flattened.push(stmt.clone());
                continue;
            }
            flattened.extend(try_body.iter().cloned());
        }
        Ok(flattened)
    }

    fn stmt_contains_throw(stmt: &HirStmt) -> bool {
        match stmt {
            HirStmt::Throw(_) => true,
            HirStmt::If(_, then_body, else_body) => then_body
                .iter()
                .chain(else_body)
                .any(Self::stmt_contains_throw),
            HirStmt::While(_, body) => body.iter().any(Self::stmt_contains_throw),
            HirStmt::Try(body, _, catch_body) => {
                body.iter().chain(catch_body).any(Self::stmt_contains_throw)
            }
            _ => false,
        }
    }

    fn stmt_awaits_named_async(
        stmt: &HirStmt,
        frame_names: &std::collections::HashSet<String>,
    ) -> bool {
        match stmt {
            HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                Self::expr_awaits_named_async(expr, frame_names)
            }
            HirStmt::Return(Some(expr)) => Self::expr_awaits_named_async(expr, frame_names),
            HirStmt::If(cond, then_body, else_body) => {
                Self::expr_awaits_named_async(cond, frame_names)
                    || then_body
                        .iter()
                        .chain(else_body)
                        .any(|stmt| Self::stmt_awaits_named_async(stmt, frame_names))
            }
            HirStmt::While(cond, body) => {
                Self::expr_awaits_named_async(cond, frame_names)
                    || body
                        .iter()
                        .any(|stmt| Self::stmt_awaits_named_async(stmt, frame_names))
            }
            HirStmt::Try(body, _, catch_body) => body
                .iter()
                .chain(catch_body)
                .any(|stmt| Self::stmt_awaits_named_async(stmt, frame_names)),
            _ => false,
        }
    }

    fn expr_awaits_named_async(
        expr: &HirExpr,
        frame_names: &std::collections::HashSet<String>,
    ) -> bool {
        match expr {
            HirExpr::AwaitPromise(_, _) => true,
            HirExpr::Await(inner) => {
                matches!(
                    inner.as_ref(),
                    HirExpr::PromiseAll(_, _)
                        | HirExpr::PromiseAllArray(_, _)
                        | HirExpr::PromiseAllTuple(_, _)
                        | HirExpr::PromiseRace(_, _)
                        | HirExpr::PromiseRaceArray(_, _)
                        | HirExpr::PromiseAny(_, _)
                        | HirExpr::PromiseAnyArray(_, _)
                        | HirExpr::PromiseAllSettled(_, _)
                        | HirExpr::PromiseAllSettledArray(_, _)
                ) || matches!(inner.as_ref(), HirExpr::Call(callee, _)
                    if matches!(callee.as_ref(), HirExpr::Var(name)
                        if name == "fetch" || name == "Promise.all" || frame_names.contains(name)))
                    || Self::expr_awaits_named_async(inner, frame_names)
            }
            HirExpr::BinOp(_, left, right)
            | HirExpr::Index(left, right)
            | HirExpr::TypedIndex(left, right, _)
            | HirExpr::DynamicPropAccess(left, right, _, _) => {
                Self::expr_awaits_named_async(left, frame_names)
                    || Self::expr_awaits_named_async(right, frame_names)
            }
            HirExpr::Assign(_, value) => Self::expr_awaits_named_async(value, frame_names),
            HirExpr::EnumReverseLookup(value, _) => {
                Self::expr_awaits_named_async(value, frame_names)
            }
            HirExpr::Call(callee, args) => {
                Self::expr_awaits_named_async(callee, frame_names)
                    || args
                        .iter()
                        .any(|arg| Self::expr_awaits_named_async(arg, frame_names))
            }
            _ => false,
        }
    }

    fn is_frame_await_source(&self, expr: &HirExpr) -> bool {
        matches!(
            expr,
            HirExpr::PromiseNew(_, _, _)
                | HirExpr::PromiseThen(_, _, _, _, _, _)
                | HirExpr::PromiseFinally(_, _, _, _)
                | HirExpr::PromiseAll(_, _)
                | HirExpr::PromiseAllArray(_, _)
                | HirExpr::PromiseAllTuple(_, _)
                | HirExpr::PromiseRace(_, _)
                | HirExpr::PromiseRaceArray(_, _)
                | HirExpr::PromiseAny(_, _)
                | HirExpr::PromiseAnyArray(_, _)
                | HirExpr::PromiseAllSettled(_, _)
                | HirExpr::PromiseAllSettledArray(_, _)
        ) || matches!(expr, HirExpr::Call(callee, _)
            if matches!(callee.as_ref(), HirExpr::Var(name)
                if name == "sleep" || name == "fetch" || name == "Promise.all" || self.frame_async_functions.contains_key(name)))
    }

    fn extract_first_frame_await_from_stmt(
        &self,
        stmt: &mut HirStmt,
        temporary: &str,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        match stmt {
            HirStmt::Expr(expr)
            | HirStmt::Return(Some(expr))
            | HirStmt::Let(_, _, expr)
            | HirStmt::Throw(expr)
            | HirStmt::If(expr, _, _) => self.extract_first_frame_await(expr, temporary),
            _ => Ok(None),
        }
    }

    fn extract_first_frame_await(
        &self,
        expr: &mut HirExpr,
        temporary: &str,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        if let HirExpr::AwaitPromise(inner, resolved) = expr {
            let awaited = inner.as_ref().clone();
            let ty = resolved.clone();
            *expr = HirExpr::Var(temporary.to_string());
            return Ok(Some((awaited, ty)));
        }

        if let HirExpr::Await(inner) = expr {
            if self.is_frame_await_source(inner) {
                let awaited = inner.as_ref().clone();
                let ty = match inner.as_ref() {
                    HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "sleep") =>
                    {
                        return Err("the void result of `await sleep(...)` cannot be used inside an expression".to_string());
                    }
                    HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "fetch") => {
                        HirType::Str
                    }
                    HirExpr::PromiseNew(_, resolved, _) => resolved.clone(),
                    HirExpr::PromiseThen(_, _, _, output, _, _) => output.clone(),
                    HirExpr::PromiseFinally(_, _, input, _) => input.clone(),
                    HirExpr::PromiseAll(_, element) | HirExpr::PromiseAllArray(_, element) => {
                        HirType::Array(Box::new(element.clone()))
                    }
                    HirExpr::PromiseAllTuple(_, elements) => HirType::Tuple(elements.clone()),
                    HirExpr::PromiseRace(_, element) | HirExpr::PromiseRaceArray(_, element) => {
                        element.clone()
                    }
                    HirExpr::PromiseAny(_, element) | HirExpr::PromiseAnyArray(_, element) => {
                        element.clone()
                    }
                    HirExpr::PromiseAllSettled(_, element)
                    | HirExpr::PromiseAllSettledArray(_, element) => {
                        HirType::Array(Box::new(HirType::Object(vec![
                            ("status".into(), HirType::Str),
                            ("value".into(), element.clone()),
                            ("reason".into(), HirType::Str),
                        ])))
                    }
                    HirExpr::Call(callee, _) => {
                        let HirExpr::Var(name) = callee.as_ref() else {
                            unreachable!()
                        };
                        self.frame_async_functions
                            .get(name)
                            .cloned()
                            .ok_or_else(|| format!("missing async result type for `{name}`"))?
                    }
                    _ => unreachable!(),
                };
                *expr = HirExpr::Var(temporary.to_string());
                return Ok(Some((awaited, ty)));
            }
        }

        match expr {
            HirExpr::BinOp(_, left, right)
            | HirExpr::Index(left, right)
            | HirExpr::TypedIndex(left, right, _)
            | HirExpr::DynamicPropAccess(left, right, _, _) => {
                if let Some(found) = self.extract_first_frame_await(left, temporary)? {
                    Ok(Some(found))
                } else {
                    self.extract_first_frame_await(right, temporary)
                }
            }
            HirExpr::Call(callee, args) => {
                if let Some(found) = self.extract_first_frame_await(callee, temporary)? {
                    return Ok(Some(found));
                }
                for arg in args {
                    if let Some(found) = self.extract_first_frame_await(arg, temporary)? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
            HirExpr::FfiCall(_, args)
            | HirExpr::DynamicCall(_, args)
            | HirExpr::ArrayLit(args)
            | HirExpr::ArrayConcat(args, _)
            | HirExpr::PromiseAll(args, _)
            | HirExpr::PromiseAllTuple(args, _)
            | HirExpr::PromiseRace(args, _)
            | HirExpr::PromiseAny(args, _)
            | HirExpr::PromiseAllSettled(args, _) => {
                for arg in args {
                    if let Some(found) = self.extract_first_frame_await(arg, temporary)? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
            HirExpr::Assign(_, value)
            | HirExpr::PromiseAllArray(value, _)
            | HirExpr::PromiseRaceArray(value, _)
            | HirExpr::PromiseAnyArray(value, _)
            | HirExpr::PromiseAllSettledArray(value, _)
            | HirExpr::ArrayLen(value)
            | HirExpr::EnumReverseLookup(value, _)
            | HirExpr::JsonGet(value, _)
            | HirExpr::JsonAsNumber(value)
            | HirExpr::JsonAsString(value)
            | HirExpr::JsonAsBool(value) => self.extract_first_frame_await(value, temporary),
            HirExpr::IndexAssign(a, b, c) => {
                for value in [a, b, c] {
                    if let Some(found) = self.extract_first_frame_await(value, temporary)? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
            HirExpr::JsonSet(object, key, value, _) => {
                for value in [object, key, value] {
                    if let Some(found) = self.extract_first_frame_await(value, temporary)? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
            HirExpr::ObjectLit(fields) | HirExpr::JsonObjectLit(fields, _) => {
                for (_, value) in fields {
                    if let Some(found) = self.extract_first_frame_await(value, temporary)? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
            HirExpr::PropAccess(obj, _, _) => self.extract_first_frame_await(obj, temporary),
            HirExpr::PropAssign(obj, _, _, value)
            | HirExpr::JsonIndex(obj, value)
            | HirExpr::JsonKey(obj, value) => {
                if let Some(found) = self.extract_first_frame_await(obj, temporary)? {
                    Ok(Some(found))
                } else {
                    self.extract_first_frame_await(value, temporary)
                }
            }
            HirExpr::Await(inner) | HirExpr::AwaitPromise(inner, _) => {
                self.extract_first_frame_await(inner, temporary)
            }
            _ => Ok(None),
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn append_nested_async_try(
        &self,
        segments: &mut Vec<AsyncSegment>,
        try_body: &[HirStmt],
        catch_name: &str,
        catch_body: &[HirStmt],
        activation_guard: &str,
        enclosing_handler: Option<AsyncRejectionHandler>,
        frame_names: &std::collections::HashSet<String>,
        extra_locals: &mut Vec<(String, HirType)>,
        guarded_rethrow_handlers: &mut HashMap<String, AsyncRejectionHandler>,
        next_temporary: &mut usize,
        next_guard: &mut usize,
    ) -> Result<(), String> {
        let try_guard = format!("__thaw_try_{}", *next_guard);
        let catch_guard = format!("__thaw_catch_{}", *next_guard);
        *next_guard += 1;
        extra_locals.push((catch_name.to_string(), HirType::Str));

        let current = segments.last_mut().unwrap();
        current.stmts.push(HirStmt::Let(
            try_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        current.stmts.push(HirStmt::Let(
            catch_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        current.stmts.push(HirStmt::If(
            HirExpr::Var(activation_guard.to_string()),
            vec![HirStmt::Expr(HirExpr::Assign(
                try_guard.clone(),
                Box::new(HirExpr::Lit(HirLit::Bool(true))),
            ))],
            Vec::new(),
        ));

        let handler = AsyncRejectionHandler {
            try_guard: try_guard.clone(),
            catch_guard: catch_guard.clone(),
            catch_binding: catch_name.to_string(),
            disable_guards: Vec::new(),
        };
        for stmt in try_body {
            if let HirStmt::Try(nested_try, nested_name, nested_catch) = stmt {
                self.append_nested_async_try(
                    segments,
                    nested_try,
                    nested_name,
                    nested_catch,
                    &try_guard,
                    Some(handler.clone()),
                    frame_names,
                    extra_locals,
                    guarded_rethrow_handlers,
                    next_temporary,
                    next_guard,
                )?;
                continue;
            }
            if let HirStmt::Throw(error) = stmt {
                segments.last_mut().unwrap().stmts.push(HirStmt::If(
                    HirExpr::Var(try_guard.clone()),
                    vec![
                        HirStmt::Expr(HirExpr::Assign(
                            catch_name.to_string(),
                            Box::new(error.clone()),
                        )),
                        HirStmt::Expr(HirExpr::Assign(
                            try_guard.clone(),
                            Box::new(HirExpr::Lit(HirLit::Bool(false))),
                        )),
                        HirStmt::Expr(HirExpr::Assign(
                            catch_guard.clone(),
                            Box::new(HirExpr::Lit(HirLit::Bool(true))),
                        )),
                    ],
                    Vec::new(),
                ));
                continue;
            }
            let first_new = segments.len();
            self.append_guarded_async_stmt(
                segments,
                stmt,
                &try_guard,
                true,
                &[],
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                Some(handler.clone()),
                next_temporary,
                next_guard,
            )?;
            for segment in &mut segments[first_new..] {
                if segment.rejection_handler.is_none() {
                    segment.rejection_handler = Some(handler.clone());
                }
            }
        }

        let catch_enclosing = enclosing_handler.map(|mut outer| {
            outer.disable_guards.push(catch_guard.clone());
            outer
        });
        for stmt in catch_body {
            if let HirStmt::Try(nested_try, nested_name, nested_catch) = stmt {
                self.append_nested_async_try(
                    segments,
                    nested_try,
                    nested_name,
                    nested_catch,
                    &catch_guard,
                    catch_enclosing.clone(),
                    frame_names,
                    extra_locals,
                    guarded_rethrow_handlers,
                    next_temporary,
                    next_guard,
                )?;
                continue;
            }
            let first_new = segments.len();
            if let HirStmt::Throw(error) = stmt {
                if let Some(outer) = &catch_enclosing {
                    guarded_rethrow_handlers.insert(catch_guard.clone(), outer.clone());
                }
                segments.last_mut().unwrap().stmts.push(HirStmt::If(
                    HirExpr::Var(catch_guard.clone()),
                    vec![HirStmt::Throw(error.clone())],
                    Vec::new(),
                ));
            } else {
                self.append_guarded_async_stmt(
                    segments,
                    stmt,
                    &catch_guard,
                    true,
                    &[],
                    frame_names,
                    extra_locals,
                    guarded_rethrow_handlers,
                    catch_enclosing.clone(),
                    next_temporary,
                    next_guard,
                )?;
            }
            if let Some(outer) = &catch_enclosing {
                for segment in &mut segments[first_new..] {
                    if segment.rejection_handler.is_none() {
                        segment.rejection_handler = Some(outer.clone());
                    }
                }
            }
        }
        Ok(())
    }

    fn async_loop_exit_stmts(
        stmt: &HirStmt,
        loop_guards: &[(String, String)],
    ) -> Result<Vec<HirStmt>, String> {
        let (depth, is_break) = match stmt {
            HirStmt::Break => (0, true),
            HirStmt::Continue => (0, false),
            HirStmt::BreakDepth(depth) => (*depth, true),
            HirStmt::ContinueDepth(depth) => (*depth, false),
            _ => return Err("expected async loop control statement".to_string()),
        };
        let target = loop_guards
            .len()
            .checked_sub(depth + 1)
            .ok_or("labeled loop target is outside the active async loop stack")?;
        let mut exits = Vec::new();
        for (index, (body_guard, loop_guard)) in loop_guards.iter().enumerate().skip(target) {
            exits.push(HirStmt::Expr(HirExpr::Assign(
                body_guard.clone(),
                Box::new(HirExpr::Lit(HirLit::Bool(false))),
            )));
            if is_break || index > target {
                exits.push(HirStmt::Expr(HirExpr::Assign(
                    loop_guard.clone(),
                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                )));
            }
        }
        Ok(exits)
    }

    // The loop splitter threads the surrounding control-flow destinations and
    // frame layout explicitly; grouping them would only hide this pass state.
    #[allow(clippy::too_many_arguments)]
    fn append_nested_async_while(
        &self,
        segments: &mut Vec<AsyncSegment>,
        cond: &HirExpr,
        body: &[HirStmt],
        parent_guard: &str,
        parent_expected: bool,
        outer_loop_guards: &[(String, String)],
        frame_names: &std::collections::HashSet<String>,
        extra_locals: &mut Vec<(String, HirType)>,
        guarded_rethrow_handlers: &mut HashMap<String, AsyncRejectionHandler>,
        rejection_handler: Option<AsyncRejectionHandler>,
        next_temporary: &mut usize,
        next_guard: &mut usize,
    ) -> Result<(), String> {
        let suffix = *next_guard;
        *next_guard += 1;
        let enabled_guard = format!("__thaw_nested_loop_enabled_{suffix}");
        let loop_guard = format!("__thaw_nested_loop_{suffix}");
        let body_guard = format!("__thaw_nested_loop_body_{suffix}");
        let mut loop_guards = outer_loop_guards.to_vec();
        loop_guards.push((body_guard.clone(), loop_guard.clone()));
        let loop_rejection_handler = rejection_handler.clone().map(|mut handler| {
            handler.disable_guards.extend([
                enabled_guard.clone(),
                loop_guard.clone(),
                body_guard.clone(),
            ]);
            handler
        });
        let current = segments.last_mut().unwrap();
        current.stmts.push(HirStmt::Let(
            enabled_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        let enable = HirStmt::Expr(HirExpr::Assign(
            enabled_guard.clone(),
            Box::new(HirExpr::Lit(HirLit::Bool(true))),
        ));
        current.stmts.push(HirStmt::If(
            HirExpr::Var(parent_guard.to_string()),
            if parent_expected {
                vec![enable.clone()]
            } else {
                Vec::new()
            },
            if parent_expected {
                Vec::new()
            } else {
                vec![enable]
            },
        ));

        let sleep_zero = || {
            HirExpr::Call(
                Box::new(HirExpr::Var("sleep".to_string())),
                vec![HirExpr::Lit(HirLit::F64(0.0))],
            )
        };
        current.awaited = Some(sleep_zero());
        current.await_guard = Some((enabled_guard.clone(), true));
        let condition_state = segments.len();
        segments.push(AsyncSegment {
            stmts: Vec::new(),
            awaited: None,
            await_guard: None,
            await_next: None,
            resume_target: None,
            rejection_handler: None,
        });

        let mut rewritten_condition = cond.clone();
        loop {
            let temporary = format!("__thaw_await_{}", *next_temporary);
            let Some((awaited, ty)) =
                self.extract_first_frame_await(&mut rewritten_condition, &temporary)?
            else {
                break;
            };
            *next_temporary += 1;
            let current = segments.last_mut().unwrap();
            current.awaited = Some(awaited);
            current.await_guard = Some((enabled_guard.clone(), true));
            segments.push(AsyncSegment {
                stmts: Vec::new(),
                awaited: None,
                await_guard: None,
                await_next: None,
                resume_target: Some((temporary, ty)),
                rejection_handler: None,
            });
        }

        let current = segments.last_mut().unwrap();
        current.stmts.push(HirStmt::Let(
            loop_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        current.stmts.push(HirStmt::If(
            HirExpr::Var(enabled_guard.clone()),
            vec![HirStmt::Expr(HirExpr::Assign(
                loop_guard.clone(),
                Box::new(rewritten_condition),
            ))],
            Vec::new(),
        ));
        current.stmts.push(HirStmt::Let(
            body_guard.clone(),
            HirType::Bool,
            HirExpr::Var(loop_guard.clone()),
        ));

        for stmt in body {
            self.append_guarded_async_stmt(
                segments,
                stmt,
                &body_guard,
                true,
                &loop_guards,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                loop_rejection_handler.clone(),
                next_temporary,
                next_guard,
            )?;
        }

        let current = segments.last_mut().unwrap();
        current.awaited = Some(sleep_zero());
        current.await_guard = Some((loop_guard, true));
        current.await_next = Some(condition_state);
        segments.push(AsyncSegment {
            stmts: Vec::new(),
            awaited: None,
            await_guard: None,
            await_next: None,
            resume_target: None,
            rejection_handler: None,
        });
        if let Some(handler) = loop_rejection_handler {
            for segment in &mut segments[condition_state..] {
                if segment.rejection_handler.is_none() {
                    segment.rejection_handler = Some(handler.clone());
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn append_nested_async_if(
        &self,
        segments: &mut Vec<AsyncSegment>,
        cond: &HirExpr,
        then_body: &[HirStmt],
        else_body: &[HirStmt],
        parent_guard: &str,
        parent_expected: bool,
        loop_guards: &[(String, String)],
        frame_names: &std::collections::HashSet<String>,
        extra_locals: &mut Vec<(String, HirType)>,
        guarded_rethrow_handlers: &mut HashMap<String, AsyncRejectionHandler>,
        rejection_handler: Option<AsyncRejectionHandler>,
        next_temporary: &mut usize,
        next_guard: &mut usize,
    ) -> Result<(), String> {
        let mut rewritten_condition = cond.clone();
        loop {
            let temporary = format!("__thaw_await_{}", *next_temporary);
            let Some((awaited, ty)) =
                self.extract_first_frame_await(&mut rewritten_condition, &temporary)?
            else {
                break;
            };
            *next_temporary += 1;
            let current = segments.last_mut().unwrap();
            current.awaited = Some(awaited);
            current.await_guard = Some((parent_guard.to_string(), parent_expected));
            segments.push(AsyncSegment {
                stmts: Vec::new(),
                awaited: None,
                await_guard: None,
                await_next: None,
                resume_target: Some((temporary, ty)),
                rejection_handler: None,
            });
        }
        let then_guard = format!("__thaw_nested_then_{}", *next_guard);
        let else_guard = format!("__thaw_nested_else_{}", *next_guard);
        *next_guard += 1;
        let current = segments.last_mut().unwrap();
        current.stmts.push(HirStmt::Let(
            then_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        current.stmts.push(HirStmt::Let(
            else_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        let choose = HirStmt::If(
            rewritten_condition,
            vec![HirStmt::Expr(HirExpr::Assign(
                then_guard.clone(),
                Box::new(HirExpr::Lit(HirLit::Bool(true))),
            ))],
            vec![HirStmt::Expr(HirExpr::Assign(
                else_guard.clone(),
                Box::new(HirExpr::Lit(HirLit::Bool(true))),
            ))],
        );
        current.stmts.push(HirStmt::If(
            HirExpr::Var(parent_guard.to_string()),
            if parent_expected {
                vec![choose.clone()]
            } else {
                Vec::new()
            },
            if parent_expected {
                Vec::new()
            } else {
                vec![choose]
            },
        ));

        for stmt in then_body {
            self.append_guarded_async_stmt(
                segments,
                stmt,
                &then_guard,
                true,
                loop_guards,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                rejection_handler.clone(),
                next_temporary,
                next_guard,
            )?;
        }
        for stmt in else_body {
            self.append_guarded_async_stmt(
                segments,
                stmt,
                &else_guard,
                true,
                loop_guards,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                rejection_handler.clone(),
                next_temporary,
                next_guard,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn append_guarded_async_stmt(
        &self,
        segments: &mut Vec<AsyncSegment>,
        stmt: &HirStmt,
        guard: &str,
        expected: bool,
        loop_guards: &[(String, String)],
        frame_names: &std::collections::HashSet<String>,
        extra_locals: &mut Vec<(String, HirType)>,
        guarded_rethrow_handlers: &mut HashMap<String, AsyncRejectionHandler>,
        rejection_handler: Option<AsyncRejectionHandler>,
        next_temporary: &mut usize,
        next_guard: &mut usize,
    ) -> Result<(), String> {
        let first_new_segment = segments.len();
        if let HirStmt::If(cond, then_body, else_body) = stmt {
            return self.append_nested_async_if(
                segments,
                cond,
                then_body,
                else_body,
                guard,
                expected,
                loop_guards,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                rejection_handler.clone(),
                next_temporary,
                next_guard,
            );
        }
        if matches!(
            stmt,
            HirStmt::Break | HirStmt::Continue | HirStmt::BreakDepth(_) | HirStmt::ContinueDepth(_)
        ) {
            let exits = Self::async_loop_exit_stmts(stmt, loop_guards)?;
            segments.last_mut().unwrap().stmts.push(HirStmt::If(
                HirExpr::Var(guard.to_string()),
                if expected { exits.clone() } else { Vec::new() },
                if expected { Vec::new() } else { exits },
            ));
            return Ok(());
        }
        if let HirStmt::While(cond, body) = stmt {
            return self.append_nested_async_while(
                segments,
                cond,
                body,
                guard,
                expected,
                loop_guards,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                rejection_handler.clone(),
                next_temporary,
                next_guard,
            );
        }
        if let HirStmt::Try(try_body, catch_name, catch_body) = stmt {
            let activation_guard = if expected {
                guard.to_string()
            } else {
                let active = format!("__thaw_active_{}", *next_guard);
                *next_guard += 1;
                let current = segments.last_mut().unwrap();
                current.stmts.push(HirStmt::Let(
                    active.clone(),
                    HirType::Bool,
                    HirExpr::Lit(HirLit::Bool(false)),
                ));
                current.stmts.push(HirStmt::If(
                    HirExpr::Var(guard.to_string()),
                    Vec::new(),
                    vec![HirStmt::Expr(HirExpr::Assign(
                        active.clone(),
                        Box::new(HirExpr::Lit(HirLit::Bool(true))),
                    ))],
                ));
                active
            };
            return self.append_nested_async_try(
                segments,
                try_body,
                catch_name,
                catch_body,
                &activation_guard,
                rejection_handler,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                next_temporary,
                next_guard,
            );
        }
        if matches!(stmt, HirStmt::Throw(_)) {
            if let Some(handler) = &rejection_handler {
                guarded_rethrow_handlers.insert(guard.to_string(), handler.clone());
            }
        }
        if let HirStmt::Expr(HirExpr::Await(inner)) = stmt {
            if self.is_frame_await_source(inner) {
                let current = segments.last_mut().unwrap();
                current.awaited = Some(inner.as_ref().clone());
                current.await_guard = Some((guard.to_string(), expected));
                segments.push(AsyncSegment {
                    stmts: Vec::new(),
                    awaited: None,
                    await_guard: None,
                    await_next: None,
                    resume_target: None,
                    rejection_handler: None,
                });
                if let Some(handler) = rejection_handler {
                    for segment in &mut segments[first_new_segment..] {
                        if segment.rejection_handler.is_none() {
                            segment.rejection_handler = Some(handler.clone());
                        }
                    }
                }
                return Ok(());
            }
        }
        let mut rewritten = stmt.clone();
        loop {
            let temporary = format!("__thaw_await_{}", *next_temporary);
            let Some((awaited, ty)) =
                self.extract_first_frame_await_from_stmt(&mut rewritten, &temporary)?
            else {
                break;
            };
            *next_temporary += 1;
            let current = segments.last_mut().unwrap();
            current.awaited = Some(awaited);
            current.await_guard = Some((guard.to_string(), expected));
            segments.push(AsyncSegment {
                stmts: Vec::new(),
                awaited: None,
                await_guard: None,
                await_next: None,
                resume_target: Some((temporary, ty)),
                rejection_handler: None,
            });
        }
        segments.last_mut().unwrap().stmts.push(HirStmt::If(
            HirExpr::Var(guard.to_string()),
            if expected {
                vec![rewritten.clone()]
            } else {
                Vec::new()
            },
            if expected {
                Vec::new()
            } else {
                vec![rewritten]
            },
        ));
        if let Some(handler) = rejection_handler {
            for segment in &mut segments[first_new_segment..] {
                if segment.rejection_handler.is_none() {
                    segment.rejection_handler = Some(handler.clone());
                }
            }
        }
        Ok(())
    }

    fn stmt_contains_frame_unsupported(stmt: &HirStmt) -> bool {
        match stmt {
            HirStmt::Let(..) | HirStmt::Return(..) | HirStmt::Throw(..) | HirStmt::Try(..) => true,
            HirStmt::If(_, then_body, else_body) => {
                if matches!(
                    (then_body.as_slice(), else_body.as_slice()),
                    ([HirStmt::Throw(_)], [])
                ) {
                    return false;
                }
                then_body.iter().any(Self::stmt_contains_frame_unsupported)
                    || else_body.iter().any(Self::stmt_contains_frame_unsupported)
            }
            HirStmt::While(_, body) => body.iter().any(Self::stmt_contains_frame_unsupported),
            _ => false,
        }
    }

    fn async_frame_field(
        &self,
        frame: PointerValue<'ctx>,
        offset: u64,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let offset = self.context.i64_type().const_int(offset, false);
        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), frame, &[offset], name)
                .map_err(|e| e.to_string())
        }
    }

    fn compile_frame_async_function(
        &mut self,
        func: &HirFunction,
        plan: &FrameAsyncPlan,
    ) -> Result<(), String> {
        let segments = &plan.segments;
        let symbol = Self::llvm_symbol_for(&func.name);
        let ramp = self.module.get_function(&symbol).unwrap();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let resume_ty = self
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        let resume = self.module.add_function(
            &format!("{symbol}.resume"),
            resume_ty,
            Some(Linkage::Internal),
        );

        let entry = self.context.append_basic_block(ramp, "entry");
        self.builder.position_at_end(entry);
        self.variables.clear();
        self.variable_hir_types.clear();
        self.catch_stack.clear();
        self.seed_global_variables();

        let alloc = self.module.get_function("thaw_arena_alloc").unwrap();
        let frame = self
            .builder
            .build_call(
                alloc,
                &[
                    self.context
                        .i64_type()
                        .const_int(self.async_frame_size(plan), false)
                        .into(),
                    self.context.i64_type().const_int(8, false).into(),
                ],
                "async_frame",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("arena allocator returned no frame")?
            .into_pointer_value();
        let completion = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_new").unwrap(),
                &[],
                "async_completion",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("promise allocator returned no value")?
            .into_pointer_value();
        let completion_slot =
            self.async_frame_field(frame, ASYNC_COMPLETION_OFFSET, "completion_slot")?;
        self.builder
            .build_store(completion_slot, completion)
            .map_err(|e| e.to_string())?;
        let waiting_slot = self.async_frame_field(frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        self.bind_async_frame_locals(frame, plan)?;
        for (param_value, param) in ramp.get_param_iter().zip(&func.params) {
            let (slot, _) = self.variables.get(&param.name).copied().ok_or_else(|| {
                format!("missing async frame parameter slot for `{}`", param.name)
            })?;
            self.builder
                .build_store(slot, param_value)
                .map_err(|e| e.to_string())?;
        }
        self.emit_async_segment(&segments[0], frame, completion, resume, 1, true, plan, None)?;

        let resume_entry = self.context.append_basic_block(resume, "entry");
        self.builder.position_at_end(resume_entry);
        self.variables.clear();
        self.variable_hir_types.clear();
        self.catch_stack.clear();
        self.seed_global_variables();
        let resume_frame = resume.get_nth_param(0).unwrap().into_pointer_value();
        let resume_result = resume.get_nth_param(1).unwrap().into_pointer_value();
        let waiting_slot =
            self.async_frame_field(resume_frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
        let waiting = self
            .builder
            .build_load(ptr_ty, waiting_slot, "waiting_promise")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        let completion_slot =
            self.async_frame_field(resume_frame, ASYNC_COMPLETION_OFFSET, "completion_slot")?;
        let resume_completion = self
            .builder
            .build_load(ptr_ty, completion_slot, "completion")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        let state_slot = self.async_frame_field(resume_frame, ASYNC_STATE_OFFSET, "state_slot")?;
        let state = self
            .builder
            .build_load(self.context.i64_type(), state_slot, "async_state")
            .map_err(|e| e.to_string())?
            .into_int_value();
        let promise_state = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_state").unwrap(),
                &[waiting.into()],
                "waiting_state",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_promise_state returned no value")?
            .into_int_value();
        let is_rejected = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                promise_state,
                self.context.i8_type().const_int(2, false),
                "waiting_rejected",
            )
            .map_err(|e| e.to_string())?;
        let rejected = self.context.append_basic_block(resume, "handle_rejection");
        let resume_ok = self.context.append_basic_block(resume, "resume_fulfilled");
        self.builder
            .build_conditional_branch(is_rejected, rejected, resume_ok)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(rejected);
        let propagate_rejection = self
            .context
            .append_basic_block(resume, "propagate_rejection");
        let rejection_handlers = segments
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(index, segment)| {
                segment.rejection_handler.as_ref().map(|handler| {
                    (
                        index,
                        handler.clone(),
                        self.context
                            .append_basic_block(resume, &format!("catch_rejection_{index}")),
                    )
                })
            })
            .collect::<Vec<_>>();
        let rejection_cases = rejection_handlers
            .iter()
            .map(|(index, _, block)| {
                (
                    self.context.i64_type().const_int(*index as u64, false),
                    *block,
                )
            })
            .collect::<Vec<_>>();
        self.builder
            .build_switch(state, propagate_rejection, &rejection_cases)
            .map_err(|e| e.to_string())?;

        for (_, handler, block) in rejection_handlers {
            self.builder.position_at_end(block);
            for name in [handler.try_guard.as_str(), handler.catch_guard.as_str()] {
                let index = plan
                    .locals
                    .iter()
                    .position(|(local, _)| local == name)
                    .ok_or_else(|| format!("missing async rejection guard `{name}`"))?;
                let slot = self.async_frame_field(
                    resume_frame,
                    self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * index as u64,
                    &format!("frame_{name}"),
                )?;
                let enabled = name == handler.catch_guard;
                self.builder
                    .build_store(
                        slot,
                        self.context.bool_type().const_int(enabled as u64, false),
                    )
                    .map_err(|e| e.to_string())?;
            }
            for name in &handler.disable_guards {
                let index = plan
                    .locals
                    .iter()
                    .position(|(local, _)| local == name)
                    .ok_or_else(|| format!("missing async disabled guard `{name}`"))?;
                let slot = self.async_frame_field(
                    resume_frame,
                    self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * index as u64,
                    &format!("frame_{name}"),
                )?;
                self.builder
                    .build_store(slot, self.context.bool_type().const_zero())
                    .map_err(|e| e.to_string())?;
            }
            let binding_index = plan
                .locals
                .iter()
                .position(|(local, _)| local == &handler.catch_binding)
                .ok_or_else(|| {
                    format!("missing async catch binding `{}`", handler.catch_binding)
                })?;
            let binding_slot = self.async_frame_field(
                resume_frame,
                self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * binding_index as u64,
                &format!("frame_{}", handler.catch_binding),
            )?;
            self.builder
                .build_store(binding_slot, resume_result)
                .map_err(|e| e.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_destroy").unwrap(),
                    &[waiting.into()],
                    "destroy_caught_waiting",
                )
                .map_err(|e| e.to_string())?;
            self.builder
                .build_store(waiting_slot, ptr_ty.const_null())
                .map_err(|e| e.to_string())?;
            self.builder
                .build_call(
                    resume,
                    &[resume_frame.into(), resume_frame.into()],
                    "resume_catch",
                )
                .map_err(|e| e.to_string())?;
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        }

        self.builder.position_at_end(propagate_rejection);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_reject").unwrap(),
                &[resume_completion.into(), resume_result.into()],
                "reject_completion",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_destroy").unwrap(),
                &[waiting.into()],
                "destroy_rejected_waiting",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;

        self.builder.position_at_end(resume_ok);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_destroy").unwrap(),
                &[waiting.into()],
                "destroy_waiting",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        let invalid = self.context.append_basic_block(resume, "invalid_state");
        let case_blocks = (1..segments.len())
            .map(|index| {
                self.context
                    .append_basic_block(resume, &format!("state_{index}"))
            })
            .collect::<Vec<_>>();
        let cases = case_blocks
            .iter()
            .enumerate()
            .map(|(index, block)| {
                (
                    self.context.i64_type().const_int((index + 1) as u64, false),
                    *block,
                )
            })
            .collect::<Vec<_>>();
        self.builder
            .build_switch(state, invalid, &cases)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(invalid);
        self.builder.build_return(None).map_err(|e| e.to_string())?;

        for (index, block) in case_blocks.into_iter().enumerate() {
            self.builder.position_at_end(block);
            self.variables.clear();
            self.variable_hir_types.clear();
            self.catch_stack.clear();
            self.seed_global_variables();
            self.bind_async_frame_locals(resume_frame, plan)?;
            self.emit_async_segment(
                &segments[index + 1],
                resume_frame,
                resume_completion,
                resume,
                index + 2,
                false,
                plan,
                Some(resume_result),
            )?;
        }
        Ok(())
    }

    // Segment emission needs the coroutine frame plus all exceptional and
    // normal successors as distinct LLVM values.
    #[allow(clippy::too_many_arguments)]
    fn emit_async_segment(
        &mut self,
        segment: &AsyncSegment,
        frame: PointerValue<'ctx>,
        completion: PointerValue<'ctx>,
        resume: FunctionValue<'ctx>,
        next_state: usize,
        ramp: bool,
        plan: &FrameAsyncPlan,
        resume_result: Option<PointerValue<'ctx>>,
    ) -> Result<(), String> {
        if let Some((name, ty)) = &segment.resume_target {
            let result_ptr = resume_result.ok_or("async resume result is unavailable")?;
            let llvm_ty = self.basic_type(ty)?;
            let value = self
                .builder
                .build_load(llvm_ty, result_ptr, &format!("awaited_{name}"))
                .map_err(|e| e.to_string())?;
            let (slot, _) = self
                .variables
                .get(name)
                .copied()
                .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
            self.builder
                .build_store(slot, value)
                .map_err(|e| e.to_string())?;
        }
        let saved_async_completion = self.active_async_completion.replace(completion);
        let block_result =
            self.compile_async_segment_block(&segment.stmts, frame, completion, plan);
        self.active_async_completion = saved_async_completion;
        match block_result? {
            AsyncBlockExit::Returned => {
                self.resolve_async_completion(
                    completion,
                    self.async_completion_result(frame, plan)?,
                    ramp,
                )?;
                return Ok(());
            }
            AsyncBlockExit::Rejected => {
                if ramp {
                    self.builder
                        .build_return(Some(&completion))
                        .map_err(|e| e.to_string())?;
                } else {
                    self.builder.build_return(None).map_err(|e| e.to_string())?;
                }
                return Ok(());
            }
            AsyncBlockExit::Continue => {}
        }
        if let Some(awaited) = &segment.awaited {
            if let Some((guard_name, expected)) = &segment.await_guard {
                let function = self.current_function();
                let schedule = self
                    .context
                    .append_basic_block(function, "await_guard_true");
                let skip = self
                    .context
                    .append_basic_block(function, "await_guard_skip");
                let (guard_slot, guard_ty) = self
                    .variables
                    .get(guard_name)
                    .copied()
                    .ok_or_else(|| format!("missing async branch guard `{guard_name}`"))?;
                let mut condition = self
                    .builder
                    .build_load(guard_ty, guard_slot, guard_name)
                    .map_err(|e| e.to_string())?
                    .into_int_value();
                if !expected {
                    condition = self
                        .builder
                        .build_not(condition, "inverse_branch_guard")
                        .map_err(|e| e.to_string())?;
                }
                self.builder
                    .build_conditional_branch(condition, schedule, skip)
                    .map_err(|e| e.to_string())?;

                self.builder.position_at_end(skip);
                let state_slot = self.async_frame_field(frame, ASYNC_STATE_OFFSET, "state_slot")?;
                self.builder
                    .build_store(
                        state_slot,
                        self.context.i64_type().const_int(next_state as u64, false),
                    )
                    .map_err(|e| e.to_string())?;
                self.builder
                    .build_call(resume, &[frame.into(), frame.into()], "skip_guarded_await")
                    .map_err(|e| e.to_string())?;
                if ramp {
                    self.builder
                        .build_return(Some(&completion))
                        .map_err(|e| e.to_string())?;
                } else {
                    self.builder.build_return(None).map_err(|e| e.to_string())?;
                }
                self.builder.position_at_end(schedule);
            }
            let waiting = match awaited {
                HirExpr::Call(callee, args) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "fetch") => {
                    self.compile_single_arg_call("thaw_http_get_async", args, "async_fetch")?
                        .into_pointer_value()
                }
                _ => self.compile_expr(awaited)?.into_pointer_value(),
            };
            let waiting_slot =
                self.async_frame_field(frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
            self.builder
                .build_store(waiting_slot, waiting)
                .map_err(|e| e.to_string())?;
            let state_slot = self.async_frame_field(frame, ASYNC_STATE_OFFSET, "state_slot")?;
            let scheduled_state = segment.await_next.unwrap_or(next_state);
            self.builder
                .build_store(
                    state_slot,
                    self.context
                        .i64_type()
                        .const_int(scheduled_state as u64, false),
                )
                .map_err(|e| e.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_subscribe").unwrap(),
                    &[
                        waiting.into(),
                        resume.as_global_value().as_pointer_value().into(),
                        frame.into(),
                    ],
                    "subscribe_resume",
                )
                .map_err(|e| e.to_string())?;
            if ramp {
                self.builder
                    .build_return(Some(&completion))
                    .map_err(|e| e.to_string())?;
            } else {
                self.builder.build_return(None).map_err(|e| e.to_string())?;
            }
        } else {
            if plan.ret != HirType::Void {
                return Err("value-returning frame-split async function does not return a value on all paths".to_string());
            }
            self.resolve_async_completion(completion, frame, ramp)?;
        }
        Ok(())
    }

    fn resolve_async_completion(
        &self,
        completion: PointerValue<'ctx>,
        result: PointerValue<'ctx>,
        ramp: bool,
    ) -> Result<(), String> {
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_resolve").unwrap(),
                &[completion.into(), result.into()],
                "resolve_completion",
            )
            .map_err(|e| e.to_string())?;
        if ramp {
            self.builder
                .build_return(Some(&completion))
                .map_err(|e| e.to_string())?;
        } else {
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn bind_async_frame_locals(
        &mut self,
        frame: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
    ) -> Result<(), String> {
        for (index, (name, ty)) in plan.locals.iter().enumerate() {
            let slot = self.async_frame_field(
                frame,
                self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * index as u64,
                &format!("frame_{name}"),
            )?;
            self.variables
                .insert(name.clone(), (slot, self.basic_type(ty)?));
            self.variable_hir_types.insert(name.clone(), ty.clone());
        }
        Ok(())
    }

    fn compile_async_segment_block(
        &mut self,
        stmts: &[HirStmt],
        frame: PointerValue<'ctx>,
        completion: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
    ) -> Result<AsyncBlockExit, String> {
        for stmt in stmts {
            if let HirStmt::If(guard, then_body, else_body) = stmt {
                let guarded = match (then_body.as_slice(), else_body.as_slice()) {
                    ([only], []) => Some((only, true)),
                    ([], [only]) => Some((only, false)),
                    _ => None,
                };
                if let Some((HirStmt::Let(name, ty, expr), expected)) = guarded {
                    let function = self.current_function();
                    let initialize = self.context.append_basic_block(function, "guarded_let");
                    let continue_block = self.context.append_basic_block(function, "let_skipped");
                    let mut condition = self.compile_expr(guard)?.into_int_value();
                    if !expected {
                        condition = self
                            .builder
                            .build_not(condition, "inverse_let_guard")
                            .map_err(|e| e.to_string())?;
                    }
                    self.builder
                        .build_conditional_branch(condition, initialize, continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(initialize);
                    let value = self.compile_expr(expr)?;
                    let (slot, slot_ty) = self
                        .variables
                        .get(name)
                        .copied()
                        .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
                    if slot_ty != self.basic_type(ty)? {
                        return Err(format!("async frame local `{name}` changed type"));
                    }
                    self.builder
                        .build_store(slot, value)
                        .map_err(|e| e.to_string())?;
                    self.builder
                        .build_unconditional_branch(continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(continue_block);
                    continue;
                }
                if let Some((HirStmt::Return(value), expected)) = guarded {
                    let function = self.current_function();
                    let return_block = self.context.append_basic_block(function, "guarded_return");
                    let continue_block =
                        self.context.append_basic_block(function, "return_skipped");
                    let mut condition = self.compile_expr(guard)?.into_int_value();
                    if !expected {
                        condition = self
                            .builder
                            .build_not(condition, "inverse_return_guard")
                            .map_err(|e| e.to_string())?;
                    }
                    self.builder
                        .build_conditional_branch(condition, return_block, continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(return_block);
                    match value {
                        Some(HirExpr::ThrowValue(error, _)) => {
                            let error = self.compile_expr(error)?.into_pointer_value();
                            self.builder
                                .build_call(
                                    self.module.get_function("thaw_promise_reject").unwrap(),
                                    &[completion.into(), error.into()],
                                    "reject_throw_value",
                                )
                                .map_err(|error| error.to_string())?;
                            if function.get_type().get_return_type().is_some() {
                                self.builder
                                    .build_return(Some(&completion))
                                    .map_err(|error| error.to_string())?;
                            } else {
                                self.builder
                                    .build_return(None)
                                    .map_err(|error| error.to_string())?;
                            }
                            self.builder.position_at_end(continue_block);
                            continue;
                        }
                        Some(expr) if plan.ret != HirType::Void => {
                            let result = self.compile_expr(expr)?;
                            let result_slot =
                                self.async_frame_field(frame, ASYNC_RESULT_OFFSET, "async_result")?;
                            self.builder
                                .build_store(result_slot, result)
                                .map_err(|e| e.to_string())?;
                        }
                        None if plan.ret == HirType::Void => {}
                        Some(_) => {
                            return Err(
                                "void frame-split async function cannot return a value".to_string()
                            )
                        }
                        None => {
                            return Err(
                                "value-returning frame-split async function cannot use `return;`"
                                    .to_string(),
                            )
                        }
                    }
                    self.resolve_async_completion(
                        completion,
                        self.async_completion_result(frame, plan)?,
                        function.get_type().get_return_type().is_some(),
                    )?;
                    self.builder.position_at_end(continue_block);
                    continue;
                }
                if let Some((HirStmt::Throw(error), expected)) = guarded {
                    let enclosing_handler = match guard {
                        HirExpr::Var(name) => plan.guarded_rethrow_handlers.get(name).cloned(),
                        _ => None,
                    };
                    let function = self.current_function();
                    let reject = self.context.append_basic_block(function, "guarded_rethrow");
                    let continue_block =
                        self.context.append_basic_block(function, "rethrow_skipped");
                    let mut condition = self.compile_expr(guard)?.into_int_value();
                    if !expected {
                        condition = self
                            .builder
                            .build_not(condition, "inverse_throw_guard")
                            .map_err(|e| e.to_string())?;
                    }
                    self.builder
                        .build_conditional_branch(condition, reject, continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(reject);
                    let error = self.compile_expr(error)?.into_pointer_value();
                    if let Some(handler) = enclosing_handler {
                        let catch_index = plan
                            .locals
                            .iter()
                            .position(|(name, _)| name == &handler.catch_binding)
                            .ok_or_else(|| {
                                format!("missing async catch binding `{}`", handler.catch_binding)
                            })?;
                        let catch_slot = self.async_frame_field(
                            frame,
                            self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * catch_index as u64,
                            "outer_catch_binding",
                        )?;
                        self.builder
                            .build_store(catch_slot, error)
                            .map_err(|e| e.to_string())?;
                        for (guard_name, value) in
                            [(&handler.try_guard, false), (&handler.catch_guard, true)]
                                .into_iter()
                                .chain(handler.disable_guards.iter().map(|name| (name, false)))
                        {
                            let guard_index = plan
                                .locals
                                .iter()
                                .position(|(name, _)| name == guard_name)
                                .ok_or_else(|| format!("missing async guard `{guard_name}`"))?;
                            let guard_slot = self.async_frame_field(
                                frame,
                                self.async_locals_offset(plan)
                                    + ASYNC_SLOT_BYTES * guard_index as u64,
                                "outer_catch_guard",
                            )?;
                            self.builder
                                .build_store(
                                    guard_slot,
                                    self.context.bool_type().const_int(value as u64, false),
                                )
                                .map_err(|e| e.to_string())?;
                        }
                        self.builder
                            .build_unconditional_branch(continue_block)
                            .map_err(|e| e.to_string())?;
                    } else {
                        self.builder
                            .build_call(
                                self.module.get_function("thaw_promise_reject").unwrap(),
                                &[completion.into(), error.into()],
                                "rethrow_rejection",
                            )
                            .map_err(|e| e.to_string())?;
                        if function.get_type().get_return_type().is_some() {
                            self.builder
                                .build_return(Some(&completion))
                                .map_err(|e| e.to_string())?;
                        } else {
                            self.builder.build_return(None).map_err(|e| e.to_string())?;
                        }
                    }
                    self.builder.position_at_end(continue_block);
                    continue;
                }
            }
            if matches!(stmt, HirStmt::Return(None)) {
                if plan.ret != HirType::Void {
                    return Err(
                        "value-returning frame-split async function cannot use `return;`"
                            .to_string(),
                    );
                }
                return Ok(AsyncBlockExit::Returned);
            }
            if let HirStmt::Return(Some(HirExpr::ThrowValue(error, _))) = stmt {
                let error = self.compile_expr(error)?.into_pointer_value();
                self.builder
                    .build_call(
                        self.module.get_function("thaw_promise_reject").unwrap(),
                        &[completion.into(), error.into()],
                        "reject_throw_value",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(AsyncBlockExit::Rejected);
            }
            if let HirStmt::Return(Some(expr)) = stmt {
                if plan.ret == HirType::Void {
                    return Err("void frame-split async function cannot return a value".to_string());
                }
                let value = self.compile_expr(expr)?;
                let result_slot =
                    self.async_frame_field(frame, ASYNC_RESULT_OFFSET, "async_result")?;
                self.builder
                    .build_store(result_slot, value)
                    .map_err(|e| e.to_string())?;
                return Ok(AsyncBlockExit::Returned);
            }
            if let HirStmt::Throw(expr) = stmt {
                let error = self.compile_expr(expr)?.into_pointer_value();
                self.builder
                    .build_call(
                        self.module.get_function("thaw_promise_reject").unwrap(),
                        &[completion.into(), error.into()],
                        "reject_completion",
                    )
                    .map_err(|e| e.to_string())?;
                return Ok(AsyncBlockExit::Rejected);
            }
            if let HirStmt::Let(name, ty, expr) = stmt {
                let value = self.compile_expr(expr)?;
                let index = plan
                    .locals
                    .iter()
                    .position(|(local, _)| local == name)
                    .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
                let slot = self.async_frame_field(
                    frame,
                    self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * index as u64,
                    &format!("frame_{name}"),
                )?;
                self.builder
                    .build_store(slot, value)
                    .map_err(|e| e.to_string())?;
                self.variables
                    .insert(name.clone(), (slot, self.basic_type(ty)?));
            } else if self.compile_stmt(stmt)? {
                return Ok(AsyncBlockExit::Returned);
            }
        }
        Ok(AsyncBlockExit::Continue)
    }

    fn async_locals_offset(&self, plan: &FrameAsyncPlan) -> u64 {
        ASYNC_FRAME_BYTES
            + if plan.ret == HirType::Void {
                0
            } else {
                ASYNC_SLOT_BYTES
            }
    }

    fn async_frame_size(&self, plan: &FrameAsyncPlan) -> u64 {
        self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * plan.locals.len() as u64
    }

    fn async_completion_result(
        &self,
        frame: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
    ) -> Result<PointerValue<'ctx>, String> {
        if plan.ret == HirType::Void {
            Ok(frame)
        } else {
            self.async_frame_field(frame, ASYNC_RESULT_OFFSET, "async_result")
        }
    }

}
