impl<'ctx> HirCompiler<'ctx> {
    fn async_block_returns_on_all_paths(stmts: &[HirStmt]) -> bool {
        stmts.iter().any(|stmt| match stmt {
            HirStmt::Return(_) | HirStmt::Throw(_) => true,
            HirStmt::If(_, then_body, else_body) => {
                Self::async_block_returns_on_all_paths(then_body)
                    && Self::async_block_returns_on_all_paths(else_body)
            }
            HirStmt::Try(try_body, _, catch_body) => {
                Self::async_block_returns_on_all_paths(try_body)
                    && Self::async_block_returns_on_all_paths(catch_body)
            }
            _ => false,
        })
    }

    fn async_stmt_exits(stmt: &HirStmt) -> bool {
        match stmt {
            HirStmt::Return(_) | HirStmt::Throw(_) => true,
            HirStmt::If(_, then_body, else_body) => then_body
                .iter()
                .chain(else_body)
                .any(Self::async_stmt_exits),
            HirStmt::While(_, body) => body.iter().any(Self::async_stmt_exits),
            HirStmt::Try(try_body, _, catch_body) => try_body
                .iter()
                .chain(catch_body)
                .any(Self::async_stmt_exits),
            _ => false,
        }
    }

    fn frame_await_plan(&self, func: &HirFunction) -> Result<Option<FrameAsyncPlan>, String> {
        if !self.frame_async_functions.contains_key(&func.name) {
            return Ok(None);
        }
        let normalized_body = self.flatten_async_finally_only_tries(&func.body)?;
        let returns_on_all_paths = Self::async_block_returns_on_all_paths(&normalized_body);
        let frame_names = self
            .frame_async_functions
            .keys()
            .chain(self.promise_returning_functions.iter())
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let will_split = normalized_body
            .iter()
            .any(|stmt| Self::stmt_awaits_frame_source(stmt, &frame_names));
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
                    .chain(self.promise_returning_functions.iter())
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
                                        &[],
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
                                        &[],
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
                    .chain(self.promise_returning_functions.iter())
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
                let branches_await = then_body
                    .iter()
                    .chain(else_body)
                    .any(|nested| Self::stmt_awaits_frame_source(nested, &frame_names));
                let branches_exit = then_body
                    .iter()
                    .chain(else_body)
                    .any(Self::async_stmt_exits);
                if branches_await || (will_split && branches_exit) {
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
                    .chain(self.promise_returning_functions.iter())
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
        let captures = self
            .async_lambda_captures
            .get(&func.name)
            .map(|captures| {
                captures
                    .iter()
                    .map(|capture| (capture.name.clone(), capture.ty.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut locals = func
            .params
            .iter()
            .skip(captures.len())
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
            captures,
            locals,
            ret: func.ret.clone(),
            guarded_rethrow_handlers,
            returns_on_all_paths,
        }))
    }

}
