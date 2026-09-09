impl<'ctx> HirCompiler<'ctx> {
    #[allow(clippy::too_many_arguments)]
    fn append_nested_async_try(
        &self,
        segments: &mut Vec<AsyncSegment>,
        try_body: &[HirStmt],
        catch_name: &str,
        catch_body: &[HirStmt],
        activation_guard: &str,
        enclosing_handler: Option<AsyncRejectionHandler>,
        loop_guards: &[(String, String)],
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
                    loop_guards,
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
                loop_guards,
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
                    loop_guards,
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
                    loop_guards,
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

}
