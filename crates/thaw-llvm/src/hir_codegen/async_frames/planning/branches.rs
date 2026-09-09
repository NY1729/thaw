impl<'ctx> HirCompiler<'ctx> {
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
                loop_guards,
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

}
