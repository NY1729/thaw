impl<'ctx> HirCompiler<'ctx> {
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

}
