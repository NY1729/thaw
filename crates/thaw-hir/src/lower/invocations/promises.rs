fn is_untyped_promise_reject(expr: &Expr) -> bool {
    let Expr::Call(call) = expr else {
        return false;
    };
    let Callee::Expr(callee) = &call.callee else {
        return false;
    };
    let Expr::Member(member) = callee.as_ref() else {
        return false;
    };
    matches!(
        (member.obj.as_ref(), &member.prop),
        (Expr::Ident(object), MemberProp::Ident(property))
            if object.sym == *"Promise"
                && property.sym == *"reject"
                && call.type_args.is_none()
    )
}

impl<'a> FnLowerer<'a> {
    fn lower_promise_member_call(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if property.sym == *"finally" {
                    let source = self.lower_expr(&member.obj)?;
                    let HirType::Promise(input) = self.infer_expr_type(&source)? else {
                        return Err("`.finally` requires a Promise receiver".into());
                    };
                    let [callback] = call.args.as_slice() else {
                        return Err("`.finally` expects exactly one callback".into());
                    };
                    let (source, callback, bindings) = if callback.spread.is_some() {
                        let source_name = format!("__thaw_promise_source_{}", self.next_binding);
                        self.next_binding += 1;
                        let source_type = HirType::Promise(input.clone());
                        self.scope
                            .insert(source_name.clone(), source_type.clone());
                        let (callbacks, mut bindings) =
                            self.lower_native_spread_values(&call.args, "Promise callback")?;
                        let [callback] = callbacks.as_slice() else {
                            return Err(
                                "`.finally` expects exactly one callback after spread expansion"
                                    .into(),
                            );
                        };
                        self.validate_promise_callback_value(callback, &[], None)?;
                        bindings.insert(0, (source_name.clone(), source_type, source));
                        (HirExpr::Var(source_name), callback.clone(), bindings)
                    } else {
                        (
                            source,
                            self.lower_promise_callback(&callback.expr, &[], None)?,
                            Vec::new(),
                        )
                    };
                    let HirType::Function(_, callback_return) = self.infer_expr_type(&callback)?
                    else {
                        unreachable!()
                    };
                    let result = HirExpr::PromiseFinally(
                        Box::new(source),
                        Box::new(callback),
                        *input,
                        *callback_return,
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"then" || property.sym == *"catch" {
                    let source = self.lower_expr(&member.obj)?;
                    let HirType::Promise(input) = self.infer_expr_type(&source)? else {
                        return Err(format!("`.{}` requires a Promise receiver", property.sym));
                    };
                    let [callback] = call.args.as_slice() else {
                        return Err(format!("`.{}` expects exactly one callback", property.sym));
                    };
                    let on_rejected = property.sym == *"catch";
                    let callback_input = if on_rejected {
                        HirType::Str
                    } else {
                        input.as_ref().clone()
                    };
                    let callback_params = if !on_rejected && callback_input == HirType::Void {
                        Vec::new()
                    } else {
                        vec![callback_input]
                    };
                    let (source, callback, bindings) = if callback.spread.is_some() {
                        let source_name = format!("__thaw_promise_source_{}", self.next_binding);
                        self.next_binding += 1;
                        let source_type = HirType::Promise(input.clone());
                        self.scope
                            .insert(source_name.clone(), source_type.clone());
                        let (callbacks, mut bindings) =
                            self.lower_native_spread_values(&call.args, "Promise callback")?;
                        let [callback] = callbacks.as_slice() else {
                            return Err(format!(
                                "`.{}` expects exactly one callback after spread expansion",
                                property.sym
                            ));
                        };
                        self.validate_promise_callback_value(callback, &callback_params, None)?;
                        bindings.insert(0, (source_name.clone(), source_type, source));
                        (HirExpr::Var(source_name), callback.clone(), bindings)
                    } else {
                        (
                            source,
                            self.lower_promise_callback(
                                &callback.expr,
                                &callback_params,
                                None,
                            )?,
                            Vec::new(),
                        )
                    };
                    let HirType::Function(_, callback_output) = self.infer_expr_type(&callback)?
                    else {
                        unreachable!()
                    };
                    let (output, flatten) = match callback_output.as_ref() {
                        HirType::Promise(inner) => (inner.as_ref().clone(), true),
                        output => (output.clone(), false),
                    };
                    if on_rejected && output != *input {
                        return Err(format!(
                            "`.catch` callback resolves to {output:?}, expected {:?}",
                            input
                        ));
                    }
                    let result = HirExpr::PromiseThen(
                        Box::new(source),
                        Box::new(callback),
                        input.as_ref().clone(),
                        output,
                        on_rejected,
                        flatten,
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
        unreachable!("promise member dispatch was checked before lowering")
    }

    /// `Promise.<method>([...])` where every element is a dynamic
    /// thenable (`Json` / `JsValue` -- a live QuickJS `Promise` handle,
    /// not a native `ThawPromise`). Returns `Some` with a call to
    /// QuickJS's own `Promise.<method>` over the array via
    /// `callDynamicMethod`, which settles the returned promise and hands
    /// back its resolved value as `Json`; `Expr::Await` already treats a
    /// `callDynamicMethod` result as resolved-at-the-boundary, so
    /// `await Promise.all([...])` is that `Json` directly. Returns `None`
    /// (leaving the caller's native-combinator path untouched) for an
    /// empty list, a hole, a spread, or any element that isn't dynamic.
    /// See `docs/design/dynamic-promise-combinators.md`.
    fn try_dynamic_promise_combinator(
        &mut self,
        method: &str,
        elems: &[Option<swc_ecma_ast::ExprOrSpread>],
    ) -> Result<Option<HirExpr>, String> {
        if elems.is_empty() {
            return Ok(None);
        }
        let mut lowered = Vec::with_capacity(elems.len());
        for element in elems {
            let Some(element) = element else {
                return Ok(None);
            };
            if element.spread.is_some() {
                return Ok(None);
            }
            lowered.push(self.lower_expr(&element.expr)?);
        }
        if !lowered.iter().all(|value| {
            matches!(
                self.infer_expr_type(value),
                Ok(HirType::Json | HirType::JsValue)
            )
        }) {
            return Ok(None);
        }
        let laundered = lowered
            .into_iter()
            .map(|value| self.coerce_to_declared(&HirType::Json, value))
            .collect::<Result<Vec<_>, String>>()?;
        let inner = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(laundered))?;
        let args = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![inner]))?;
        let promise_global = HirExpr::Call(
            Box::new(HirExpr::Var("getDynamicValue".to_string())),
            vec![HirExpr::Lit(HirLit::Str("Promise".to_string()))],
        );
        Ok(Some(HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicMethod".to_string())),
            vec![
                promise_global,
                HirExpr::Lit(HirLit::Str(method.to_string())),
                args,
            ],
        )))
    }

    fn lower_promise_static_call(
        &mut self,
        callee_name: &str,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        if callee_name == "Promise.all" {
            let [arg] = call.args.as_slice() else {
                return Err("`Promise.all` expects exactly one array argument".into());
            };
            if arg.spread.is_some() {
                let (arguments, bindings) =
                    self.lower_native_spread_values(&call.args, "Promise.all")?;
                let [value] = arguments.as_slice() else {
                    return Err("`Promise.all` expects exactly one array argument".into());
                };
                let result = self.lower_spread_promise_combinator(value.clone(), "all")?;
                return self.wrap_call_argument_bindings(result, &bindings);
            }
            let Expr::Array(array) = arg.expr.as_ref() else {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "all")?;
                return Ok(HirExpr::PromiseAllArray(Box::new(values), element));
            };
            if array
                .elems
                .iter()
                .flatten()
                .any(|element| element.spread.is_some())
            {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "all")?;
                return Ok(HirExpr::PromiseAllArray(Box::new(values), element));
            }
            if let Some(dynamic) = self.try_dynamic_promise_combinator("all", &array.elems)? {
                return Ok(dynamic);
            }
            let mut element_types = Vec::new();
            let promises = array
                .elems
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    let Some(element) = element else {
                        return Err(format!("Promise.all element {index} is missing"));
                    };
                    if element.spread.is_some() {
                        return Err("spread elements are not supported in `Promise.all`".into());
                    }
                    let value = self.lower_expr(&element.expr)?;
                    let resolved = match self.infer_expr_type(&value)? {
                        HirType::Promise(inner) => *inner,
                        returned
                            if matches!(&value, HirExpr::Call(callee, _)
                                if matches!(callee.as_ref(), HirExpr::Var(name)
                                    if self.signatures.get(name).is_some_and(|signature| signature.is_async))) => returned,
                        other => Err(format!(
                            "Promise.all element {index} must be a Promise, got {other:?}"
                        ))?,
                    };
                    if resolved == HirType::Void
                        && !is_untyped_promise_reject(&element.expr)
                    {
                        return Err(format!("Promise.all element {index} resolves to void"));
                    }
                    element_types.push(resolved);
                    Ok(value)
                })
                .collect::<Result<Vec<_>, String>>()?;
            let first = element_types.first().cloned().unwrap_or(HirType::F64);
            if element_types.iter().all(|element| element == &first) {
                return Ok(HirExpr::PromiseAll(promises, first));
            }
            return Ok(HirExpr::PromiseAllTuple(promises, element_types));
        }

        if callee_name == "Promise.allSettled" {
            let [arg] = call.args.as_slice() else {
                return Err("`Promise.allSettled` expects exactly one array argument".into());
            };
            if arg.spread.is_some() {
                let (arguments, bindings) =
                    self.lower_native_spread_values(&call.args, "Promise.allSettled")?;
                let [value] = arguments.as_slice() else {
                    return Err(
                        "`Promise.allSettled` expects exactly one array argument".into(),
                    );
                };
                let result =
                    self.lower_spread_promise_combinator(value.clone(), "allSettled")?;
                return self.wrap_call_argument_bindings(result, &bindings);
            }
            let Expr::Array(array) = arg.expr.as_ref() else {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "allSettled")?;
                return Ok(HirExpr::PromiseAllSettledArray(Box::new(values), element));
            };
            if array
                .elems
                .iter()
                .flatten()
                .any(|element| element.spread.is_some())
            {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "allSettled")?;
                return Ok(HirExpr::PromiseAllSettledArray(Box::new(values), element));
            }
            if let Some(dynamic) =
                self.try_dynamic_promise_combinator("allSettled", &array.elems)?
            {
                return Ok(dynamic);
            }
            let mut element_type = None;
            let promises = array
                .elems
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    let Some(element) = element else {
                        return Err(format!("Promise.allSettled element {index} is missing"));
                    };
                    if element.spread.is_some() {
                        return Err(
                            "spread elements are not supported in `Promise.allSettled`".into(),
                        );
                    }
                    let value = self.lower_expr(&element.expr)?;
                    let resolved = match self.infer_expr_type(&value)? {
                        HirType::Promise(inner) => *inner,
                        returned
                            if matches!(&value, HirExpr::Call(callee, _)
                                if matches!(callee.as_ref(), HirExpr::Var(name)
                                    if self.signatures.get(name).is_some_and(|signature| signature.is_async))) => returned,
                        other => Err(format!(
                            "Promise.allSettled element {index} must be a Promise, got {other:?}"
                        ))?,
                    };
                    if resolved == HirType::Void {
                        if is_untyped_promise_reject(&element.expr) {
                            return Ok(value);
                        }
                        return Err(format!(
                            "Promise.allSettled element {index} resolves to void"
                        ));
                    }
                    if let Some(expected) = &element_type {
                        if expected != &resolved {
                            return Err(format!(
                                "Promise.allSettled element {index} resolves to {resolved:?}, expected {expected:?}"
                            ));
                        }
                    } else {
                        element_type = Some(resolved);
                    }
                    Ok(value)
                })
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::PromiseAllSettled(
                promises,
                element_type.unwrap_or(HirType::F64),
            ));
        }

        if callee_name == "Promise.race" {
            let [arg] = call.args.as_slice() else {
                return Err("`Promise.race` expects exactly one array argument".into());
            };
            if arg.spread.is_some() {
                let (arguments, bindings) =
                    self.lower_native_spread_values(&call.args, "Promise.race")?;
                let [value] = arguments.as_slice() else {
                    return Err("`Promise.race` expects exactly one array argument".into());
                };
                let result = self.lower_spread_promise_combinator(value.clone(), "race")?;
                return self.wrap_call_argument_bindings(result, &bindings);
            }
            let Expr::Array(array) = arg.expr.as_ref() else {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "race")?;
                return Ok(HirExpr::PromiseRaceArray(Box::new(values), element));
            };
            if array
                .elems
                .iter()
                .flatten()
                .any(|element| element.spread.is_some())
            {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "race")?;
                return Ok(HirExpr::PromiseRaceArray(Box::new(values), element));
            }
            if array.elems.is_empty() {
                return Err("`Promise.race` requires at least one promise".into());
            }
            if let Some(dynamic) = self.try_dynamic_promise_combinator("race", &array.elems)? {
                return Ok(dynamic);
            }
            let mut element_type = None;
            let promises = array
                .elems
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    let Some(element) = element else {
                        return Err(format!("Promise.race element {index} is missing"));
                    };
                    if element.spread.is_some() {
                        return Err("spread elements are not supported in `Promise.race`".into());
                    }
                    let value = self.lower_expr(&element.expr)?;
                    let resolved = match self.infer_expr_type(&value)? {
                        HirType::Promise(inner) => *inner,
                        returned
                            if matches!(&value, HirExpr::Call(callee, _)
                                if matches!(callee.as_ref(), HirExpr::Var(name)
                                    if self.signatures.get(name).is_some_and(|signature| signature.is_async))) => returned,
                        other => Err(format!(
                            "Promise.race element {index} must be a Promise, got {other:?}"
                        ))?,
                    };
                    if resolved == HirType::Void {
                        if is_untyped_promise_reject(&element.expr) {
                            return Ok(value);
                        }
                        return Err(format!("Promise.race element {index} resolves to void"));
                    }
                    if let Some(expected) = &element_type {
                        if expected != &resolved {
                            return Err(format!(
                                "Promise.race element {index} resolves to {resolved:?}, expected {expected:?}"
                            ));
                        }
                    } else {
                        element_type = Some(resolved);
                    }
                    Ok(value)
                })
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::PromiseRace(
                promises,
                element_type.unwrap_or(HirType::F64),
            ));
        }

        if callee_name == "Promise.any" {
            let [arg] = call.args.as_slice() else {
                return Err("`Promise.any` expects exactly one array argument".into());
            };
            if arg.spread.is_some() {
                let (arguments, bindings) =
                    self.lower_native_spread_values(&call.args, "Promise.any")?;
                let [value] = arguments.as_slice() else {
                    return Err("`Promise.any` expects exactly one array argument".into());
                };
                let result = self.lower_spread_promise_combinator(value.clone(), "any")?;
                return self.wrap_call_argument_bindings(result, &bindings);
            }
            let Expr::Array(array) = arg.expr.as_ref() else {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "any")?;
                return Ok(HirExpr::PromiseAnyArray(Box::new(values), element));
            };
            if array
                .elems
                .iter()
                .flatten()
                .any(|element| element.spread.is_some())
            {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "any")?;
                return Ok(HirExpr::PromiseAnyArray(Box::new(values), element));
            }
            if array.elems.is_empty() {
                return Err("`Promise.any` requires at least one promise".into());
            }
            if let Some(dynamic) = self.try_dynamic_promise_combinator("any", &array.elems)? {
                return Ok(dynamic);
            }
            let mut element_type = None;
            let promises = array
                .elems
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    let Some(element) = element else {
                        return Err(format!("Promise.any element {index} is missing"));
                    };
                    if element.spread.is_some() {
                        return Err("spread elements are not supported in `Promise.any`".into());
                    }
                    let value = self.lower_expr(&element.expr)?;
                    let resolved = match self.infer_expr_type(&value)? {
                        HirType::Promise(inner) => *inner,
                        returned
                            if matches!(&value, HirExpr::Call(callee, _)
                                if matches!(callee.as_ref(), HirExpr::Var(name)
                                    if self.signatures.get(name).is_some_and(|signature| signature.is_async))) => returned,
                        other => Err(format!(
                            "Promise.any element {index} must be a Promise, got {other:?}"
                        ))?,
                    };
                    if resolved == HirType::Void {
                        if is_untyped_promise_reject(&element.expr) {
                            return Ok(value);
                        }
                        return Err(format!("Promise.any element {index} resolves to void"));
                    }
                    if let Some(expected) = &element_type {
                        if expected != &resolved {
                            return Err(format!(
                                "Promise.any element {index} resolves to {resolved:?}, expected {expected:?}"
                            ));
                        }
                    } else {
                        element_type = Some(resolved);
                    }
                    Ok(value)
                })
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::PromiseAny(
                promises,
                element_type.unwrap_or(HirType::F64),
            ));
        }
        unreachable!("promise static dispatch was checked before lowering")
    }
}
