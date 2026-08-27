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
}
