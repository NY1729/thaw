impl<'a> FnLowerer<'a> {
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
