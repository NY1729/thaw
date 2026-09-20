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
    fn lower_promise_try(&mut self, call: &CallExpr) -> Result<HirExpr, String> {
        let Some((callback_arg, argument_exprs)) = call.args.split_first() else {
            return Err("`Promise.try` expects a callback".into());
        };
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            return Err("`Promise.try` does not yet support spread arguments".into());
        }
        let arguments = argument_exprs
            .iter()
            .map(|argument| self.lower_expr(&argument.expr))
            .collect::<Result<Vec<_>, _>>()?;
        let argument_types = arguments
            .iter()
            .map(|argument| self.infer_expr_type(argument))
            .collect::<Result<Vec<_>, _>>()?;
        let expected = match &call.type_args {
            Some(type_args) => {
                let [expected] = type_args.params.as_slice() else {
                    return Err("`Promise.try` expects at most one type argument".into());
                };
                Some(lower_ts_type(
                    expected,
                    self.interfaces,
                    self.generic_interfaces,
                )?)
            }
            None => None,
        };
        let callback = self.lower_promise_callback(
            &callback_arg.expr,
            &argument_types,
            None,
        )?;
        let HirType::Function(_, output) = self.infer_expr_type(&callback)? else {
            unreachable!("Promise.try callback was lowered as a function")
        };
        let (resolved, assimilates) = match output.as_ref() {
            HirType::Promise(inner) => (inner.as_ref().clone(), true),
            output => (output.clone(), false),
        };
        if expected.as_ref().is_some_and(|expected| expected != &resolved) {
            return Err(format!(
                "Promise.try callback resolves to {resolved:?}, expected {:?}",
                expected.unwrap()
            ));
        }
        let callback_name = format!("__thaw_promise_try_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let mut bindings = vec![(callback_name.clone(), callback_type, callback)];
        let mut call_arguments = Vec::with_capacity(arguments.len());
        let mut captures = vec![HirParam {
            name: callback_name.clone(),
            ty: self.scope[&callback_name].clone(),
        }];
        for (argument, ty) in arguments.into_iter().zip(argument_types) {
            let name = format!("__thaw_promise_try_arg_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            call_arguments.push(HirExpr::Var(name.clone()));
            captures.push(HirParam {
                name: name.clone(),
                ty: ty.clone(),
            });
            bindings.push((name, ty, argument));
        }
        let resolve_name = format!("__thaw_promise_try_resolve_{}", self.next_binding);
        self.next_binding += 1;
        let resolve_params = if resolved == HirType::Void && !assimilates {
            Vec::new()
        } else {
            vec![if assimilates {
                HirType::Promise(Box::new(resolved.clone()))
            } else {
                resolved.clone()
            }]
        };
        let resolve_type = HirType::Function(resolve_params, Box::new(HirType::Void));
        let result = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name)),
            call_arguments,
        );
        let body = if resolved == HirType::Void && !assimilates {
            HirExpr::Block(vec![
                HirStmt::Expr(result),
                HirStmt::Return(Some(HirExpr::Call(
                    Box::new(HirExpr::Var(resolve_name.clone())),
                    Vec::new(),
                ))),
            ])
        } else {
            HirExpr::Call(
                Box::new(HirExpr::Var(resolve_name.clone())),
                vec![result],
            )
        };
        let executor = HirExpr::Lambda(
            captures,
            vec![HirParam {
                name: resolve_name.clone(),
                ty: resolve_type,
            }],
            HirType::Void,
            Box::new(body),
        );
        self.wrap_call_argument_bindings(
            HirExpr::PromiseNew(Box::new(executor), resolved, assimilates),
            &bindings,
        )
    }

    fn lower_promise_with_resolvers(&mut self, call: &CallExpr) -> Result<HirExpr, String> {
        if !call.args.is_empty() {
            return Err("`Promise.withResolvers` expects no arguments".into());
        }
        let resolved = match &call.type_args {
            Some(type_args) => {
                let [resolved] = type_args.params.as_slice() else {
                    return Err("`Promise.withResolvers` expects one type argument".into());
                };
                lower_ts_type(resolved, self.interfaces, self.generic_interfaces)?
            }
            None => HirType::Json,
        };
        let resolve_params = if resolved == HirType::Void {
            Vec::new()
        } else {
            vec![resolved.clone()]
        };
        let resolve_type = HirType::Function(resolve_params, Box::new(HirType::Void));
        let reject_type = HirType::Function(vec![HirType::Str], Box::new(HirType::Void));
        let promise_type = HirType::Promise(Box::new(resolved.clone()));
        let result_type = HirType::Object(vec![
            ("promise".into(), promise_type.clone()),
            ("resolve".into(), resolve_type.clone()),
            ("reject".into(), reject_type.clone()),
        ]);
        let result_name = format!("__thaw_with_resolvers_{}", self.next_binding);
        self.next_binding += 1;
        let promise_name = format!("__thaw_with_resolvers_promise_{}", self.next_binding);
        self.next_binding += 1;
        let resolve_name = format!("__thaw_with_resolvers_resolve_{}", self.next_binding);
        self.next_binding += 1;
        let reject_name = format!("__thaw_with_resolvers_reject_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(result_name.clone(), result_type.clone());
        self.scope.insert(promise_name.clone(), promise_type.clone());

        let assign = |field: &str, value: &str| {
            HirStmt::Expr(HirExpr::PropAssign(
                Box::new(HirExpr::Var(result_name.clone())),
                result_type.clone(),
                field.into(),
                Box::new(HirExpr::Var(value.into())),
            ))
        };
        let executor = HirExpr::Lambda(
            vec![HirParam {
                name: result_name.clone(),
                ty: result_type.clone(),
            }],
            vec![
                HirParam {
                    name: resolve_name.clone(),
                    ty: resolve_type,
                },
                HirParam {
                    name: reject_name.clone(),
                    ty: reject_type,
                },
            ],
            HirType::Void,
            Box::new(HirExpr::Block(vec![
                assign("resolve", &resolve_name),
                assign("reject", &reject_name),
                HirStmt::Return(None),
            ])),
        );
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                result_name.clone(),
                result_type.clone(),
                HirExpr::ObjectAlloc(result_type.clone()),
            ),
            HirStmt::Let(
                promise_name.clone(),
                promise_type,
                HirExpr::PromiseNew(Box::new(executor), resolved, false),
            ),
            assign("promise", &promise_name),
            HirStmt::Return(Some(HirExpr::Var(result_name))),
        ]);
        Ok(HirExpr::Call(
            Box::new(HirExpr::Lambda(
                Vec::new(),
                Vec::new(),
                result_type,
                Box::new(body),
            )),
            Vec::new(),
        ))
    }

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
