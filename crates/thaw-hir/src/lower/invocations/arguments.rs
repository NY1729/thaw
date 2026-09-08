impl<'a> FnLowerer<'a> {
    fn lower_native_spread_values(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        label: &str,
    ) -> Result<(Vec<HirExpr>, Vec<LoweredBinding>), String> {
        let lowered = arguments
            .iter()
            .map(|argument| self.lower_expr(&argument.expr))
            .collect::<Result<Vec<_>, _>>()?;
        let preserve_order = arguments.iter().any(|argument| argument.spread.is_some())
            || lowered.iter().any(contains_await);
        let mut bindings = Vec::new();
        let mut values = Vec::new();
        for (argument, value) in arguments.iter().zip(lowered) {
            if !preserve_order {
                values.push(value);
                continue;
            }
            if argument.spread.is_none() {
                let ty = self.infer_expr_type(&value)?;
                let name = format!("__thaw_native_arg_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                bindings.push((name.clone(), ty, value));
                values.push(HirExpr::Var(name));
                continue;
            }
            if let HirExpr::ArrayLit(elements) = value {
                for element in elements {
                    if matches!(element, HirExpr::Lit(_)) {
                        values.push(element);
                        continue;
                    }
                    let ty = self.infer_expr_type(&element)?;
                    let name = format!("__thaw_native_arg_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    bindings.push((name.clone(), ty, element));
                    values.push(HirExpr::Var(name));
                }
                continue;
            }
            let source_type = self.infer_expr_type(&value)?;
            let HirType::Tuple(elements) = &source_type else {
                return Err(format!(
                    "{label} spread source must have statically known tuple length, got {source_type:?}"
                ));
            };
            let elements = elements.clone();
            let name = format!("__thaw_native_spread_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), source_type.clone());
            bindings.push((name.clone(), source_type, value));
            values.extend(elements.into_iter().enumerate().map(|(index, ty)| {
                HirExpr::TypedIndex(
                    Box::new(HirExpr::Var(name.clone())),
                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                    ty,
                )
            }));
        }
        Ok((values, bindings))
    }

    fn select_omitted_class_arguments(
        &mut self,
        symbol: &mut Symbol,
        signature: &mut FnSignature,
        mut arguments: Vec<HirExpr>,
        receiver_count: usize,
        label: &str,
    ) -> Result<Vec<HirExpr>, String> {
        let native_rest_values = signature.native_rest.clone().map(|element| {
            let fixed_argument_count = signature.params.len() - receiver_count - 1;
            let values = if arguments.len() > fixed_argument_count {
                arguments.split_off(fixed_argument_count)
            } else {
                Vec::new()
            };
            (element, values)
        });
        let logical_param_count =
            signature.params.len() - usize::from(signature.native_rest.is_some());
        if logical_param_count < usize::BITS as usize
            && arguments.len() + receiver_count <= logical_param_count
        {
            let mut omitted_mask = 0usize;
            for index in receiver_count..logical_param_count {
                let omitted = match arguments.get(index - receiver_count) {
                    None => true,
                    Some(value) => self.infer_expr_type(value)? == HirType::Undefined,
                };
                if omitted {
                    omitted_mask |= 1usize << index;
                }
            }
            let wrapper = omitted_parameter_symbol(symbol, omitted_mask);
            if omitted_mask != 0 {
                if let Some(wrapper_signature) = self.signatures.get(&wrapper).cloned() {
                    arguments = arguments
                        .into_iter()
                        .enumerate()
                        .filter_map(|(index, argument)| {
                            (omitted_mask & (1usize << (index + receiver_count)) == 0)
                                .then_some(argument)
                        })
                        .collect();
                    *symbol = wrapper;
                    *signature = wrapper_signature;
                }
            }
        }
        if signature.native_rest.is_none()
            && arguments.len() + receiver_count != signature.params.len()
        {
            let wrapper = default_arity_symbol(symbol, arguments.len() + receiver_count);
            if let Some(wrapper_signature) = self.signatures.get(&wrapper).cloned() {
                *symbol = wrapper;
                *signature = wrapper_signature;
            }
        }
        if let Some((element, values)) = native_rest_values {
            let packed = if element == HirType::Dynamic {
                let expected = signature
                    .params
                    .last()
                    .ok_or("tuple rest parameter is missing its ABI slot")?;
                self.coerce_to_declared(expected, HirExpr::ArrayLit(values))?
            } else {
                let values = values
                    .into_iter()
                    .map(|value| self.coerce_to_declared(&element, value))
                    .collect::<Result<Vec<_>, _>>()?;
                native_rest_array(values, &element)
            };
            arguments.push(packed);
        }
        let expected = &signature.params[receiver_count..];
        if arguments.len() != expected.len() {
            return Err(format!(
                "{label} expects {} argument(s), got {}",
                expected.len(),
                arguments.len()
            ));
        }
        arguments
            .into_iter()
            .zip(expected)
            .map(|(value, expected)| self.coerce_to_declared(expected, value))
            .collect()
    }

}
