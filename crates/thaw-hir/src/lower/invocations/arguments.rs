#[derive(Clone, Copy)]
struct NativeSpreadOptions {
    array_reads: bool,
    snapshot_expanded_values: bool,
    logical_argument_offsets: bool,
    /// Coerce an object/array literal to its expected slot type before it is bound.
    precoerce_literals: bool,
}

impl<'a> FnLowerer<'a> {
    fn lower_literal_call_spread(
        &mut self,
        argument: &swc_ecma_ast::ExprOrSpread,
        expected: &[HirType],
        rest: Option<&HirType>,
        argument_offset: usize,
    ) -> Result<Option<HirExpr>, String> {
        if argument.spread.is_none() { return Ok(None); }
        let Expr::Array(array) = argument.expr.as_ref() else { return Ok(None) };
        if array.elems.iter().flatten().any(|element| element.spread.is_some()) { return Ok(None); }
        // The temporary spread container is unobservable; preserve its elements and evaluation order.
        let values = array.elems.iter().enumerate().map(|(index, element)| match element {
            Some(element) => self.lower_expr_with_expected_type(
                &element.expr,
                expected.get(argument_offset + index).or(rest),
            ),
            None => Ok(HirExpr::Lit(HirLit::ArrayHole)),
        }).collect::<Result<Vec<_>, _>>()?;
        Ok(Some(HirExpr::ArrayLit(values)))
    }

    fn lower_storage_call_spread(&mut self, argument: &swc_ecma_ast::ExprOrSpread, value: HirExpr) -> Result<HirExpr, String> {
        if argument.spread.is_some() && self.infer_expr_type(&value)? == HirType::Json {
            if let Some(source) = self.source_value_hint(&argument.expr) {
                if let HirType::Tuple(elements) = self.infer_expr_type(&source)? {
                    return Ok(HirExpr::JsonAsNative(Box::new(value), HirType::Tuple(vec![HirType::Json; elements.len()])));
                }
            }
        }
        Ok(value)
    }

    fn lower_native_spread_values(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        label: &str,
    ) -> Result<(Vec<HirExpr>, Vec<LoweredBinding>), String> {
        self.lower_native_spread_values_with_expected(arguments, label, &[], None)
    }

    fn lower_native_spread_values_with_snapshots(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        label: &str,
    ) -> Result<(Vec<HirExpr>, Vec<LoweredBinding>), String> {
        self.lower_native_spread_values_impl(arguments, label, &[], None, NativeSpreadOptions { array_reads: false, snapshot_expanded_values: true, logical_argument_offsets: false, precoerce_literals: false })
    }

    fn lower_native_spread_array_values(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        label: &str,
    ) -> Result<(Vec<HirExpr>, Vec<LoweredBinding>), String> {
        self.lower_native_spread_values_impl(arguments, label, &[], None, NativeSpreadOptions { array_reads: true, snapshot_expanded_values: false, logical_argument_offsets: false, precoerce_literals: false })
    }

    /// `lower_native_spread_array_values` with the receiver's element type as context, so an
    /// object/tuple literal is built against it (`out.push({ text: s[i] })`).
    fn lower_native_spread_array_values_for(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        label: &str,
        element: Option<&HirType>,
    ) -> Result<(Vec<HirExpr>, Vec<LoweredBinding>), String> {
        let element = element.filter(|ty| matches!(ty, HirType::Object(_) | HirType::Tuple(_) | HirType::Array(_)));
        self.lower_native_spread_values_impl(arguments, label, &[], element, NativeSpreadOptions { array_reads: true, snapshot_expanded_values: false, logical_argument_offsets: false, precoerce_literals: true })
    }

    fn lower_error_constructor_arguments(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        label: &str,
    ) -> Result<(Vec<HirExpr>, Vec<LoweredBinding>), String> {
        self.lower_native_spread_values_impl(arguments, label, &[], None, NativeSpreadOptions { array_reads: false, snapshot_expanded_values: true, logical_argument_offsets: false, precoerce_literals: false })
    }

    fn lower_native_spread_values_with_expected(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        label: &str,
        expected: &[HirType],
        rest: Option<&HirType>,
    ) -> Result<(Vec<HirExpr>, Vec<LoweredBinding>), String> {
        self.lower_native_spread_values_impl(arguments, label, expected, rest, NativeSpreadOptions { array_reads: false, snapshot_expanded_values: false, logical_argument_offsets: false, precoerce_literals: false })
    }

    /// `lower_native_spread_values_with_expected` for stores into a typed slot (`Map.set`).
    fn lower_native_spread_values_for_store(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        label: &str,
        expected: &[HirType],
    ) -> Result<(Vec<HirExpr>, Vec<LoweredBinding>), String> {
        self.lower_native_spread_values_impl(arguments, label, expected, None, NativeSpreadOptions { array_reads: false, snapshot_expanded_values: false, logical_argument_offsets: false, precoerce_literals: true })
    }

    fn lower_native_spread_values_with_overload_expected(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        label: &str,
        expected: &[HirType],
        rest: Option<&HirType>,
    ) -> Result<(Vec<HirExpr>, Vec<LoweredBinding>), String> {
        self.lower_native_spread_values_impl(arguments, label, expected, rest, NativeSpreadOptions { array_reads: false, snapshot_expanded_values: false, logical_argument_offsets: true, precoerce_literals: false })
    }

    fn lower_native_spread_values_impl(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        label: &str,
        expected: &[HirType],
        rest: Option<&HirType>,
        options: NativeSpreadOptions,
    ) -> Result<(Vec<HirExpr>, Vec<LoweredBinding>), String> {
        let mut lowered = Vec::with_capacity(arguments.len());
        let mut argument_offset = 0;
        let mut source_argument_index = 0;
        for argument in arguments {
            if let Some(value) = self.lower_literal_call_spread(
                argument,
                expected,
                rest,
                if options.logical_argument_offsets { argument_offset } else { source_argument_index },
            )? {
                if let HirExpr::ArrayLit(elements) = &value {
                    argument_offset += elements.len();
                }
                source_argument_index += 1;
                lowered.push(value);
                continue;
            }
            let expected_type = if argument.spread.is_some() {
                None
            } else {
                expected.get(if options.logical_argument_offsets { argument_offset } else { source_argument_index }).or(rest)
            };
            // An ambient QuickJS export passed by name keeps its real JS identity.
            let ambient_value = self.ambient_js_export_argument(&argument.expr, argument.spread.is_some());
            let value = match ambient_value {
                Some(value) => value,
                None => self.lower_expr_with_expected_type(&argument.expr, expected_type)?,
            };
            // A literal stored into a typed slot is coerced while it is still a literal
            // (`Optional` field reads then unwrap like a declaration's; `[]` takes the slot's
            // element type instead of `number[]`).
            let value = match expected_type {
                Some(expected @ (HirType::Object(_) | HirType::Array(_)))
                    if options.precoerce_literals
                        && match expected {
                            HirType::Object(_) => matches!(value, HirExpr::ObjectLit(_)) || Self::is_wrapped_object_literal(&value),
                            _ => matches!(value, HirExpr::ArrayLit(_)),
                        } =>
                {
                    match (self.coerce_to_declared(expected, value.clone()), expected) {
                        // A bare `[]` always infers `number[]`; allocate the slot's element type.
                        (Ok(HirExpr::ArrayLit(items)), HirType::Array(element))
                            if items.is_empty() && **element != HirType::F64 =>
                        {
                            HirExpr::ArrayAlloc(Box::new(HirExpr::Lit(HirLit::F64(0.0))), element.as_ref().clone())
                        }
                        (Ok(coerced), _) => coerced,
                        (Err(_), _) => value,
                    }
                }
                _ => value,
            };
            let value = if argument.spread.is_some() {
                let value = self.project_native_base(value)?;
                self.bytes_to_f64_array(value)?
            } else {
                value
            };
            if argument.spread.is_some() {
                if let HirType::Tuple(elements) = self.infer_expr_type(&value)? {
                    argument_offset += elements.len();
                }
            } else {
                argument_offset += 1;
            }
            source_argument_index += 1;
            lowered.push(self.lower_storage_call_spread(argument, value)?);
        }
        // Every argument must be evaluated before wrappers can omit it or reuse it.
        let mut bindings = Vec::new();
        let mut values = Vec::new();
        for (argument, value) in arguments.iter().zip(lowered) {
            let value = if options.array_reads && argument.spread.is_none() {
                self.lower_array_index_operand(value)?
            } else {
                value
            };
            if argument.spread.is_none() {
                // A literal has no side effects or ordering, so wrappers that need a
                // static literal (Temporal calendars/units, `.flat(2)`) can still see it.
                if !options.snapshot_expanded_values && matches!(value, HirExpr::Lit(_)) {
                    values.push(value);
                    continue;
                }
                // `infer_expr_type` erases `Bytes` to `Array(F64)`; keep it so a bound
                // `Buffer` argument still reads as one (`Buffer.isBuffer`, hex/utf8 methods).
                let ty = match self.infer_expr_type_inner(&value) {
                    Ok(HirType::Bytes) if !options.snapshot_expanded_values => HirType::Bytes,
                    _ => self.infer_expr_type(&value)?,
                };
                let name = format!("__thaw_native_arg_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                if matches!(value, HirExpr::ObjectLit(_)) || Self::is_wrapped_object_literal(&value) {
                    self.fresh_object_bindings.insert(name.clone());
                }
                bindings.push((name.clone(), ty, value));
                values.push(HirExpr::Var(name));
                continue;
            }
            if let HirExpr::ArrayLit(elements) = value {
                for element in elements {
                    let element = if options.array_reads {
                        self.lower_array_index_operand(element)?
                    } else {
                        element
                    };
                    if !options.snapshot_expanded_values && matches!(element, HirExpr::Lit(_)) {
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
            if options.snapshot_expanded_values {
                for (index, ty) in elements.into_iter().enumerate() {
                    let element = HirExpr::TypedIndex(
                        Box::new(HirExpr::Var(name.clone())),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        ty.clone(),
                    );
                    let element_name = format!("__thaw_native_arg_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(element_name.clone(), ty.clone());
                    bindings.push((element_name.clone(), ty, element));
                    values.push(HirExpr::Var(element_name));
                }
            } else {
                values.extend(elements.into_iter().enumerate().map(|(index, ty)| {
                    HirExpr::TypedIndex(
                        Box::new(HirExpr::Var(name.clone())),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        ty,
                    )
                }));
            }
        }
        Ok((values, bindings))
    }

    fn pad_source_mask_arguments(
        &mut self,
        signature: &FnSignature,
        mut arguments: Vec<HirExpr>,
        receiver_count: usize,
        label: &str,
    ) -> Result<Vec<HirExpr>, String> {
        let logical_param_count = signature.params.len() - usize::from(signature.native_rest.is_some());
        let fixed_argument_count = logical_param_count.saturating_sub(receiver_count);
        let native_rest_values = signature.native_rest.clone().map(|element| {
            let values = if arguments.len() > fixed_argument_count {
                arguments.split_off(fixed_argument_count)
            } else {
                Vec::new()
            };
            (element, values)
        });
        if arguments.len() <= fixed_argument_count {
            for index in arguments.len()..fixed_argument_count {
                let parameter_index = index + receiver_count;
                if signature.source_omittable.contains(parameter_index) {
                    arguments.push(omitted_parameter_value(&signature.params[parameter_index])?);
                } else {
                    break;
                }
            }
        }
        for (index, argument) in arguments.iter_mut().enumerate() {
            let parameter_index = index + receiver_count;
            if signature.source_omittable.contains(parameter_index)
                && self.infer_expr_type(argument)? == HirType::Undefined
            {
                *argument = omitted_parameter_value(&signature.params[parameter_index])?;
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
            .map(|(value, expected)| self.coerce_primitive_array_argument(value, expected))
            .collect()
    }

}
