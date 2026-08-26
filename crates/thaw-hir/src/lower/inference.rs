impl<'a> FnLowerer<'a> {
    /// Coerces a value into its declared native layout.
    fn coerce_to_declared(&self, declared: &HirType, value: HirExpr) -> Result<HirExpr, String> {
        if *declared == HirType::Dynamic {
            return Ok(value);
        }
        if let Some(adapted) = self.adapt_named_function_to_callable(declared, &value)? {
            return Ok(adapted);
        }
        if let HirType::Dictionary(element) = declared {
            if self.infer_expr_type(&value)? == *declared {
                return Ok(value);
            }
            let HirExpr::ObjectLit(fields) = value else {
                return Err(format!(
                    "dictionary value must be an object literal with {element:?} values"
                ));
            };
            let fields = fields
                .into_iter()
                .map(|(name, value)| Ok((name, self.coerce_to_declared(element.as_ref(), value)?)))
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::JsonObjectLit(fields, element.as_ref().clone()));
        }
        if let HirType::Union(elements) = declared {
            let actual = self.infer_expr_type(&value)?;
            if &actual == declared {
                return Ok(value);
            }
            if let HirType::Union(source) = &actual {
                if equivalent_union_members(source, elements) {
                    let parameter = "__thaw_union_retag_value".to_string();
                    let mut statements = Vec::with_capacity(source.len());
                    for (source_index, member) in source.iter().enumerate() {
                        let target_index = elements
                            .iter()
                            .position(|target| target == member)
                            .expect("equivalent union contains every source member");
                        let converted = HirExpr::UnionInject(
                            Box::new(HirExpr::UnionValue(
                                Box::new(HirExpr::Var(parameter.clone())),
                                source_index,
                                source.clone(),
                            )),
                            target_index,
                            elements.clone(),
                        );
                        if source_index + 1 == source.len() {
                            statements.push(HirStmt::Return(Some(converted)));
                        } else {
                            statements.push(HirStmt::If(
                                HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::UnionTag(
                                        Box::new(HirExpr::Var(parameter.clone())),
                                        source.clone(),
                                    )),
                                    Box::new(HirExpr::Lit(HirLit::F64(source_index as f64))),
                                ),
                                vec![HirStmt::Return(Some(converted))],
                                Vec::new(),
                            ));
                        }
                    }
                    let adapter = HirExpr::Lambda(
                        Vec::new(),
                        vec![HirParam {
                            name: parameter,
                            ty: actual,
                        }],
                        declared.clone(),
                        Box::new(HirExpr::Block(statements)),
                    );
                    return Ok(HirExpr::Call(Box::new(adapter), vec![value]));
                }
            }
            if let Some(index) = elements.iter().position(|element| element == &actual) {
                return Ok(HirExpr::UnionInject(
                    Box::new(value),
                    index,
                    elements.clone(),
                ));
            }
            if matches!(value, HirExpr::ObjectLit(_)) {
                for (index, member) in elements.iter().enumerate() {
                    if !matches!(member, HirType::Object(_)) {
                        continue;
                    }
                    if let Ok(adapted) = self.coerce_to_declared(member, value.clone()) {
                        return Ok(HirExpr::UnionInject(
                            Box::new(adapted),
                            index,
                            elements.clone(),
                        ));
                    }
                }
            }
            return Err(format!(
                "value has type {actual:?}, which is not a member of {declared:?}"
            ));
        }
        if let HirType::Optional(payload) = declared {
            return match self.infer_expr_type(&value)? {
                HirType::Undefined => Ok(HirExpr::OptionalNone(payload.as_ref().clone())),
                actual if actual == *declared => Ok(value),
                _ => {
                    let value = self.coerce_to_declared(payload.as_ref(), value)?;
                    Ok(HirExpr::OptionalSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let HirType::Nullable(payload) = declared {
            return match self.infer_expr_type(&value)? {
                HirType::Null => Ok(HirExpr::NullableNone(payload.as_ref().clone())),
                actual if actual == *declared => Ok(value),
                _ => {
                    let value = self.coerce_to_declared(payload.as_ref(), value)?;
                    Ok(HirExpr::NullableSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let HirType::Nullish(payload) = declared {
            return match self.infer_expr_type(&value)? {
                HirType::Null => Ok(HirExpr::NullishNull(payload.as_ref().clone())),
                HirType::Undefined => Ok(HirExpr::NullishUndefined(payload.as_ref().clone())),
                actual if actual == *declared => Ok(value),
                _ => {
                    let value = self.coerce_to_declared(payload.as_ref(), value)?;
                    Ok(HirExpr::NullishSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let (HirType::Array(element), HirExpr::ArrayLit(values)) = (declared, &value) {
            let values = values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    self.coerce_to_declared(element, value.clone())
                        .map_err(|error| format!("array element {index}: {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(HirExpr::ArrayLit(values));
        }
        if let (HirType::Tuple(expected), HirExpr::ArrayLit(values)) = (declared, &value) {
            if expected.len() != values.len() {
                return Err(format!(
                    "tuple literal has {} element(s), expected {}",
                    values.len(),
                    expected.len()
                ));
            }
            let values = expected
                .iter()
                .zip(values)
                .enumerate()
                .map(|(index, (expected, value))| {
                    self.coerce_to_declared(expected, value.clone())
                        .map_err(|error| format!("tuple element {index}: {error}"))
                })
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::ArrayLit(values));
        }
        let (HirType::Object(declared_fields), HirExpr::ObjectLit(lit_fields)) = (declared, &value)
        else {
            self.expect_type(declared, &value, "value")?;
            return Ok(value);
        };

        if lit_fields.len() > declared_fields.len()
            || lit_fields
                .iter()
                .any(|(name, _)| !declared_fields.iter().any(|(declared, _)| declared == name))
        {
            return Err(format!(
                "object literal has {} field(s), expected {} for this type",
                lit_fields.len(),
                declared_fields.len()
            ));
        }

        let reordered = declared_fields
            .iter()
            .map(|(name, expected_ty)| {
                let Some((_, field_value)) = lit_fields.iter().find(|(n, _)| n == name) else {
                    return Ok((name.clone(), omitted_parameter_value(expected_ty)?));
                };
                let field_value = self
                    .coerce_to_declared(expected_ty, field_value.clone())
                    .map_err(|error| format!("field `{name}`: {error}"))?;
                Ok((name.clone(), field_value))
            })
            .collect::<Result<Vec<_>, String>>()?;

        Ok(HirExpr::ObjectLit(reordered))
    }

    fn lower_union_property_read(
        &self,
        object: HirExpr,
        elements: &[HirType],
        property: &str,
    ) -> Result<HirExpr, String> {
        let mut field_types = Vec::with_capacity(elements.len());
        for element in elements {
            let HirType::Object(fields) = element else {
                return Err(format!(
                    "cannot access `.{property}` because union member {element:?} is not an object"
                ));
            };
            let field = fields
                .iter()
                .find(|(name, _)| name == property)
                .map(|(_, ty)| ty.clone())
                .ok_or_else(|| {
                    format!("cannot access `.{property}` because a union member has no such field")
                })?;
            if !field_types.contains(&field) {
                field_types.push(field);
            }
        }
        let result_type = match field_types.as_slice() {
            [] => return Err("cannot read a property from an empty union".into()),
            [field] => field.clone(),
            fields => {
                let mut members = Vec::new();
                for field in fields {
                    Self::flatten_property_union_members(field, &mut members)?;
                }
                match members.as_slice() {
                    [member] => member.clone(),
                    _ => HirType::Union(members),
                }
            }
        };
        let source_type = HirType::Union(elements.to_vec());
        let parameter = "__thaw_union_property_value".to_string();
        let mut statements = Vec::with_capacity(elements.len());
        for (index, element) in elements.iter().enumerate() {
            let field = HirExpr::PropAccess(
                Box::new(HirExpr::UnionValue(
                    Box::new(HirExpr::Var(parameter.clone())),
                    index,
                    elements.to_vec(),
                )),
                element.clone(),
                property.to_string(),
            );
            let HirType::Object(fields) = element else {
                unreachable!("union property was validated above")
            };
            let field_type = fields
                .iter()
                .find(|(name, _)| name == property)
                .map(|(_, ty)| ty)
                .expect("union property was validated above");
            let returns = if field_types.len() == 1 {
                vec![HirStmt::Return(Some(field))]
            } else {
                self.lower_flattened_property_return(field, field_type, &result_type)?
            };
            if index + 1 == elements.len() {
                statements.extend(returns);
            } else {
                statements.push(HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(
                            Box::new(HirExpr::Var(parameter.clone())),
                            elements.to_vec(),
                        )),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                    ),
                    returns,
                    Vec::new(),
                ));
            }
        }
        Ok(HirExpr::Call(
            Box::new(HirExpr::Lambda(
                Vec::new(),
                vec![HirParam {
                    name: parameter,
                    ty: source_type,
                }],
                result_type,
                Box::new(HirExpr::Block(statements)),
            )),
            vec![object],
        ))
    }

    fn flatten_property_union_members(
        field: &HirType,
        members: &mut Vec<HirType>,
    ) -> Result<(), String> {
        match field {
            HirType::Optional(payload) => {
                Self::flatten_property_union_members(payload, members)?;
                Self::flatten_property_union_members(&HirType::Undefined, members)
            }
            HirType::Nullable(payload) => {
                Self::flatten_property_union_members(payload, members)?;
                Self::flatten_property_union_members(&HirType::Null, members)
            }
            HirType::Nullish(payload) => {
                Self::flatten_property_union_members(payload, members)?;
                Self::flatten_property_union_members(&HirType::Null, members)?;
                Self::flatten_property_union_members(&HirType::Undefined, members)
            }
            HirType::Union(elements) => {
                for element in elements {
                    Self::flatten_property_union_members(element, members)?;
                }
                Ok(())
            }
            field
                if matches!(
                    field,
                    HirType::F64
                        | HirType::I64
                        | HirType::Bool
                        | HirType::Str
                        | HirType::Json
                        | HirType::JsValue
                        | HirType::Array(_)
                        | HirType::Tuple(_)
                        | HirType::Object(_)
                        | HirType::Function(_, _)
                        | HirType::Null
                        | HirType::Undefined
                ) =>
            {
                if !members.contains(field) {
                    members.push(field.clone());
                }
                Ok(())
            }
            other => Err(format!(
                "union object property has unsupported tagged payload {other:?}"
            )),
        }
    }

    fn lower_flattened_property_return(
        &self,
        value: HirExpr,
        ty: &HirType,
        result: &HirType,
    ) -> Result<Vec<HirStmt>, String> {
        let absent_return = |value: HirExpr| -> Result<Vec<HirStmt>, String> {
            Ok(vec![HirStmt::Return(Some(
                self.coerce_to_declared(result, value)?,
            ))])
        };
        match ty {
            HirType::Optional(payload) => {
                let mut statements = vec![HirStmt::If(
                    HirExpr::OptionalIsNone(Box::new(value.clone()), payload.as_ref().clone()),
                    absent_return(HirExpr::Lit(HirLit::Undefined))?,
                    Vec::new(),
                )];
                statements.extend(self.lower_flattened_property_return(
                    HirExpr::OptionalValue(Box::new(value), payload.as_ref().clone()),
                    payload,
                    result,
                )?);
                Ok(statements)
            }
            HirType::Nullable(payload) => {
                let mut statements = vec![HirStmt::If(
                    HirExpr::NullableIsNone(Box::new(value.clone()), payload.as_ref().clone()),
                    absent_return(HirExpr::Lit(HirLit::Null))?,
                    Vec::new(),
                )];
                statements.extend(self.lower_flattened_property_return(
                    HirExpr::NullableValue(Box::new(value), payload.as_ref().clone()),
                    payload,
                    result,
                )?);
                Ok(statements)
            }
            HirType::Nullish(payload) => {
                let mut statements = vec![
                    HirStmt::If(
                        HirExpr::NullishIsNull(Box::new(value.clone()), payload.as_ref().clone()),
                        absent_return(HirExpr::Lit(HirLit::Null))?,
                        Vec::new(),
                    ),
                    HirStmt::If(
                        HirExpr::NullishIsUndefined(
                            Box::new(value.clone()),
                            payload.as_ref().clone(),
                        ),
                        absent_return(HirExpr::Lit(HirLit::Undefined))?,
                        Vec::new(),
                    ),
                ];
                statements.extend(self.lower_flattened_property_return(
                    HirExpr::NullishValue(Box::new(value), payload.as_ref().clone()),
                    payload,
                    result,
                )?);
                Ok(statements)
            }
            HirType::Union(elements) => {
                let mut statements = Vec::new();
                for (index, member) in elements.iter().enumerate() {
                    let returns = self.lower_flattened_property_return(
                        HirExpr::UnionValue(Box::new(value.clone()), index, elements.clone()),
                        member,
                        result,
                    )?;
                    if index + 1 == elements.len() {
                        statements.extend(returns);
                    } else {
                        statements.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(
                                    Box::new(value.clone()),
                                    elements.clone(),
                                )),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            returns,
                            Vec::new(),
                        ));
                    }
                }
                Ok(statements)
            }
            _ => absent_return(value),
        }
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

    fn expect_type(
        &self,
        expected: &HirType,
        value: &HirExpr,
        context: &str,
    ) -> Result<(), String> {
        let actual = self.infer_expr_type(value)?;
        let callable_compatible = match (expected, &actual) {
            (
                HirType::CallableFunction(fixed, _, rest, expected_ret),
                HirType::Function(params, ret),
            ) => {
                let mut abi = fixed.clone();
                if let Some(rest) = rest {
                    abi.push(HirType::Array(rest.clone()));
                }
                abi == *params && expected_ret == ret
            }
            _ => false,
        };
        if actual == HirType::Dynamic
            || *expected == HirType::Dynamic
            || actual == *expected
            || callable_compatible
        {
            Ok(())
        } else {
            Err(format!(
                "{context} has type {actual:?}, expected {expected:?}"
            ))
        }
    }

    /// Infers the concrete native type of an expression. This is also the
    /// shared checker for assignments, returns, operators, indexes and call
    /// arguments, keeping unresolved/dynamic layouts out of LLVM lowering.
    fn infer_expr_type(&self, expr: &HirExpr) -> Result<HirType, String> {
        match expr {
            HirExpr::Lit(HirLit::F64(_)) => Ok(HirType::F64),
            HirExpr::Lit(HirLit::Str(_)) => Ok(HirType::Str),
            HirExpr::Lit(HirLit::Bool(_)) => Ok(HirType::Bool),
            HirExpr::Lit(HirLit::Undefined) => Ok(HirType::Undefined),
            HirExpr::Lit(HirLit::Null) => Ok(HirType::Null),
            HirExpr::Var(name) => self
                .scope
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown variable `{name}`")),
            HirExpr::FunctionRef(_, params, ret) => {
                Ok(HirType::Function(params.clone(), Box::new(ret.clone())))
            }
            HirExpr::MethodRef(_, _, params, ret, _) => {
                Ok(HirType::Function(params.clone(), Box::new(ret.clone())))
            }
            HirExpr::OptionalSome(value, payload) => {
                self.expect_type(payload, value, "optional payload")?;
                Ok(HirType::Optional(Box::new(payload.clone())))
            }
            HirExpr::OptionalNone(payload) => Ok(HirType::Optional(Box::new(payload.clone()))),
            HirExpr::OptionalIsNone(value, payload) => {
                self.expect_type(
                    &HirType::Optional(Box::new(payload.clone())),
                    value,
                    "optional test",
                )?;
                Ok(HirType::Bool)
            }
            HirExpr::OptionalValue(value, payload) => {
                self.expect_type(
                    &HirType::Optional(Box::new(payload.clone())),
                    value,
                    "optional value extraction",
                )?;
                Ok(payload.clone())
            }
            HirExpr::NullableSome(value, payload) => {
                self.expect_type(payload, value, "nullable payload")?;
                Ok(HirType::Nullable(Box::new(payload.clone())))
            }
            HirExpr::NullableNone(payload) => Ok(HirType::Nullable(Box::new(payload.clone()))),
            HirExpr::NullableIsNone(value, payload) => {
                self.expect_type(
                    &HirType::Nullable(Box::new(payload.clone())),
                    value,
                    "nullable null check",
                )?;
                Ok(HirType::Bool)
            }
            HirExpr::NullableValue(value, payload) => {
                self.expect_type(
                    &HirType::Nullable(Box::new(payload.clone())),
                    value,
                    "nullable payload extraction",
                )?;
                Ok(payload.clone())
            }
            HirExpr::NullishSome(value, payload) => {
                self.expect_type(payload, value, "nullish payload")?;
                Ok(HirType::Nullish(Box::new(payload.clone())))
            }
            HirExpr::NullishNull(payload) | HirExpr::NullishUndefined(payload) => {
                Ok(HirType::Nullish(Box::new(payload.clone())))
            }
            HirExpr::NullishIsNull(value, payload)
            | HirExpr::NullishIsUndefined(value, payload)
            | HirExpr::NullishIsNone(value, payload) => {
                self.expect_type(
                    &HirType::Nullish(Box::new(payload.clone())),
                    value,
                    "nullish tag check",
                )?;
                Ok(HirType::Bool)
            }
            HirExpr::NullishValue(value, payload) => {
                self.expect_type(
                    &HirType::Nullish(Box::new(payload.clone())),
                    value,
                    "nullish payload extraction",
                )?;
                Ok(payload.clone())
            }
            HirExpr::UnionInject(value, index, elements) => {
                let member = elements
                    .get(*index)
                    .ok_or_else(|| format!("union member index {index} is out of bounds"))?;
                self.expect_type(member, value, "union payload")?;
                Ok(HirType::Union(elements.clone()))
            }
            HirExpr::UnionTag(value, elements) => {
                self.expect_type(&HirType::Union(elements.clone()), value, "union tag access")?;
                Ok(HirType::F64)
            }
            HirExpr::UnionValue(value, index, elements) => {
                self.expect_type(
                    &HirType::Union(elements.clone()),
                    value,
                    "union value extraction",
                )?;
                elements
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| format!("union member index {index} is out of bounds"))
            }
            HirExpr::UnionMemberIsEqual(union, member, index, elements) => {
                self.expect_type(
                    &HirType::Union(elements.clone()),
                    union,
                    "union equality receiver",
                )?;
                let expected = elements
                    .get(*index)
                    .ok_or_else(|| format!("union member index {index} is out of bounds"))?;
                self.expect_type(expected, member, "union equality member")?;
                Ok(HirType::Bool)
            }
            HirExpr::UnionIsEqual(left, right, elements) => {
                let union = HirType::Union(elements.clone());
                self.expect_type(&union, left, "union equality left operand")?;
                self.expect_type(&union, right, "union equality right operand")?;
                Ok(HirType::Bool)
            }
            HirExpr::ArrayAlloc(length, element) => {
                self.expect_type(&HirType::F64, length, "array allocation length")?;
                Ok(HirType::Array(Box::new(element.clone())))
            }
            HirExpr::ArraySetLen(array, length, element) => {
                self.expect_type(
                    &HirType::Array(Box::new(element.clone())),
                    array,
                    "array length update receiver",
                )?;
                self.expect_type(&HirType::F64, length, "array length update")?;
                Ok(HirType::Array(Box::new(element.clone())))
            }
            HirExpr::Assign(name, value) => {
                let expected = self
                    .scope
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("unknown variable `{name}`"))?;
                self.expect_type(&expected, value, &format!("assignment to `{name}`"))?;
                Ok(expected)
            }
            HirExpr::BinOp(op, left, right) => {
                let left_ty = self.infer_expr_type(left)?;
                let right_ty = self.infer_expr_type(right)?;
                match op {
                    BinOp::EqEqEq => {
                        if left_ty != HirType::Dynamic
                            && right_ty != HirType::Dynamic
                            && left_ty != right_ty
                        {
                            return Err(format!(
                                "strict equality compares incompatible types {left_ty:?} and {right_ty:?}"
                            ));
                        }
                        Ok(HirType::Bool)
                    }
                    BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                        if !matches!(left_ty, HirType::F64 | HirType::Dynamic)
                            || !matches!(right_ty, HirType::F64 | HirType::Dynamic)
                        {
                            return Err(format!(
                                "numeric comparison requires F64 operands, got {left_ty:?} and {right_ty:?}"
                            ));
                        }
                        Ok(HirType::Bool)
                    }
                    BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::Mod
                    | BinOp::Exp
                    | BinOp::BitOr
                    | BinOp::BitXor
                    | BinOp::BitAnd
                    | BinOp::LShift
                    | BinOp::RShift
                    | BinOp::ZeroFillRShift => {
                        if !matches!(left_ty, HirType::F64 | HirType::Dynamic)
                            || !matches!(right_ty, HirType::F64 | HirType::Dynamic)
                        {
                            return Err(format!(
                                "arithmetic requires F64 operands, got {left_ty:?} and {right_ty:?}"
                            ));
                        }
                        Ok(HirType::F64)
                    }
                }
            }
            HirExpr::Call(callee, args) => {
                let HirExpr::Var(name) = callee.as_ref() else {
                    let (params, ret) = match self.infer_expr_type(callee)? {
                        HirType::Function(params, ret) => (params, ret),
                        HirType::CallableFunction(mut params, _, rest, ret) => {
                            if let Some(rest) = rest {
                                params.push(HirType::Array(rest));
                            }
                            (params, ret)
                        }
                        _ => return Err("call target is not a function value".into()),
                    };
                    if params.len() != args.len() {
                        return Err(format!(
                            "function value expects {} argument(s), got {}",
                            params.len(),
                            args.len()
                        ));
                    }
                    return Ok(*ret);
                };
                match name.as_str() {
                    "console.log" => return Ok(HirType::Void),
                    "__thaw_string_concat" => {
                        if args.len() != 2 {
                            return Err("string concatenation expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::Str, argument, "string concatenation")?;
                        }
                        return Ok(HirType::Str);
                    }
                    "__thaw_bool_to_string" => {
                        let [argument] = args.as_slice() else {
                            return Err("boolean string conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Bool, argument, "boolean string conversion")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_number_to_string" => {
                        let [argument] = args.as_slice() else {
                            return Err("number string conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "number string conversion")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_bool_to_number" => {
                        let [argument] = args.as_slice() else {
                            return Err("boolean number conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Bool, argument, "boolean number conversion")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_string_to_number" => {
                        let [argument] = args.as_slice() else {
                            return Err("string number conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "string number conversion")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_parse_float" => {
                        let [argument] = args.as_slice() else {
                            return Err("parseFloat expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "parseFloat operand")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_parse_int" => {
                        let [text, radix] = args.as_slice() else {
                            return Err("parseInt expects text and radix operands".into());
                        };
                        self.expect_type(&HirType::Str, text, "parseInt text")?;
                        self.expect_type(&HirType::F64, radix, "parseInt radix")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_string_lt" | "__thaw_string_gt" | "__thaw_string_lte"
                    | "__thaw_string_gte" => {
                        if args.len() != 2 {
                            return Err("string comparison expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::Str, argument, "string comparison")?;
                        }
                        return Ok(HirType::Bool);
                    }
                    "__thaw_number_array_to_string"
                    | "__thaw_string_array_to_string"
                    | "__thaw_bool_array_to_string"
                    | "__thaw_object_array_to_string"
                    | "__thaw_object_to_string" => return Ok(HirType::Str),
                    "__thaw_number_array_join"
                    | "__thaw_string_array_join"
                    | "__thaw_bool_array_join"
                    | "__thaw_object_array_join" => {
                        if args.len() != 2 {
                            return Err("array join expects two operands".into());
                        }
                        self.expect_type(&HirType::Str, &args[1], "array join separator")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_array_reverse" => {
                        let [array] = args.as_slice() else {
                            return Err("array reverse expects one operand".into());
                        };
                        let ty = self.infer_expr_type(array)?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array reverse requires a homogeneous array".into());
                        }
                        return Ok(ty);
                    }
                    "__thaw_array_copy_within" => {
                        if args.len() != 4 {
                            return Err("array copyWithin expects four operands".into());
                        }
                        let ty = self.infer_expr_type(&args[0])?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array copyWithin requires a homogeneous array".into());
                        }
                        for argument in &args[1..] {
                            self.expect_type(&HirType::F64, argument, "copyWithin index")?;
                        }
                        return Ok(ty);
                    }
                    "__thaw_array_fill"
                    | "__thaw_number_array_fill"
                    | "__thaw_pointer_array_fill"
                    | "__thaw_bool_array_fill" => {
                        if args.len() != 4 {
                            return Err("array fill expects four operands".into());
                        }
                        let ty = self.infer_expr_type(&args[0])?;
                        let HirType::Array(element) = &ty else {
                            return Err("array fill requires a homogeneous array".into());
                        };
                        self.expect_type(element, &args[1], "fill value")?;
                        for argument in &args[2..] {
                            self.expect_type(&HirType::F64, argument, "fill index")?;
                        }
                        return Ok(ty);
                    }
                    "__thaw_array_slice" => {
                        if args.len() != 3 {
                            return Err("array slice expects three operands".into());
                        }
                        let ty = self.infer_expr_type(&args[0])?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array slice requires a homogeneous array".into());
                        }
                        for argument in &args[1..] {
                            self.expect_type(&HirType::F64, argument, "slice index")?;
                        }
                        return Ok(ty);
                    }
                    "__thaw_array_to_reversed" => {
                        let [array] = args.as_slice() else {
                            return Err("array toReversed expects one operand".into());
                        };
                        let ty = self.infer_expr_type(array)?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array toReversed requires a homogeneous array".into());
                        }
                        return Ok(ty);
                    }
                    "__thaw_number_array_sort"
                    | "__thaw_string_array_sort"
                    | "__thaw_bool_array_sort"
                    | "__thaw_object_array_sort"
                    | "__thaw_number_array_to_sorted"
                    | "__thaw_string_array_to_sorted"
                    | "__thaw_bool_array_to_sorted"
                    | "__thaw_object_array_to_sorted" => {
                        let [array] = args.as_slice() else {
                            return Err("array sort expects one operand".into());
                        };
                        let ty = self.infer_expr_type(array)?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array sort requires a homogeneous array".into());
                        }
                        return Ok(ty);
                    }
                    "__thaw_number_array_index_of"
                    | "__thaw_string_array_index_of"
                    | "__thaw_bool_array_index_of"
                    | "__thaw_object_array_index_of"
                    | "__thaw_number_array_last_index_of"
                    | "__thaw_string_array_last_index_of"
                    | "__thaw_bool_array_last_index_of"
                    | "__thaw_object_array_last_index_of"
                    | "__thaw_number_array_includes"
                    | "__thaw_string_array_includes"
                    | "__thaw_bool_array_includes"
                    | "__thaw_object_array_includes" => {
                        if args.len() != 3 {
                            return Err("array search expects three operands".into());
                        }
                        self.expect_type(&HirType::F64, &args[2], "array search start")?;
                        return Ok(if name.ends_with("_includes") {
                            HirType::Bool
                        } else {
                            HirType::F64
                        });
                    }
                    "__thaw_string_index_of"
                    | "__thaw_string_last_index_of"
                    | "__thaw_string_includes"
                    | "__thaw_string_starts_with"
                    | "__thaw_string_ends_with" => {
                        if args.len() != 3 {
                            return Err("string search expects three operands".into());
                        }
                        self.expect_type(&HirType::Str, &args[0], "string search receiver")?;
                        self.expect_type(&HirType::Str, &args[1], "string search needle")?;
                        self.expect_type(&HirType::F64, &args[2], "string search position")?;
                        return Ok(
                            if matches!(
                                name.as_str(),
                                "__thaw_string_index_of" | "__thaw_string_last_index_of"
                            ) {
                                HirType::F64
                            } else {
                                HirType::Bool
                            },
                        );
                    }
                    "__thaw_string_trim"
                    | "__thaw_string_trim_start"
                    | "__thaw_string_trim_end"
                    | "__thaw_string_to_lower_case"
                    | "__thaw_string_to_upper_case" => {
                        let [argument] = args.as_slice() else {
                            return Err("string trim expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "string trim receiver")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_string_to_array" => {
                        let [value] = args.as_slice() else {
                            return Err("string iterator conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, value, "string iterator source")?;
                        return Ok(HirType::Array(Box::new(HirType::Str)));
                    }
                    "__thaw_string_repeat" => {
                        let [value, count] = args.as_slice() else {
                            return Err("string repeat expects two operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "string repeat receiver")?;
                        self.expect_type(&HirType::F64, count, "string repeat count")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_string_length" => {
                        let [argument] = args.as_slice() else {
                            return Err("string length expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "string length receiver")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_string_char_code_at" => {
                        let [value, index] = args.as_slice() else {
                            return Err("string charCodeAt expects two operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "charCodeAt receiver")?;
                        self.expect_type(&HirType::F64, index, "charCodeAt index")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_json_is_array" => {
                        let [value] = args.as_slice() else {
                            return Err("Array.isArray expects one operand".into());
                        };
                        self.expect_type(&HirType::Json, value, "Array.isArray JSON operand")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_json_keys" => {
                        let [value] = args.as_slice() else {
                            return Err("Object.keys expects one operand".into());
                        };
                        self.expect_type(&HirType::Json, value, "Object.keys JSON operand")?;
                        return Ok(HirType::Array(Box::new(HirType::Str)));
                    }
                    "__thaw_number_is_nan"
                    | "__thaw_number_is_finite"
                    | "__thaw_number_is_integer"
                    | "__thaw_number_is_safe_integer" => {
                        let [argument] = args.as_slice() else {
                            return Err("number predicate expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "number predicate")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_number_object_is" => {
                        let [left, right] = args.as_slice() else {
                            return Err("Object.is number comparison expects two operands".into());
                        };
                        self.expect_type(&HirType::F64, left, "Object.is left operand")?;
                        self.expect_type(&HirType::F64, right, "Object.is right operand")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_number_neg" | "__thaw_math_abs" | "__thaw_math_floor"
                    | "__thaw_math_ceil" | "__thaw_math_trunc" | "__thaw_math_sqrt"
                    | "__thaw_math_sign" | "__thaw_math_round" | "__thaw_math_exp"
                    | "__thaw_math_log" | "__thaw_math_log2" | "__thaw_math_log10"
                    | "__thaw_math_sin" | "__thaw_math_cos" | "__thaw_math_tan"
                    | "__thaw_math_asin" | "__thaw_math_acos" | "__thaw_math_atan"
                    | "__thaw_math_sinh" | "__thaw_math_cosh" | "__thaw_math_tanh"
                    | "__thaw_math_cbrt" | "__thaw_math_acosh" | "__thaw_math_asinh"
                    | "__thaw_math_atanh" | "__thaw_math_expm1" | "__thaw_math_log1p" => {
                        let [argument] = args.as_slice() else {
                            return Err("unary Math function expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "Math operand")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_fround" | "__thaw_math_clz32" => {
                        let [argument] = args.as_slice() else {
                            return Err("unary Math function expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "Math operand")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_random" => {
                        if !args.is_empty() {
                            return Err("Math.random expects no operands".into());
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_pow" => {
                        if args.len() != 2 {
                            return Err("Math.pow expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math.pow operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_atan2" => {
                        if args.len() != 2 {
                            return Err("Math.atan2 expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math.atan2 operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_imul" => {
                        if args.len() != 2 {
                            return Err("Math.imul expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math.imul operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_min" | "__thaw_math_max" | "__thaw_math_hypot" => {
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math extrema operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "fetch" => return Ok(HirType::Str),
                    "sleep" => return Ok(HirType::Promise(Box::new(HirType::Void))),
                    "Promise.all" => {
                        for (index, arg) in args.iter().enumerate() {
                            match self.infer_expr_type(arg)? {
                                HirType::Promise(value) if *value == HirType::F64 => {}
                                HirType::F64
                                    if matches!(arg, HirExpr::Call(callee, _)
                                        if matches!(callee.as_ref(), HirExpr::Var(name)
                                            if self.signatures.get(name).is_some_and(|signature| signature.is_async && signature.ret == HirType::F64))) => {}
                                other => {
                                    return Err(format!(
                                        "Promise.all element {index} must be Promise<number>, got {other:?}"
                                    ))
                                }
                            }
                        }
                        return Ok(HirType::Promise(Box::new(HirType::Array(Box::new(
                            HirType::F64,
                        )))));
                    }
                    "JSON.parse" => return Ok(HirType::Json),
                    "JSON.stringify" => return Ok(HirType::Str),
                    // QuickJS-NG fallback path (docs/design/bridge.md
                    // section 7): `loadScript` evaluates JS source into
                    // the global engine context; `callDynamic` calls a
                    // top-level function it defined, by name, with `Json`
                    // args in and a `Json` result out.
                    "loadScript" => return Ok(HirType::Bool),
                    "callDynamic" => return Ok(HirType::Json),
                    "getDynamicValue" => return Ok(HirType::JsValue),
                    "callDynamicValue" => return Ok(HirType::Json),
                    "callDynamicValueHandle" => return Ok(HirType::JsValue),
                    "callDynamicValueWithValue" => return Ok(HirType::Json),
                    "releaseDynamicValue" => return Ok(HirType::Bool),
                    "getDynamicProperty" => return Ok(HirType::JsValue),
                    "setDynamicProperty" => return Ok(HirType::Bool),
                    "callDynamicMethod" => return Ok(HirType::Json),
                    "readDynamicValue" => return Ok(HirType::Json),
                    "callDynamicValueMixed" => return Ok(HirType::Json),
                    "constructDynamicValue" => return Ok(HirType::JsValue),
                    "loadNativeAddon" => return Ok(HirType::Bool),
                    "loadNativeAddonEmbedded" => return Ok(HirType::Bool),
                    "callNativeAddon" => return Ok(HirType::Json),
                    "callNativeAddonWithCallback" => return Ok(HirType::Json),
                    "pollNativeAddonEvents" => return Ok(HirType::F64),
                    _ => {}
                }
                if let Some(HirType::Function(params, ret)) = self.scope.get(name) {
                    if params.len() != args.len() {
                        return Err(format!(
                            "function value `{name}` expects {} argument(s), got {}",
                            params.len(),
                            args.len()
                        ));
                    }
                    return Ok(ret.as_ref().clone());
                }
                if let Some(HirType::CallableFunction(params, _, rest, ret)) = self.scope.get(name)
                {
                    let abi_count = params.len() + usize::from(rest.is_some());
                    if abi_count != args.len() {
                        return Err(format!(
                            "callable function value `{name}` expects {abi_count} ABI argument(s), got {}",
                            args.len()
                        ));
                    }
                    return Ok(ret.as_ref().clone());
                }
                if let Some(return_type) = self.generic_call_returns.get(name) {
                    return Ok(return_type.clone());
                }
                let signature = self.signatures.get(name).or_else(|| {
                    name.split_once("__thaw_")
                        .and_then(|(base, _)| self.signatures.get(base))
                        .filter(|signature| !signature.generic_type_params.is_empty())
                });
                match signature {
                    Some(sig) => {
                        if !sig.generic_type_params.is_empty() {
                            let actual = args
                                .iter()
                                .map(|arg| self.infer_expr_type(arg))
                                .collect::<Result<Vec<_>, _>>()?;
                            let types = infer_generic_type_tuple(
                                sig,
                                &actual,
                                self.interfaces,
                                self.generic_interfaces,
                            )?;
                            let substitution = sig
                                .generic_type_params
                                .iter()
                                .cloned()
                                .zip(types)
                                .collect::<HashMap<_, _>>();
                            resolve_ts_type_with_substitution(
                                sig.generic_return_type
                                    .as_ref()
                                    .expect("generic return type"),
                                &substitution,
                                self.interfaces,
                                self.generic_interfaces,
                                &mut Vec::new(),
                            )
                        } else if sig.is_async {
                            Ok(HirType::Promise(Box::new(sig.ret.clone())))
                        } else {
                            Ok(sig.ret.clone())
                        }
                    }
                    None => Err(format!("call to unknown function `{name}`")),
                }
            }
            HirExpr::FunctionCallWithThis(_, _, _, _, ret) => Ok(ret.clone()),
            HirExpr::FunctionBindThis(_, _, bound, params, ret) => Ok(HirType::Function(
                params[bound.len()..].to_vec(),
                Box::new(ret.clone()),
            )),
            HirExpr::PromiseAll(_, element) => Ok(HirType::Promise(Box::new(HirType::Array(
                Box::new(element.clone()),
            )))),
            HirExpr::PromiseAllArray(_, element) => Ok(HirType::Promise(Box::new(HirType::Array(
                Box::new(element.clone()),
            )))),
            HirExpr::PromiseAllTuple(_, elements) => {
                Ok(HirType::Promise(Box::new(HirType::Tuple(elements.clone()))))
            }
            HirExpr::PromiseRace(_, element)
            | HirExpr::PromiseRaceArray(_, element)
            | HirExpr::PromiseAny(_, element)
            | HirExpr::PromiseAnyArray(_, element) => {
                Ok(HirType::Promise(Box::new(element.clone())))
            }
            HirExpr::PromiseAllSettled(_, element)
            | HirExpr::PromiseAllSettledArray(_, element) => Ok(HirType::Promise(Box::new(
                HirType::Array(Box::new(promise_settled_result_type(element.clone()))),
            ))),
            HirExpr::PromiseNew(_, resolved, _) => Ok(HirType::Promise(Box::new(resolved.clone()))),
            HirExpr::PromiseThen(_, _, _, output, _, _) => {
                Ok(HirType::Promise(Box::new(output.clone())))
            }
            HirExpr::PromiseFinally(_, _, input, _) => {
                Ok(HirType::Promise(Box::new(input.clone())))
            }
            HirExpr::DynamicCall(signature, _) => Ok(signature.ret.clone()),
            HirExpr::ArrayLit(values) => {
                if values.is_empty() {
                    return Ok(HirType::Array(Box::new(HirType::F64)));
                }
                let array_element_type =
                    |value: &HirExpr| -> Result<HirType, String> { self.infer_expr_type(value) };
                let elements = values
                    .iter()
                    .map(array_element_type)
                    .collect::<Result<Vec<_>, _>>()?;
                if elements.iter().all(|element| element == &elements[0]) {
                    Ok(HirType::Array(Box::new(elements[0].clone())))
                } else {
                    Ok(HirType::Tuple(elements))
                }
            }
            HirExpr::ArrayConcat(_, element) => Ok(HirType::Array(Box::new(element.clone()))),
            HirExpr::Index(arr, index) => {
                self.expect_type(&HirType::F64, index, "array index")?;
                match self.infer_expr_type(arr)? {
                    HirType::Array(elem) => Ok(*elem),
                    other => Err(format!("cannot index into a value of type {other:?}")),
                }
            }
            HirExpr::TypedIndex(_, _, element) => Ok(element.clone()),
            HirExpr::IndexAssign(arr, index, value) => {
                self.expect_type(&HirType::F64, index, "array index")?;
                let HirType::Array(element) = self.infer_expr_type(arr)? else {
                    return Err("index assignment target is not an array".into());
                };
                self.expect_type(&element, value, "array assignment")?;
                Ok(*element)
            }
            HirExpr::ArrayLen(_) => Ok(HirType::F64),
            HirExpr::EnumReverseLookup(_, _) => Ok(HirType::Optional(Box::new(HirType::Str))),
            HirExpr::EnvVar(_) => Ok(HirType::Str),
            HirExpr::ObjectLit(fields) => {
                let fields = fields
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), self.infer_expr_type(value)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(HirType::Object(fields))
            }
            HirExpr::ObjectAlloc(ty @ HirType::Object(_)) => Ok(ty.clone()),
            HirExpr::ObjectAlloc(other) => Err(format!(
                "object allocation requires an object type, got {other:?}"
            )),
            HirExpr::PropAccess(_, object_ty, field) => match object_ty {
                HirType::Object(fields) => fields
                    .iter()
                    .find(|(name, _)| name == field)
                    .map(|(_, ty)| ty.clone())
                    .ok_or_else(|| format!("object has no field `{field}`")),
                other => Err(format!(
                    "cannot access `.{field}` on a value of type {other:?}"
                )),
            },
            HirExpr::DynamicPropAccess(_, _, _, result) => Ok(result.clone()),
            HirExpr::PropAssign(_, _, _, value) => self.infer_expr_type(value),
            HirExpr::JsonObjectLit(_, element) => {
                Ok(HirType::Dictionary(Box::new(element.clone())))
            }
            HirExpr::JsonGet(_, _) | HirExpr::JsonIndex(_, _) | HirExpr::JsonKey(_, _) => {
                Ok(HirType::Json)
            }
            HirExpr::JsonSet(_, _, _, element) => Ok(element.clone()),
            HirExpr::JsonAsNumber(_) => Ok(HirType::F64),
            HirExpr::JsonAsString(_) => Ok(HirType::Str),
            HirExpr::JsonAsBool(_) => Ok(HirType::Bool),
            HirExpr::FfiCall(sig, _) => Ok(sig.ret.clone()),
            HirExpr::Await(inner) | HirExpr::AwaitPromise(inner, _) => {
                match self.infer_expr_type(inner)? {
                    HirType::Promise(value) => Ok(*value),
                    // Legacy/direct await sources can already expose their
                    // resolved type to the surrounding expression.
                    other => Ok(other),
                }
            }
            // The Lambda node now preserves typed parameters and its body,
            // but function values do not have a native ABI until the next
            // callback-lowering phase. Keep the enclosing local dynamic
            // instead of discarding or pretending to know that ABI.
            HirExpr::RecursiveClosure(_, ty, _) => Ok(ty.clone()),
            HirExpr::TypedClosure(ty, _) => Ok(ty.clone()),
            HirExpr::Lambda(_, params, ret, _) => Ok(HirType::Function(
                params.iter().map(|param| param.ty.clone()).collect(),
                Box::new(ret.clone()),
            )),
            HirExpr::ThrowValue(_, fallback) => self.infer_expr_type(fallback),
            HirExpr::Block(stmts) => self.infer_return_type(stmts),
        }
    }
}
