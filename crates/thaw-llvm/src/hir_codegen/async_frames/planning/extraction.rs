impl<'ctx> HirCompiler<'ctx> {
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
                    HirExpr::PromiseNew(_, resolved, _, _)
                    | HirExpr::PromiseNewMixed(_, resolved, _) => resolved.clone(),
                    HirExpr::PromiseThen(_, _, _, output, _, _) => output.clone(),
                    HirExpr::PromiseThenBoth(_, _, _, _, output, _, _) => output.clone(),
                    HirExpr::PromiseFinally(_, _, input, _) => input.clone(),
                    HirExpr::PromiseAllArray(_, HirType::Void) => HirType::Void,
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
                    // An argument-binding wrapper `(lambda)(args)` that returns a Promise.
                    HirExpr::Call(callee, _)
                        if matches!(callee.as_ref(), HirExpr::Lambda(_, _, HirType::Promise(_), _)) =>
                    {
                        let HirExpr::Lambda(_, _, HirType::Promise(resolved), _) = callee.as_ref() else {
                            unreachable!()
                        };
                        resolved.as_ref().clone()
                    }
                    HirExpr::Call(callee, _) => {
                        let HirExpr::Var(name) = callee.as_ref() else {
                            return Err(format!("unsupported frame await call target {callee:?}"));
                        };
                        self.frame_async_functions
                            .get(name)
                            .cloned()
                            .ok_or_else(|| format!("missing async result type for `{name}`"))?
                    }
                    other => return Err(format!("unsupported frame await source {other:?}")),
                };
                *expr = HirExpr::Var(temporary.to_string());
                return Ok(Some((awaited, ty)));
            }
        }

        match expr {
            HirExpr::BinOp(_, left, right)
            | HirExpr::EvalThen(left, right)
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
            | HirExpr::JsonAsBool(value)
            | HirExpr::JsonAsNative(value, _)
            | HirExpr::NonArrowThis { payload: value, .. }
            | HirExpr::NonArrowReceiverPack { value, .. } => {
                self.extract_first_frame_await(value, temporary)
            }
            HirExpr::IndexAssign(a, b, c) => {
                for value in [a, b, c] {
                    if let Some(found) = self.extract_first_frame_await(value, temporary)? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
            HirExpr::JsonSet(object, key, value, _, _)
            | HirExpr::JsonIndexSet(object, key, value) => {
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
            | HirExpr::JsonKey(obj, value)
            | HirExpr::JsonDelete(obj, value) => {
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


    /// `console.log("k", 1 + await f())` lowers the left operand through an
    /// argument-binding wrapper `(lambda)(args)` around the awaiting body, which
    /// hides the await from the one-await-per-segment extraction. Turn each
    /// wrapper that (transitively) awaits into plain `let` statements so the
    /// extraction sees the await directly.
    fn hoist_await_binding_wrappers(&self, body: &[HirStmt]) -> Vec<HirStmt> {
        let frame_names = self
            .frame_async_functions
            .keys()
            .chain(self.promise_returning_functions.iter())
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let mut out = Vec::with_capacity(body.len());
        for stmt in body {
            // A statement-level call whose binding wrapper has a statement body
            // (`console.log(await a(), await b())`): splice it into plain statements.
            if let HirStmt::Expr(HirExpr::Call(callee, args)) = stmt {
                if let HirExpr::Lambda(_, params, _, block) = callee.as_ref() {
                    if let HirExpr::Block(inner) = block.as_ref() {
                        if params.len() == args.len()
                            && (args.iter().any(|arg| self.expr_awaits_frame_source(arg, &frame_names))
                                || inner.iter().any(|stmt| self.stmt_awaits_frame_source(stmt, &frame_names)))
                            && !inner.iter().any(|stmt| matches!(stmt, HirStmt::Return(_)))
                        {
                            let mut spliced = Vec::new();
                            for (param, arg) in params.iter().zip(args) {
                                spliced.push(HirStmt::Let(param.name.clone(), param.ty.clone(), arg.clone()));
                            }
                            spliced.extend(inner.iter().cloned());
                            out.extend(self.hoist_await_binding_wrappers(&spliced));
                            continue;
                        }
                    }
                }
            }
            let mut stmt = stmt.clone();
            let mut pre = Vec::new();
            match &mut stmt {
                HirStmt::Expr(e) | HirStmt::Return(Some(e)) | HirStmt::Let(_, _, e) | HirStmt::Throw(e) => {
                    self.unwrap_await_wrappers(e, &frame_names, &mut pre);
                }
                HirStmt::If(cond, then_body, else_body) => {
                    self.unwrap_await_wrappers(cond, &frame_names, &mut pre);
                    *then_body = self.hoist_await_binding_wrappers(then_body);
                    *else_body = self.hoist_await_binding_wrappers(else_body);
                }
                HirStmt::While(_, inner) | HirStmt::Finally(inner, _) => {
                    *inner = self.hoist_await_binding_wrappers(inner);
                }
                HirStmt::Try(inner, _, catch_body, _) => {
                    *inner = self.hoist_await_binding_wrappers(inner);
                    *catch_body = self.hoist_await_binding_wrappers(catch_body);
                }
                _ => {}
            }
            out.extend(pre);
            out.push(stmt);
        }
        out
    }

    fn unwrap_await_wrappers(
        &self,
        expr: &mut HirExpr,
        frame_names: &std::collections::HashSet<String>,
        pre: &mut Vec<HirStmt>,
    ) {
        while let HirExpr::Call(callee, args) = expr {
            let HirExpr::Lambda(_, params, _, body) = callee.as_ref() else { break };
            if params.len() != args.len()
                || matches!(body.as_ref(), HirExpr::Block(_))
                || !(self.expr_awaits_frame_source(body, frame_names)
                    || args.iter().any(|arg| self.expr_awaits_frame_source(arg, frame_names)))
            {
                break;
            }
            let params = params.clone();
            let body = body.as_ref().clone();
            for (param, mut arg) in params.into_iter().zip(std::mem::take(args)) {
                self.unwrap_await_wrappers(&mut arg, frame_names, pre);
                pre.push(HirStmt::Let(param.name, param.ty, arg));
            }
            *expr = body;
        }
        match expr {
            HirExpr::BinOp(_, a, b)
            | HirExpr::EvalThen(a, b)
            | HirExpr::Index(a, b)
            | HirExpr::TypedIndex(a, b, _)
            | HirExpr::DynamicPropAccess(a, b, _, _) => {
                self.unwrap_await_wrappers(a, frame_names, pre);
                self.unwrap_await_wrappers(b, frame_names, pre);
            }
            HirExpr::Call(callee, args) => {
                self.unwrap_await_wrappers(callee, frame_names, pre);
                for arg in args {
                    self.unwrap_await_wrappers(arg, frame_names, pre);
                }
            }
            HirExpr::FfiCall(_, args)
            | HirExpr::DynamicCall(_, args)
            | HirExpr::ArrayLit(args)
            | HirExpr::ArrayConcat(args, _) => {
                for arg in args {
                    self.unwrap_await_wrappers(arg, frame_names, pre);
                }
            }
            HirExpr::ObjectLit(fields) | HirExpr::JsonObjectLit(fields, _) => {
                for (_, value) in fields {
                    self.unwrap_await_wrappers(value, frame_names, pre);
                }
            }
            HirExpr::Assign(_, value) | HirExpr::PropAccess(value, _, _) | HirExpr::TypedClosure(_, value) => {
                self.unwrap_await_wrappers(value, frame_names, pre);
            }
            _ => {}
        }
    }
}
