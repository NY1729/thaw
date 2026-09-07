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
        ) || matches!(self.expr_hir_type(expr), Some(HirType::Promise(_)))
            || matches!(expr, HirExpr::Call(callee, _)
            if matches!(callee.as_ref(), HirExpr::Var(name)
                if name == "sleep" || name == "fetch" || name == "Promise.all"
                    || self.frame_async_functions.contains_key(name)
                    || self.promise_returning_functions.contains(name)))
    }

}
