impl<'a> FnLowerer<'a> {
    pub(in crate::lower) fn adapt_named_function_to_required_function(
        &self,
        declared: &HirType,
        value: &HirExpr,
    ) -> Result<Option<HirExpr>, String> {
        let HirType::Function(params, ret) = declared else { return Ok(None) };
        let HirExpr::FunctionRef(symbol, source_params, source_ret) = value else { return Ok(None) };
        if source_ret != ret.as_ref() || source_params.len() != params.len() { return Ok(None) }
        let Some(signature) = self.signatures.get(symbol).filter(|signature| !signature.uses_this) else {
            return Ok(None);
        };
        if signature.params != *source_params || !signature.generic_type_params.is_empty() { return Ok(None) }
        let mut arguments = Vec::with_capacity(params.len());
        let mut adapter_params = Vec::with_capacity(params.len());
        for (index, (source, target)) in source_params.iter().zip(params).enumerate() {
            let name = format!("__thaw_required_callable_argument_{index}");
            adapter_params.push(HirParam { name: name.clone(), ty: target.clone() });
            let arg = HirExpr::Var(name);
            match source {
                HirType::Optional(inner) if inner.as_ref() == target =>
                    arguments.push(HirExpr::OptionalSome(Box::new(arg), target.clone())),
                HirType::Nullish(inner) if inner.as_ref() == target =>
                    arguments.push(HirExpr::NullishSome(Box::new(arg), target.clone())),
                _ if source == target => arguments.push(arg),
                _ => return Ok(None),
            }
        }
        let call = HirExpr::Call(Box::new(HirExpr::Var(symbol.clone())), arguments);
        let body = if **ret == HirType::Void {
            HirExpr::Block(vec![HirStmt::Expr(call), HirStmt::Return(None)])
        } else { HirExpr::Block(vec![HirStmt::Return(Some(call))]) };
        let adapter = HirExpr::Lambda(Vec::new(), adapter_params, ret.as_ref().clone(), Box::new(body));
        Ok(Some(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_closure_alias".into())),
            vec![HirExpr::TypedClosure(declared.clone(), Box::new(adapter)), value.clone()],
        )))
    }

    pub(in crate::lower) fn normalize_function_ref_value(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let HirExpr::FunctionRef(symbol, physical_params, ret) = &value else {
            return Ok(value);
        };
        let logical = self.logical_function_ref_type(symbol, physical_params, ret);
        let HirType::CallableFunction(fixed, _, rest, logical_ret) = &logical else {
            return Ok(value);
        };
        let mut abi = fixed.clone();
        if let Some(rest) = rest {
            abi.push(HirType::Array(rest.clone()));
        }
        if abi == *physical_params && logical_ret.as_ref() == ret {
            return Ok(HirExpr::TypedClosure(logical, Box::new(value)));
        }
        let adapted = self.coerce_to_declared(&logical, value.clone())?;
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_closure_alias".into())),
            vec![adapted, value],
        ))
    }

    fn adapt_named_function_to_callable(
        &self,
        declared: &HirType,
        value: &HirExpr,
    ) -> Result<Option<HirExpr>, String> {
        let HirType::CallableFunction(fixed, optional, rest, ret) = declared else {
            return Ok(None);
        };
        if optional.is_empty() {
            return Ok(None);
        }
        let HirExpr::FunctionRef(symbol, source_params, source_ret) = value else {
            return Ok(None);
        };
        if source_ret != ret.as_ref() {
            return Ok(None);
        }
        let expected_abi_count = fixed.len() + usize::from(rest.is_some());
        if source_params.len() != expected_abi_count {
            return Ok(None);
        }
        if let Some(rest) = rest {
            if source_params.last() != Some(&HirType::Array(rest.clone())) {
                return Ok(None);
            }
        }
        let mut parameters = fixed
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__thaw_callable_argument_{index}"),
                ty: ty.clone(),
            })
            .collect::<Vec<_>>();
        if let Some(rest) = rest {
            parameters.push(HirParam {
                name: "__thaw_callable_rest".into(),
                ty: HirType::Array(rest.clone()),
            });
        }
        // Adapt only a physically compatible, receiver-free canonical ABI.
        // Bound instance methods have already captured their receiver before
        // they reach this path; named `this` functions remain unsupported.
        let source_signature = self
            .signatures
            .get(symbol)
            .filter(|signature| {
                !signature.uses_this && signature.source_omittable == *optional
            });
        if let Some(signature) = source_signature {
            if signature.params == *source_params {
                let mut arguments = Vec::with_capacity(expected_abi_count);
                for (index, (actual, expected)) in
                    source_params.iter().zip(fixed.iter()).enumerate()
                {
                    let argument = HirExpr::Var(parameters[index].name.clone());
                    arguments.push(match (actual, expected) {
                        (actual, expected) if actual == expected => argument,
                        (
                            HirType::Json,
                            HirType::Optional(payload),
                        ) if *payload.as_ref() == HirType::Json && optional.contains(index) =>
                        {
                            HirExpr::Conditional(
                            Box::new(HirExpr::OptionalIsNone(
                                Box::new(argument.clone()),
                                HirType::Json,
                            )),
                            Box::new(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_undefined".into())),
                                Vec::new(),
                            )),
                            Box::new(HirExpr::OptionalValue(
                                Box::new(argument),
                                HirType::Json,
                            )),
                            HirType::Json,
                            )
                        }
                        (HirType::Optional(payload), expected)
                            if *payload.as_ref() == *expected && optional.contains(index) =>
                        {
                            HirExpr::OptionalValue(Box::new(argument), payload.as_ref().clone())
                        }
                        (HirType::Nullish(payload), expected)
                            if *payload.as_ref() == *expected && optional.contains(index) =>
                        {
                            HirExpr::NullishValue(Box::new(argument), payload.as_ref().clone())
                        }
                        _ => return Ok(None),
                    });
                }
                if rest.is_some() {
                    arguments.push(HirExpr::Var("__thaw_callable_rest".into()));
                }
                let call = HirExpr::Call(Box::new(HirExpr::Var(symbol.into())), arguments);
                let body = if **ret == HirType::Void {
                    HirExpr::Block(vec![HirStmt::Expr(call), HirStmt::Return(None)])
                } else {
                    HirExpr::Block(vec![HirStmt::Return(Some(call))])
                };
                let closure = HirExpr::Lambda(
                    Vec::new(),
                    parameters,
                    ret.as_ref().clone(),
                    Box::new(body),
                );
                return Ok(Some(HirExpr::TypedClosure(
                    declared.clone(),
                    Box::new(closure),
                )));
            }
        }
        Ok(None)
    }
}

impl<'a> FnLowerer<'a> {
    pub(in crate::lower) fn adapt_overloaded_callable_value(
        &mut self,
        declared: &HirType,
        value: HirExpr,
    ) -> Result<Option<HirExpr>, String> {
        let HirType::Function(expected_params, expected_ret) = declared else {
            return Ok(None);
        };
        let actual_type = self.infer_expr_type(&value)?;
        let (overloads, physical) = match actual_type {
            HirType::OverloadedFunction(overloads, physical) => (overloads, *physical),
            HirType::Function(_, _) => {
                let HirExpr::FunctionRef(symbol, params, ret) = &value else {
                    return Ok(None);
                };
                let Some(signature) = self.signatures.get(symbol)
                    .filter(|signature| {
                        !signature.public_overloads.is_empty()
                            && !signature.uses_this
                            && signature.generic_type_params.is_empty()
                    })
                else {
                    return Ok(None);
                };
                (
                    signature.public_overloads.clone(),
                    HirType::Function(params.clone(), Box::new(ret.clone())),
                )
            }
            _ => return Ok(None),
        };
        let (physical_params, physical_ret) = match &physical {
            HirType::Function(params, ret) => (params.clone(), ret.as_ref().clone()),
            HirType::CallableFunction(params, optional, None, ret) if optional.is_empty() => {
                (params.clone(), ret.as_ref().clone())
            }
            _ => return Err("overloaded callback has an unsupported physical ABI".into()),
        };

        let mut selected = None;
        for overload in &overloads {
            let callable = match overload {
                HirType::FunctionWithThis(_, _) => continue,
                other => other,
            };
            let (params, ret) = match callable {
                HirType::Function(params, ret) => (params, ret.as_ref()),
                HirType::CallableFunction(params, optional, None, ret) if optional.is_empty() => {
                    (params, ret.as_ref())
                }
                _ => continue,
            };
            if params.len() != expected_params.len() || params.len() != physical_params.len() {
                continue;
            }
            let params_match = params.iter().zip(expected_params).all(|(public, expected)| {
                let value = HirExpr::TypedClosure(
                    expected.clone(),
                    Box::new(HirExpr::Lit(HirLit::Undefined)),
                );
                self.expect_type(public, &value, "overloaded callback parameter").is_ok()
            });
            let return_matches = {
                let result = HirExpr::TypedClosure(
                    ret.clone(),
                    Box::new(HirExpr::Lit(HirLit::Undefined)),
                );
                self.expect_type(expected_ret, &result, "overloaded callback return").is_ok()
            };
            if params_match && return_matches {
                selected = Some((params.clone(), ret.clone()));
                break;
            }
        }
        let Some((public_params, public_ret)) = selected else {
            return Err("no public overload is compatible with the expected callback type".into());
        };

        let target_name = format!("__thaw_overloaded_callback_target_{}", self.next_binding);
        self.next_binding += 1;
        let target_previous = self.scope.insert(target_name.clone(), physical.clone());
        let mut params = Vec::with_capacity(expected_params.len());
        let mut args = Vec::with_capacity(expected_params.len());
        let mut parameter_previous = Vec::with_capacity(expected_params.len());
        let adapted = (|| -> Result<HirExpr, String> {
            for (index, (expected, public)) in expected_params.iter().zip(&public_params).enumerate() {
                let name = format!("__thaw_overloaded_callback_arg_{}_{}", self.next_binding, index);
                self.next_binding += 1;
                parameter_previous.push((name.clone(), self.scope.insert(name.clone(), expected.clone())));
                params.push(HirParam { name: name.clone(), ty: expected.clone() });
                let value = HirExpr::Var(name);
                let value = self.coerce_to_declared(public, value)?;
                let value = self.coerce_to_declared(&physical_params[index], value)?;
                args.push(value);
            }
            let call = HirExpr::Call(Box::new(HirExpr::Var(target_name.clone())), args.clone());
            let call = if physical_ret == public_ret {
                call
            } else if matches!(&physical_ret, HirType::Union(members) if members.contains(&public_ret)) {
                self.project_overload_call_result(call, &physical_ret, &public_ret)?
            } else {
                return Err(format!(
                    "public callback return {public_ret:?} cannot be projected from physical return {physical_ret:?}"
                ));
            };
            let call = self.coerce_to_declared(expected_ret, call)?;
            let lambda = HirExpr::Lambda(
                vec![HirParam { name: target_name.clone(), ty: physical.clone() }],
                params.clone(),
                expected_ret.as_ref().clone(),
                Box::new(HirExpr::Block(vec![HirStmt::Return(Some(call))])),
            );
            Ok(HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam { name: target_name.clone(), ty: physical.clone() }],
                    declared.clone(),
                    Box::new(lambda),
                )),
                vec![value.clone()],
            ))
        })();
        for (name, previous) in parameter_previous {
            match previous {
                Some(previous) => { self.scope.insert(name, previous); }
                None => { self.scope.remove(&name); }
            }
        }
        match target_previous {
            Some(previous) => { self.scope.insert(target_name, previous); }
            None => { self.scope.remove(&target_name); }
        }
        adapted.map(Some)
    }
}
