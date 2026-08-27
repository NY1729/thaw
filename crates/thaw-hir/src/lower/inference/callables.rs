impl<'a> FnLowerer<'a> {
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
        if optional.count() > 16 {
            return Err("callable values support at most 16 omittable parameters".into());
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
        let body = HirExpr::Block(self.build_callable_adapter_dispatch(
            symbol,
            fixed,
            optional,
            rest.as_deref(),
            &parameters,
            0,
            0,
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
        omitted_mask: usize,
    ) -> Result<Vec<HirStmt>, String> {
        let Some(next) = optional
            .first_at_or_after(index)
            .filter(|next| *next < fixed.len())
        else {
            let target = if omitted_mask == 0 {
                symbol.to_string()
            } else {
                omitted_parameter_symbol(symbol, omitted_mask)
            };
            let signature = self.signatures.get(&target).ok_or_else(|| {
                format!(
                    "function `{symbol}` cannot use this optional callable shape: missing omission adapter `{target}`"
                )
            })?;
            let mut arguments = Vec::with_capacity(signature.params.len());
            let mut source_index = 0usize;
            for (position, parameter) in parameters.iter().take(fixed.len()).enumerate() {
                if omitted_mask & (1usize << position) != 0 {
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
            return Ok(vec![HirStmt::Return(Some(HirExpr::Call(
                Box::new(HirExpr::Var(target)),
                arguments,
            )))]);
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
        let absent = self.build_callable_adapter_dispatch(
            symbol,
            fixed,
            optional,
            rest,
            parameters,
            next + 1,
            omitted_mask | (1usize << next),
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
