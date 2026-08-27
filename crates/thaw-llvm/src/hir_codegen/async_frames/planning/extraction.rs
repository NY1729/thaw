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

}
