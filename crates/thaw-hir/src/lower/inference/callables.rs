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
        // Class methods still have the legacy wrapper ABI in this task. The
        // generated symbol prefix is the class-method discriminator; a method
        // that does not read `this` still has a physical receiver parameter.
        let source_signature = self
            .signatures
            .get(symbol)
            .filter(|_| {
                !symbol.starts_with("__thaw_class_") && !symbol.ends_with("__thaw_unbound")
            })
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
                    if actual == expected {
                        arguments.push(argument);
                    } else if *actual == HirType::Json
                        && *expected == HirType::Optional(Box::new(HirType::Json))
                        && optional.contains(index)
                    {
                        arguments.push(HirExpr::Conditional(
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
                        ));
                    } else {
                        return Ok(None);
                    }
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
        if optional.count() > 16 {
            return Err("callable values support at most 16 omittable parameters".into());
        }

        let body = HirExpr::Block(self.build_callable_adapter_dispatch(
            symbol,
            fixed,
            optional,
            rest.as_deref(),
            &parameters,
            0,
            HirOptionalMask::default(),
        )?);
        let closure = HirExpr::Lambda(Vec::new(), parameters, ret.as_ref().clone(), Box::new(body));
        Ok(Some(HirExpr::TypedClosure(
            declared.clone(),
            Box::new(closure),
        )))
    }

    #[allow(clippy::too_many_arguments)]
    fn build_callable_adapter_dispatch(
        &self,
        symbol: &str,
        fixed: &[HirType],
        optional: &HirOptionalMask,
        rest: Option<&HirType>,
        parameters: &[HirParam],
        index: usize,
        omitted_mask: HirOptionalMask,
    ) -> Result<Vec<HirStmt>, String> {
        let Some(next) = optional
            .first_at_or_after(index)
            .filter(|next| *next < fixed.len())
        else {
            let target = if omitted_mask.is_empty() {
                symbol.to_string()
            } else {
                omitted_parameter_symbol(symbol, &omitted_mask)
            };
            let signature = self.signatures.get(&target).ok_or_else(|| {
                format!(
                    "function `{symbol}` cannot use this optional callable shape: missing omission adapter `{target}`"
                )
            })?;
            let mut arguments = Vec::with_capacity(signature.params.len());
            let mut source_index = 0usize;
            for (position, parameter) in parameters.iter().take(fixed.len()).enumerate() {
                if omitted_mask.contains(position) {
                    continue;
                }
                let expected = &signature.params[source_index];
                source_index += 1;
                let value = HirExpr::Var(parameter.name.clone());
                let value = match (&parameter.ty, expected) {
                    (HirType::Optional(payload), expected) if payload.as_ref() == expected => {
                        HirExpr::OptionalValue(Box::new(value), payload.as_ref().clone())
                    }
                    (HirType::Nullish(payload), expected) if payload.as_ref() == expected => {
                        HirExpr::NullishValue(Box::new(value), payload.as_ref().clone())
                    }
                    (HirType::Nullish(payload), HirType::Nullable(expected))
                        if payload.as_ref() == expected.as_ref() =>
                    {
                        HirExpr::Conditional(
                            Box::new(HirExpr::NullishIsNull(
                                Box::new(value.clone()),
                                payload.as_ref().clone(),
                            )),
                            Box::new(HirExpr::NullableNone(expected.as_ref().clone())),
                            Box::new(HirExpr::NullableSome(
                                Box::new(HirExpr::NullishValue(
                                    Box::new(value),
                                    payload.as_ref().clone(),
                                )),
                                expected.as_ref().clone(),
                            )),
                            HirType::Nullable(expected.clone()),
                        )
                    }
                    (actual, expected) if actual == expected => value,
                    (actual, expected) => {
                        return Err(format!(
                            "callable adapter for `{symbol}` cannot convert parameter {} from {actual:?} to {expected:?}",
                            position + 1
                        ));
                    }
                };
                arguments.push(value);
            }
            if rest.is_some() {
                arguments.push(HirExpr::Var("__thaw_callable_rest".into()));
            }
            let call = HirExpr::Call(Box::new(HirExpr::Var(target)), arguments);
            return Ok(if signature.ret == HirType::Void {
                vec![HirStmt::Expr(call), HirStmt::Return(None)]
            } else {
                vec![HirStmt::Return(Some(call))]
            });
        };

        let parameter = &parameters[next];
        let condition = match &parameter.ty {
            HirType::Optional(payload) => HirExpr::OptionalIsNone(
                Box::new(HirExpr::Var(parameter.name.clone())),
                payload.as_ref().clone(),
            ),
            HirType::Nullish(payload) => HirExpr::NullishIsUndefined(
                Box::new(HirExpr::Var(parameter.name.clone())),
                payload.as_ref().clone(),
            ),
            other => {
                return Err(format!(
                    "optional callable parameter {} has non-optional ABI type {other:?}",
                    next + 1
                ));
            }
        };
        let mut absent_mask = omitted_mask.clone();
        absent_mask.insert(next);
        let absent = self.build_callable_adapter_dispatch(
            symbol,
            fixed,
            optional,
            rest,
            parameters,
            next + 1,
            absent_mask,
        )?;
        let present = self.build_callable_adapter_dispatch(
            symbol,
            fixed,
            optional,
            rest,
            parameters,
            next + 1,
            omitted_mask,
        )?;
        Ok(vec![HirStmt::If(condition, absent, present)])
    }
}
