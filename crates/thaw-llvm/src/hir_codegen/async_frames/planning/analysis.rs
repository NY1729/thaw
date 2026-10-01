impl<'ctx> HirCompiler<'ctx> {
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
        ) || matches!(self.expr_hir_type(expr), Some(HirType::Promise(_)))
            || matches!(expr, HirExpr::Call(callee, _)
            if matches!(callee.as_ref(), HirExpr::Var(name)
                if name == "sleep" || name == "fetch" || name == "Promise.all"
                    || self.frame_async_functions.contains_key(name)
                    || self.promise_returning_functions.contains(name)))
    }

}
