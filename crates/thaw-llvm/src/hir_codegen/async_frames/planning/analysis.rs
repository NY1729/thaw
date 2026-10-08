impl<'ctx> HirCompiler<'ctx> {
    /// Names HIR declared as the caught-exception carrier (`let e: <carrier> = undefined`
    /// before a `try`). Such a name is the catch binding itself, so the planner must not
    /// register a second (string) local under it.
    fn collect_async_carrier_catch_names(
        stmts: &[HirStmt],
        names: &mut std::collections::HashSet<String>,
    ) {
        let carrier = thaw_hir::caught_exception_carrier_type();
        for stmt in stmts {
            match stmt {
                HirStmt::Let(name, ty, _) if *ty == carrier => {
                    names.insert(name.clone());
                }
                HirStmt::If(_, then_body, else_body) => {
                    Self::collect_async_carrier_catch_names(then_body, names);
                    Self::collect_async_carrier_catch_names(else_body, names);
                }
                HirStmt::While(_, body) | HirStmt::Finally(body, _) => {
                    Self::collect_async_carrier_catch_names(body, names);
                }
                HirStmt::Try(body, _, catch_body, _) => {
                    Self::collect_async_carrier_catch_names(body, names);
                    Self::collect_async_carrier_catch_names(catch_body, names);
                }
                _ => {}
            }
        }
    }

    fn collect_async_frame_locals(
        &self,
        stmt: &HirStmt,
        locals: &mut Vec<(String, HirType)>,
    ) -> Result<(), String> {
        match stmt {
            HirStmt::Let(name, ty, _) => {
                // Inlined finalizer copies (one per exit) re-declare the same
                // generated local; the copies never overlap, so one slot of
                // the same type serves them all.
                if let Some((_, existing)) = locals.iter().find(|(existing, _)| existing == name) {
                    if existing == ty {
                        return Ok(());
                    }
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
            HirStmt::Finally(body, _) => {
                for nested in body {
                    self.collect_async_frame_locals(nested, locals)?;
                }
            }
            HirStmt::Try(body, _, catch_body, _) => {
                for nested in body.iter().chain(catch_body) {
                    self.collect_async_frame_locals(nested, locals)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn flatten_async_finally_only_tries(&self, body: &[HirStmt]) -> Result<Vec<HirStmt>, String> {
        // A call anywhere in the try body can throw after an await resumes.
        // Keep the synthetic catch so finally still runs on that path.
        Ok(body.to_vec())
    }

    fn is_frame_await_source(&self, expr: &HirExpr) -> bool {
        matches!(
            expr,
            HirExpr::PromiseNew(_, _, _, _)
                | HirExpr::PromiseNewMixed(_, _, _)
                | HirExpr::PromiseThen(_, _, _, _, _, _)
                | HirExpr::PromiseThenBoth(_, _, _, _, _, _, _)
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
        ) || matches!(self.expr_hir_type(expr), Some(HirType::Promise(_)))
            // An argument-binding wrapper `(lambda)(args)` that returns a Promise.
            || matches!(expr, HirExpr::Call(callee, _)
                if matches!(callee.as_ref(), HirExpr::Lambda(_, _, HirType::Promise(_), body)
                    if matches!(body.as_ref(), HirExpr::PromiseNew(..))))
            || matches!(expr, HirExpr::Call(callee, _)
            if matches!(callee.as_ref(), HirExpr::Var(name)
                if name == "sleep" || name == "fetch" || name == "Promise.all"
                    || self.frame_async_functions.contains_key(name)
                    || self.promise_returning_functions.contains(name)))
    }

}
