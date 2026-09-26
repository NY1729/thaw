impl<'a> FnLowerer<'a> {
    /// `.length` of a union whose members are all arrays: every native array
    /// stores its own length, and the member layouts differ, so dispatch on
    /// the runtime tag and take the selected member's length.
    fn lower_union_array_length(
        &mut self,
        object: HirExpr,
        elements: &[HirType],
    ) -> Result<HirExpr, String> {
        let name = format!("__thaw_union_array_{}", self.next_binding);
        self.next_binding += 1;
        let object_type = HirType::Union(elements.to_vec());
        self.scope.insert(name.clone(), object_type.clone());
        let bound = HirExpr::Var(name.clone());
        let mut result: Option<HirExpr> = None;
        for (index, element) in elements.iter().enumerate().rev() {
            let HirType::Array(_) = element else {
                return Err(format!(
                    "cannot take `.length`: union member {element:?} is not an array"
                ));
            };
            let length = HirExpr::ArrayLen(Box::new(HirExpr::UnionValue(
                Box::new(bound.clone()),
                index,
                elements.to_vec(),
            )));
            result = Some(match result {
                None => length,
                Some(rest) => HirExpr::Conditional(
                    Box::new(HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(
                            Box::new(bound.clone()),
                            elements.to_vec(),
                        )),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                    )),
                    Box::new(length),
                    Box::new(rest),
                    HirType::F64,
                ),
            });
        }
        let result = result.ok_or("cannot take `.length` of an empty union")?;
        self.wrap_call_argument_bindings(result, &[(name, object_type, object)])
    }

    fn lower_union_property_read(
        &mut self,
        object: HirExpr,
        elements: &[HirType],
        property: &str,
    ) -> Result<HirExpr, String> {
        if property == "length" && elements.iter().all(|e| matches!(e, HirType::Array(_))) {
            return self.lower_union_array_length(object, elements);
        }
        let mut field_types = Vec::with_capacity(elements.len());
        for element in elements {
            if matches!(element, HirType::Undefined | HirType::Null) {
                continue;
            }
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
            if matches!(element, HirType::Undefined | HirType::Null) {
                let returns = vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(format!(
                    "Cannot read properties of {}",
                    if *element == HirType::Null { "null" } else { "undefined" }
                ))))];
                if index + 1 == elements.len() {
                    statements.extend(returns);
                } else {
                    statements.push(HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(
                                Box::new(HirExpr::Var(parameter.clone())), elements.to_vec(),
                            )),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        ),
                        returns,
                        Vec::new(),
                    ));
                }
                continue;
            }
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
                        | HirType::Map(_, _)
                        | HirType::Set(_)
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
        &mut self,
        value: HirExpr,
        ty: &HirType,
        result: &HirType,
    ) -> Result<Vec<HirStmt>, String> {
        match ty {
            HirType::Optional(payload) => {
                let mut statements = vec![HirStmt::If(
                    HirExpr::OptionalIsNone(Box::new(value.clone()), payload.as_ref().clone()),
                    vec![HirStmt::Return(Some(
                        self.coerce_to_declared(result, HirExpr::Lit(HirLit::Undefined))?,
                    ))],
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
                    vec![HirStmt::Return(Some(
                        self.coerce_to_declared(result, HirExpr::Lit(HirLit::Null))?,
                    ))],
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
                        vec![HirStmt::Return(Some(
                            self.coerce_to_declared(result, HirExpr::Lit(HirLit::Null))?,
                        ))],
                        Vec::new(),
                    ),
                    HirStmt::If(
                        HirExpr::NullishIsUndefined(
                            Box::new(value.clone()),
                            payload.as_ref().clone(),
                        ),
                        vec![HirStmt::Return(Some(
                            self.coerce_to_declared(result, HirExpr::Lit(HirLit::Undefined))?,
                        ))],
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
            _ => Ok(vec![HirStmt::Return(Some(
                self.coerce_to_declared(result, value)?,
            ))]),
        }
    }
}
