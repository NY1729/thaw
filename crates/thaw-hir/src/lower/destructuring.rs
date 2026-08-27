impl<'a> FnLowerer<'a> {
    fn lower_binding_pattern(
        &mut self,
        pattern: &Pat,
        value: HirExpr,
        ty: &HirType,
        statements: &mut Vec<HirStmt>,
    ) -> Result<(), String> {
        match pattern {
            Pat::Ident(binding) => {
                let array_discriminants = self.hir_array_element_discriminants(&value);
                let property_discriminants = self.hir_object_array_property_discriminants(&value);
                let function_discriminants = self.hir_function_property_discriminants(&value);
                let object_function_discriminants =
                    self.hir_object_function_property_discriminants(&value);
                let binding_type = if let Some(annotation) = &binding.type_ann {
                    let annotated = lower_ts_type(
                        &annotation.type_ann,
                        self.interfaces,
                        self.generic_interfaces,
                    )?;
                    self.expect_type(&annotated, &value, "destructured binding")?;
                    annotated
                } else {
                    ty.clone()
                };
                let name = self.bind_local(binding.id.sym.as_ref(), binding_type.clone());
                if let Some(discriminants) = array_discriminants {
                    self.array_element_discriminants
                        .insert(name.clone(), discriminants);
                }
                if let Some(discriminants) = property_discriminants {
                    self.object_array_property_discriminants
                        .insert(name.clone(), discriminants);
                }
                if let Some(discriminants) = function_discriminants {
                    if let Some(value) = discriminants.value {
                        self.function_value_discriminants
                            .insert(name.clone(), value);
                    }
                    if let Some(array) = discriminants.array {
                        self.function_value_array_discriminants
                            .insert(name.clone(), array);
                    }
                    if !discriminants.nested_array.is_empty() {
                        self.function_value_nested_array_discriminants
                            .insert(name.clone(), discriminants.nested_array);
                    }
                    if !discriminants.object.is_empty() {
                        self.function_value_object_array_property_discriminants
                            .insert(name.clone(), discriminants.object);
                    }
                    if !discriminants.functions.is_empty() {
                        self.function_value_object_function_property_discriminants
                            .insert(name.clone(), discriminants.functions);
                    }
                }
                if let Some(discriminants) = object_function_discriminants {
                    self.object_function_property_discriminants
                        .insert(name.clone(), discriminants);
                }
                statements.push(HirStmt::Let(name, binding_type, value));
                Ok(())
            }
            Pat::Object(pattern) => {
                if let HirType::Dictionary(element) = ty {
                    return self.lower_dictionary_object_binding_pattern(
                        pattern,
                        value,
                        element,
                        statements,
                    );
                }
                if let HirType::Union(elements) = ty {
                    return self
                        .lower_union_object_binding_pattern(pattern, value, elements, statements);
                }
                let HirType::Object(fields) = ty else {
                    return Err(format!("object pattern cannot destructure {ty:?}"));
                };
                let mut used = BTreeSet::new();
                for property in &pattern.props {
                    match property {
                        ObjectPatProp::Assign(property) => {
                            let key = property.key.id.sym.to_string();
                            let field_type = fields
                                .iter()
                                .find(|(name, _)| name == &key)
                                .map(|(_, ty)| ty.clone())
                                .ok_or_else(|| format!("object has no field `{key}`"))?;
                            used.insert(key.clone());
                            let mut field_value =
                                HirExpr::PropAccess(Box::new(value.clone()), ty.clone(), key);
                            let mut binding_type = field_type.clone();
                            if let Some(default) = &property.value {
                                let default = self.lower_expr(default)?;
                                field_value = self.lower_undefined_default(field_value, default)?;
                                binding_type = self.infer_expr_type(&field_value)?;
                            }
                            self.lower_binding_pattern(
                                &Pat::Ident(property.key.clone()),
                                field_value,
                                &binding_type,
                                statements,
                            )?;
                        }
                        ObjectPatProp::KeyValue(property) => {
                            let key =
                                match &property.key {
                                    PropName::Ident(key) => key.sym.to_string(),
                                    PropName::Str(key) => key.value.to_string_lossy().into_owned(),
                                    PropName::Computed(computed) => match computed.expr.as_ref() {
                                        Expr::Lit(Lit::Str(key)) => {
                                            key.value.to_string_lossy().into_owned()
                                        }
                                        _ => return Err(
                                            "computed destructuring keys must be string literals"
                                                .into(),
                                        ),
                                    },
                                    _ => return Err("unsupported object destructuring key".into()),
                                };
                            let field_type = fields
                                .iter()
                                .find(|(name, _)| name == &key)
                                .map(|(_, ty)| ty.clone())
                                .ok_or_else(|| format!("object has no field `{key}`"))?;
                            used.insert(key.clone());
                            self.lower_binding_pattern(
                                &property.value,
                                HirExpr::PropAccess(Box::new(value.clone()), ty.clone(), key),
                                &field_type,
                                statements,
                            )?;
                        }
                        ObjectPatProp::Rest(rest) => {
                            let remaining = fields
                                .iter()
                                .filter(|(name, _)| !used.contains(name))
                                .cloned()
                                .collect::<Vec<_>>();
                            let rest_value = HirExpr::ObjectLit(
                                remaining
                                    .iter()
                                    .map(|(name, _)| {
                                        (
                                            name.clone(),
                                            HirExpr::PropAccess(
                                                Box::new(value.clone()),
                                                ty.clone(),
                                                name.clone(),
                                            ),
                                        )
                                    })
                                    .collect(),
                            );
                            self.lower_binding_pattern(
                                &rest.arg,
                                rest_value,
                                &HirType::Object(remaining),
                                statements,
                            )?;
                        }
                    }
                }
                Ok(())
            }
            Pat::Array(pattern) => {
                if let HirType::Union(elements) = ty {
                    if elements
                        .iter()
                        .all(|element| matches!(element, HirType::Tuple(_)))
                    {
                        return self.lower_union_tuple_binding_pattern(
                            pattern, value, elements, statements,
                        );
                    }
                }
                let HirType::Tuple(elements) = ty else {
                    return Err(format!(
                        "array pattern requires a fixed-length tuple, got {ty:?}"
                    ));
                };
                for (index, element_pattern) in pattern.elems.iter().enumerate() {
                    let Some(element_pattern) = element_pattern else {
                        continue;
                    };
                    if let Pat::Rest(rest) = element_pattern {
                        let remaining = elements[index..].to_vec();
                        let rest_value = HirExpr::ArrayLit(
                            remaining
                                .iter()
                                .enumerate()
                                .map(|(offset, element)| {
                                    HirExpr::TypedIndex(
                                        Box::new(value.clone()),
                                        Box::new(HirExpr::Lit(HirLit::F64(
                                            (index + offset) as f64,
                                        ))),
                                        element.clone(),
                                    )
                                })
                                .collect(),
                        );
                        let rest_type = if remaining
                            .first()
                            .is_some_and(|first| remaining.iter().all(|element| element == first))
                        {
                            HirType::Array(Box::new(
                                remaining.first().cloned().unwrap_or(HirType::F64),
                            ))
                        } else {
                            HirType::Tuple(remaining)
                        };
                        self.lower_binding_pattern(&rest.arg, rest_value, &rest_type, statements)?;
                        break;
                    }
                    let element_type = elements
                        .get(index)
                        .cloned()
                        .ok_or_else(|| format!("tuple pattern index {index} is out of bounds"))?;
                    self.lower_binding_pattern(
                        element_pattern,
                        HirExpr::TypedIndex(
                            Box::new(value.clone()),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            element_type.clone(),
                        ),
                        &element_type,
                        statements,
                    )?;
                }
                Ok(())
            }
            Pat::Assign(assign) => {
                let default = self.lower_expr(&assign.right)?;
                let default_type = self.infer_expr_type(&default)?;
                let value = self.lower_undefined_default(value, default)?;
                let value_type = self.infer_expr_type(&value)?;
                self.lower_binding_pattern(&assign.left, value, &value_type, statements)?;
                if let Pat::Ident(binding) = assign.left.as_ref() {
                    self.destructuring_default_types
                        .insert(self.resolve_binding(binding.id.sym.as_ref()), default_type);
                }
                Ok(())
            }
            Pat::Rest(_) => Err("rest patterns are only valid inside object/array patterns".into()),
            _ => Err("unsupported destructuring binding pattern".into()),
        }
    }

    fn lower_union_object_binding_pattern(
        &mut self,
        pattern: &swc_ecma_ast::ObjectPat,
        value: HirExpr,
        elements: &[HirType],
        statements: &mut Vec<HirStmt>,
    ) -> Result<(), String> {
        let discriminants = match &value {
            HirExpr::Var(name) => self.union_discriminants.get(name).cloned(),
            _ => None,
        };
        let mut extracted = Vec::new();
        let mut discriminant_bindings = Vec::new();
        let mut used = BTreeSet::new();
        for property in &pattern.props {
            match property {
                ObjectPatProp::Assign(property) => {
                    let key = property.key.id.sym.to_string();
                    let mut source_types =
                        Self::union_destructured_property_source_types(elements, &key)?;
                    let mut field_value =
                        self.lower_union_property_read(value.clone(), elements, &key)?;
                    let mut field_type = self.infer_expr_type(&field_value)?;
                    used.insert(key.clone());
                    if let Some(default) = &property.value {
                        let default = self.lower_expr(default)?;
                        let default_type = self.infer_expr_type(&default)?;
                        field_value = self.lower_undefined_default(field_value, default)?;
                        field_type = self.infer_expr_type(&field_value)?;
                        source_types = Self::defaulted_destructured_source_types(
                            &source_types,
                            &default_type,
                        )?;
                    }
                    self.lower_binding_pattern(
                        &Pat::Ident(property.key.clone()),
                        field_value,
                        &field_type,
                        statements,
                    )?;
                    if property.value.is_none() {
                        let name = self.resolve_binding(property.key.id.sym.as_ref());
                        discriminant_bindings.push((key, name.clone()));
                    }
                    extracted.push(CorrelatedDestructuredBinding {
                        name: self.resolve_binding(property.key.id.sym.as_ref()),
                        ty: field_type,
                        source_types,
                    });
                }
                ObjectPatProp::KeyValue(property) => {
                    let key = match &property.key {
                        PropName::Ident(key) => key.sym.to_string(),
                        PropName::Str(key) => key.value.to_string_lossy().into_owned(),
                        PropName::Computed(computed) => match computed.expr.as_ref() {
                            Expr::Lit(Lit::Str(key)) => key.value.to_string_lossy().into_owned(),
                            _ => {
                                return Err(
                                    "computed destructuring keys must be string literals".into()
                                )
                            }
                        },
                        _ => return Err("unsupported object destructuring key".into()),
                    };
                    let source_types =
                        Self::union_destructured_property_source_types(elements, &key)?;
                    let field_value =
                        self.lower_union_property_read(value.clone(), elements, &key)?;
                    let field_type = self.infer_expr_type(&field_value)?;
                    used.insert(key.clone());
                    self.lower_binding_pattern(
                        &property.value,
                        field_value,
                        &field_type,
                        statements,
                    )?;
                    if let Pat::Ident(binding) = property.value.as_ref() {
                        discriminant_bindings
                            .push((key.clone(), self.resolve_binding(binding.id.sym.as_ref())));
                    }
                    self.collect_correlated_destructured_bindings(
                        &property.value,
                        &source_types,
                        &mut extracted,
                    )?;
                }
                ObjectPatProp::Rest(rest) => {
                    let source_types = Self::union_destructured_rest_source_types(elements, &used)?;
                    let (rest_value, rest_type) =
                        self.lower_union_object_rest(value.clone(), elements, &used)?;
                    self.lower_binding_pattern(&rest.arg, rest_value, &rest_type, statements)?;
                    self.collect_correlated_destructured_bindings(
                        &rest.arg,
                        &source_types,
                        &mut extracted,
                    )?;
                }
            }
        }
        if let Some(discriminants) = discriminants {
            self.register_destructured_union_correlations(
                elements,
                &discriminants,
                &discriminant_bindings,
                &extracted,
            )?;
        }
        Ok(())
    }

    fn lower_union_tuple_binding_pattern(
        &mut self,
        pattern: &swc_ecma_ast::ArrayPat,
        value: HirExpr,
        elements: &[HirType],
        statements: &mut Vec<HirStmt>,
    ) -> Result<(), String> {
        for (index, element_pattern) in pattern.elems.iter().enumerate() {
            let Some(element_pattern) = element_pattern else {
                continue;
            };
            if let Pat::Rest(rest) = element_pattern {
                let (rest_value, rest_type) =
                    self.lower_union_tuple_rest(value.clone(), elements, index)?;
                self.lower_binding_pattern(&rest.arg, rest_value, &rest_type, statements)?;
                break;
            }
            let element_value =
                self.lower_union_tuple_index_read(value.clone(), elements, index)?;
            let element_type = self.infer_expr_type(&element_value)?;
            self.lower_binding_pattern(element_pattern, element_value, &element_type, statements)?;
        }
        Ok(())
    }

    fn lower_union_object_assignment_pattern(
        &mut self,
        pattern: &swc_ecma_ast::ObjectPat,
        value: HirExpr,
        elements: &[HirType],
        statements: &mut Vec<HirStmt>,
    ) -> Result<(), String> {
        let discriminants = match &value {
            HirExpr::Var(name) => self.union_discriminants.get(name).cloned(),
            _ => None,
        };
        let mut discriminant_bindings = Vec::new();
        let mut used = BTreeSet::new();
        for property in &pattern.props {
            match property {
                ObjectPatProp::Assign(property) => {
                    let key = property.key.id.sym.to_string();
                    let mut field_value =
                        self.lower_union_property_read(value.clone(), elements, &key)?;
                    let mut field_type = self.infer_expr_type(&field_value)?;
                    used.insert(key.clone());
                    if let Some(default) = &property.value {
                        let default = self.lower_expr(default)?;
                        field_value = self.lower_undefined_default(field_value, default)?;
                        field_type = self.infer_expr_type(&field_value)?;
                    }
                    self.lower_assignment_pattern(
                        &Pat::Ident(property.key.clone()),
                        field_value,
                        &field_type,
                        statements,
                    )?;
                    if property.value.is_none() {
                        discriminant_bindings
                            .push((key, self.resolve_binding(property.key.id.sym.as_ref())));
                    }
                }
                ObjectPatProp::KeyValue(property) => {
                    let key = match &property.key {
                        PropName::Ident(key) => key.sym.to_string(),
                        PropName::Str(key) => key.value.to_string_lossy().into_owned(),
                        PropName::Computed(computed) => match computed.expr.as_ref() {
                            Expr::Lit(Lit::Str(key)) => key.value.to_string_lossy().into_owned(),
                            _ => {
                                return Err(
                                    "computed destructuring keys must be string literals".into()
                                )
                            }
                        },
                        _ => return Err("unsupported object destructuring key".into()),
                    };
                    let field_value =
                        self.lower_union_property_read(value.clone(), elements, &key)?;
                    let field_type = self.infer_expr_type(&field_value)?;
                    used.insert(key.clone());
                    self.lower_assignment_pattern(
                        &property.value,
                        field_value,
                        &field_type,
                        statements,
                    )?;
                    if let Pat::Ident(binding) = property.value.as_ref() {
                        discriminant_bindings
                            .push((key, self.resolve_binding(binding.id.sym.as_ref())));
                    }
                }
                ObjectPatProp::Rest(rest) => {
                    let (rest_value, rest_type) =
                        self.lower_union_object_rest(value.clone(), elements, &used)?;
                    self.lower_assignment_pattern(&rest.arg, rest_value, &rest_type, statements)?;
                }
            }
        }
        if let Some(discriminants) = discriminants {
            let source_types = elements
                .iter()
                .cloned()
                .map(|element| vec![element])
                .collect::<Vec<_>>();
            let mut extracted = Vec::new();
            self.collect_correlated_destructured_bindings(
                &Pat::Object(pattern.clone()),
                &source_types,
                &mut extracted,
            )?;
            self.register_destructured_union_correlations(
                elements,
                &discriminants,
                &discriminant_bindings,
                &extracted,
            )?;
        }
        Ok(())
    }

    fn lower_union_tuple_assignment_pattern(
        &mut self,
        pattern: &swc_ecma_ast::ArrayPat,
        value: HirExpr,
        elements: &[HirType],
        statements: &mut Vec<HirStmt>,
    ) -> Result<(), String> {
        for (index, element_pattern) in pattern.elems.iter().enumerate() {
            let Some(element_pattern) = element_pattern else {
                continue;
            };
            if let Pat::Rest(rest) = element_pattern {
                let (rest_value, rest_type) =
                    self.lower_union_tuple_rest(value.clone(), elements, index)?;
                self.lower_assignment_pattern(&rest.arg, rest_value, &rest_type, statements)?;
                break;
            }
            let element_value =
                self.lower_union_tuple_index_read(value.clone(), elements, index)?;
            let element_type = self.infer_expr_type(&element_value)?;
            self.lower_assignment_pattern(
                element_pattern,
                element_value,
                &element_type,
                statements,
            )?;
        }
        Ok(())
    }

    fn lower_union_tuple_index_read(
        &self,
        value: HirExpr,
        elements: &[HirType],
        index: usize,
    ) -> Result<HirExpr, String> {
        let mut element_types = Vec::new();
        for element in elements {
            let HirType::Tuple(tuple) = element else {
                return Err("union array pattern requires tuple members".into());
            };
            let element_type = tuple
                .get(index)
                .cloned()
                .ok_or_else(|| format!("tuple pattern index {index} is out of bounds"))?;
            if !element_types.contains(&element_type) {
                element_types.push(element_type);
            }
        }
        let result_type = match element_types.as_slice() {
            [element] => element.clone(),
            element_types => {
                let mut members = Vec::new();
                for element_type in element_types {
                    Self::flatten_property_union_members(element_type, &mut members)?;
                }
                match members.as_slice() {
                    [member] => member.clone(),
                    _ => HirType::Union(members),
                }
            }
        };
        let parameter = "__thaw_union_tuple_value".to_string();
        let mut body = Vec::new();
        for (source_index, element) in elements.iter().enumerate() {
            let HirType::Tuple(tuple) = element else {
                unreachable!()
            };
            let element_type = tuple[index].clone();
            let item = HirExpr::TypedIndex(
                Box::new(HirExpr::UnionValue(
                    Box::new(HirExpr::Var(parameter.clone())),
                    source_index,
                    elements.to_vec(),
                )),
                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                element_type.clone(),
            );
            let returns = if element_types.len() == 1 {
                vec![HirStmt::Return(Some(item))]
            } else {
                self.lower_flattened_property_return(item, &element_type, &result_type)?
            };
            if source_index + 1 == elements.len() {
                body.extend(returns);
            } else {
                body.push(HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(
                            Box::new(HirExpr::Var(parameter.clone())),
                            elements.to_vec(),
                        )),
                        Box::new(HirExpr::Lit(HirLit::F64(source_index as f64))),
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
                    ty: HirType::Union(elements.to_vec()),
                }],
                result_type,
                Box::new(HirExpr::Block(body)),
            )),
            vec![value],
        ))
    }

    fn lower_union_tuple_rest(
        &self,
        value: HirExpr,
        elements: &[HirType],
        start: usize,
    ) -> Result<(HirExpr, HirType), String> {
        let mut rest_types = Vec::new();
        for element in elements {
            let HirType::Tuple(tuple) = element else {
                return Err("union array rest requires tuple members".into());
            };
            if start > tuple.len() {
                return Err(format!("tuple rest index {start} is out of bounds"));
            }
            let remaining = tuple[start..].to_vec();
            let rest_type = if remaining
                .first()
                .is_some_and(|first| remaining.iter().all(|element| element == first))
            {
                HirType::Array(Box::new(remaining.first().cloned().unwrap_or(HirType::F64)))
            } else {
                HirType::Tuple(remaining)
            };
            if !rest_types.contains(&rest_type) {
                rest_types.push(rest_type);
            }
        }
        let result_type = match rest_types.as_slice() {
            [rest] => rest.clone(),
            rests => HirType::Union(rests.to_vec()),
        };
        let parameter = "__thaw_union_tuple_rest_value".to_string();
        let mut body = Vec::new();
        for (source_index, element) in elements.iter().enumerate() {
            let HirType::Tuple(tuple) = element else {
                unreachable!()
            };
            let member = HirExpr::UnionValue(
                Box::new(HirExpr::Var(parameter.clone())),
                source_index,
                elements.to_vec(),
            );
            let rest = HirExpr::ArrayLit(
                tuple[start..]
                    .iter()
                    .enumerate()
                    .map(|(offset, ty)| {
                        HirExpr::TypedIndex(
                            Box::new(member.clone()),
                            Box::new(HirExpr::Lit(HirLit::F64((start + offset) as f64))),
                            ty.clone(),
                        )
                    })
                    .collect(),
            );
            let rest = self.coerce_to_declared(&result_type, rest)?;
            if source_index + 1 == elements.len() {
                body.push(HirStmt::Return(Some(rest)));
            } else {
                body.push(HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(
                            Box::new(HirExpr::Var(parameter.clone())),
                            elements.to_vec(),
                        )),
                        Box::new(HirExpr::Lit(HirLit::F64(source_index as f64))),
                    ),
                    vec![HirStmt::Return(Some(rest))],
                    Vec::new(),
                ));
            }
        }
        Ok((
            HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: parameter,
                        ty: HirType::Union(elements.to_vec()),
                    }],
                    result_type.clone(),
                    Box::new(HirExpr::Block(body)),
                )),
                vec![value],
            ),
            result_type,
        ))
    }

    fn union_destructured_property_source_types(
        elements: &[HirType],
        property: &str,
    ) -> Result<Vec<Vec<HirType>>, String> {
        elements
            .iter()
            .map(|element| {
                let HirType::Object(fields) = element else {
                    return Err("correlated destructuring requires object members".into());
                };
                fields
                    .iter()
                    .find(|(name, _)| name == property)
                    .map(|(_, ty)| vec![ty.clone()])
                    .ok_or_else(|| format!("object has no field `{property}`"))
            })
            .collect()
    }

    fn union_destructured_rest_source_types(
        elements: &[HirType],
        used: &BTreeSet<Symbol>,
    ) -> Result<Vec<Vec<HirType>>, String> {
        elements
            .iter()
            .map(|element| {
                let HirType::Object(fields) = element else {
                    return Err("correlated destructuring requires object members".into());
                };
                Ok(vec![HirType::Object(
                    fields
                        .iter()
                        .filter(|(name, _)| !used.contains(name))
                        .cloned()
                        .collect(),
                )])
            })
            .collect()
    }

    fn defaulted_destructured_source_types(
        source_types: &[Vec<HirType>],
        default_type: &HirType,
    ) -> Result<Vec<Vec<HirType>>, String> {
        let mut default_members = Vec::new();
        Self::flatten_property_union_members(default_type, &mut default_members)?;
        source_types
            .iter()
            .map(|source_group| {
                let mut input_members = Vec::new();
                for source_type in source_group {
                    Self::flatten_property_union_members(source_type, &mut input_members)?;
                }
                let mut output = Vec::new();
                for member in input_members {
                    if member == HirType::Undefined {
                        for default in &default_members {
                            if !output.contains(default) {
                                output.push(default.clone());
                            }
                        }
                    } else if !output.contains(&member) {
                        output.push(member);
                    }
                }
                Ok(output)
            })
            .collect()
    }

    fn nested_destructured_property_source_types(
        source_types: &[Vec<HirType>],
        property: &str,
    ) -> Result<Vec<Vec<HirType>>, String> {
        source_types
            .iter()
            .map(|source_group| {
                let mut fields_for_source = Vec::new();
                for source in source_group {
                    let options = match source {
                        HirType::Union(elements)
                            if elements
                                .iter()
                                .all(|element| matches!(element, HirType::Object(_))) =>
                        {
                            elements.as_slice()
                        }
                        other => std::slice::from_ref(other),
                    };
                    for option in options {
                        let HirType::Object(fields) = option else {
                            return Err(format!(
                                "nested object pattern cannot destructure {option:?}"
                            ));
                        };
                        let field = fields
                            .iter()
                            .find(|(name, _)| name == property)
                            .map(|(_, ty)| ty.clone())
                            .ok_or_else(|| format!("object has no field `{property}`"))?;
                        if !fields_for_source.contains(&field) {
                            fields_for_source.push(field);
                        }
                    }
                }
                Ok(fields_for_source)
            })
            .collect()
    }

    fn nested_destructured_rest_source_types(
        source_types: &[Vec<HirType>],
        used: &BTreeSet<Symbol>,
    ) -> Result<Vec<Vec<HirType>>, String> {
        source_types
            .iter()
            .map(|source_group| {
                let mut rests = Vec::new();
                for source in source_group {
                    let options = match source {
                        HirType::Union(elements)
                            if elements
                                .iter()
                                .all(|element| matches!(element, HirType::Object(_))) =>
                        {
                            elements.as_slice()
                        }
                        other => std::slice::from_ref(other),
                    };
                    for option in options {
                        let HirType::Object(fields) = option else {
                            return Err(format!(
                                "nested object rest cannot destructure {option:?}"
                            ));
                        };
                        let rest = HirType::Object(
                            fields
                                .iter()
                                .filter(|(name, _)| !used.contains(name))
                                .cloned()
                                .collect(),
                        );
                        if !rests.contains(&rest) {
                            rests.push(rest);
                        }
                    }
                }
                Ok(rests)
            })
            .collect()
    }

    fn nested_destructured_index_source_types(
        source_types: &[Vec<HirType>],
        index: usize,
    ) -> Result<Vec<Vec<HirType>>, String> {
        source_types
            .iter()
            .map(|source_group| {
                let mut values = Vec::new();
                for source in source_group {
                    let options = match source {
                        HirType::Union(elements)
                            if elements
                                .iter()
                                .all(|element| matches!(element, HirType::Tuple(_))) =>
                        {
                            elements.as_slice()
                        }
                        other => std::slice::from_ref(other),
                    };
                    for option in options {
                        let HirType::Tuple(tuple) = option else {
                            return Err(format!(
                                "nested array pattern cannot destructure {option:?}"
                            ));
                        };
                        let value = tuple.get(index).cloned().ok_or_else(|| {
                            format!("tuple pattern index {index} is out of bounds")
                        })?;
                        if !values.contains(&value) {
                            values.push(value);
                        }
                    }
                }
                Ok(values)
            })
            .collect()
    }

    fn nested_destructured_tuple_rest_source_types(
        source_types: &[Vec<HirType>],
        start: usize,
    ) -> Result<Vec<Vec<HirType>>, String> {
        source_types
            .iter()
            .map(|source_group| {
                let mut rests = Vec::new();
                for source in source_group {
                    let options = match source {
                        HirType::Union(elements)
                            if elements
                                .iter()
                                .all(|element| matches!(element, HirType::Tuple(_))) =>
                        {
                            elements.as_slice()
                        }
                        other => std::slice::from_ref(other),
                    };
                    for option in options {
                        let HirType::Tuple(tuple) = option else {
                            return Err(format!("nested array rest cannot destructure {option:?}"));
                        };
                        if start > tuple.len() {
                            return Err(format!("tuple rest index {start} is out of bounds"));
                        }
                        let remaining = tuple[start..].to_vec();
                        let rest = if remaining
                            .first()
                            .is_some_and(|first| remaining.iter().all(|element| element == first))
                        {
                            HirType::Array(Box::new(
                                remaining.first().cloned().unwrap_or(HirType::F64),
                            ))
                        } else {
                            HirType::Tuple(remaining)
                        };
                        if !rests.contains(&rest) {
                            rests.push(rest);
                        }
                    }
                }
                Ok(rests)
            })
            .collect()
    }

    fn collect_correlated_destructured_bindings(
        &self,
        pattern: &Pat,
        source_types: &[Vec<HirType>],
        bindings: &mut Vec<CorrelatedDestructuredBinding>,
    ) -> Result<(), String> {
        match pattern {
            Pat::Ident(binding) => {
                let name = self.resolve_binding(binding.id.sym.as_ref());
                let ty = self
                    .scope
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| format!("unknown destructured binding `{name}`"))?;
                bindings.push(CorrelatedDestructuredBinding {
                    name,
                    ty,
                    source_types: source_types.to_vec(),
                });
            }
            Pat::Object(pattern) => {
                let mut used = BTreeSet::new();
                for property in &pattern.props {
                    match property {
                        ObjectPatProp::Assign(property) => {
                            let key = property.key.id.sym.to_string();
                            used.insert(key.clone());
                            if property.value.is_none() {
                                let nested = Self::nested_destructured_property_source_types(
                                    source_types,
                                    &key,
                                )?;
                                self.collect_correlated_destructured_bindings(
                                    &Pat::Ident(property.key.clone()),
                                    &nested,
                                    bindings,
                                )?;
                            }
                        }
                        ObjectPatProp::KeyValue(property) => {
                            let key = match &property.key {
                                PropName::Ident(key) => key.sym.to_string(),
                                PropName::Str(key) => key.value.to_string_lossy().into_owned(),
                                PropName::Computed(computed) => match computed.expr.as_ref() {
                                    Expr::Lit(Lit::Str(key)) => {
                                        key.value.to_string_lossy().into_owned()
                                    }
                                    _ => return Ok(()),
                                },
                                _ => return Ok(()),
                            };
                            used.insert(key.clone());
                            let nested = Self::nested_destructured_property_source_types(
                                source_types,
                                &key,
                            )?;
                            self.collect_correlated_destructured_bindings(
                                &property.value,
                                &nested,
                                bindings,
                            )?;
                        }
                        ObjectPatProp::Rest(rest) => {
                            let nested =
                                Self::nested_destructured_rest_source_types(source_types, &used)?;
                            self.collect_correlated_destructured_bindings(
                                &rest.arg, &nested, bindings,
                            )?;
                        }
                    }
                }
            }
            Pat::Array(pattern) => {
                for (index, element) in pattern.elems.iter().enumerate() {
                    let Some(element) = element else { continue };
                    if let Pat::Rest(rest) = element {
                        let nested =
                            Self::nested_destructured_tuple_rest_source_types(source_types, index)?;
                        self.collect_correlated_destructured_bindings(
                            &rest.arg, &nested, bindings,
                        )?;
                        break;
                    }
                    let nested = Self::nested_destructured_index_source_types(source_types, index)?;
                    self.collect_correlated_destructured_bindings(element, &nested, bindings)?;
                }
            }
            Pat::Assign(assign) => {
                let default_type = match assign.left.as_ref() {
                    Pat::Ident(binding) => self
                        .destructuring_default_types
                        .get(&self.resolve_binding(binding.id.sym.as_ref())),
                    _ => None,
                };
                if let Some(default_type) = default_type {
                    let transformed =
                        Self::defaulted_destructured_source_types(source_types, default_type)?;
                    self.collect_correlated_destructured_bindings(
                        &assign.left,
                        &transformed,
                        bindings,
                    )?;
                }
            }
            Pat::Rest(_) => {}
            _ => {}
        }
        Ok(())
    }

    fn register_destructured_union_correlations(
        &mut self,
        source_elements: &[HirType],
        discriminants: &HashMap<Symbol, Vec<Option<HirLit>>>,
        discriminant_bindings: &[(Symbol, Symbol)],
        extracted: &[CorrelatedDestructuredBinding],
    ) -> Result<(), String> {
        let targets = extracted
            .iter()
            .filter_map(|binding| {
                let HirType::Union(result_elements) = &binding.ty else {
                    return None;
                };
                let source_members = binding
                    .source_types
                    .iter()
                    .map(|source_types| {
                        let mut flattened = Vec::new();
                        for source_type in source_types {
                            Self::flatten_property_union_members(source_type, &mut flattened)?;
                        }
                        flattened
                            .iter()
                            .map(|member| {
                                result_elements
                                    .iter()
                                    .position(|result| result == member)
                                    .ok_or_else(|| {
                                        format!(
                                            "correlated binding `{}` lost union member {member:?}",
                                            binding.name
                                        )
                                    })
                            })
                            .collect::<Result<Vec<_>, String>>()
                    })
                    .collect::<Result<Vec<_>, String>>();
                Some(source_members.map(|source_members| CorrelatedUnionTarget {
                    name: binding.name.clone(),
                    elements: result_elements.clone(),
                    source_members,
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        if targets.is_empty() {
            return Ok(());
        }
        for (property, name) in discriminant_bindings {
            let Some(literals) = discriminants.get(property) else {
                continue;
            };
            if literals.len() == source_elements.len()
                && literals.iter().all(Option::is_some)
                && literals
                    .iter()
                    .skip(1)
                    .any(|literal| literal != &literals[0])
            {
                self.destructured_union_correlations.insert(
                    name.clone(),
                    DestructuredUnionCorrelation {
                        literals: literals.clone(),
                        targets: targets.clone(),
                    },
                );
            }
        }
        Ok(())
    }

    fn invalidate_destructured_union_correlation(&mut self, name: &str) {
        self.destructured_union_correlations.remove(name);
        for correlation in self.destructured_union_correlations.values_mut() {
            correlation.targets.retain(|target| target.name != name);
        }
    }

    fn propagate_destructured_union_alias(&mut self, source: &str, alias: &str) {
        if let Some(correlation) = self.destructured_union_correlations.get(source).cloned() {
            self.destructured_union_correlations
                .insert(alias.to_string(), correlation);
        }
        for correlation in self.destructured_union_correlations.values_mut() {
            let Some(source_target) = correlation
                .targets
                .iter()
                .find(|target| target.name == source)
                .cloned()
            else {
                continue;
            };
            if correlation
                .targets
                .iter()
                .any(|target| target.name == alias)
            {
                continue;
            }
            let Some(HirType::Union(alias_elements)) = self.scope.get(alias) else {
                continue;
            };
            let mut source_members = Vec::with_capacity(source_target.source_members.len());
            let mut compatible = true;
            for source_group in &source_target.source_members {
                let mut aliases = Vec::new();
                for source_index in source_group {
                    let source_type = &source_target.elements[*source_index];
                    let Some(alias_index) = alias_elements
                        .iter()
                        .position(|alias_type| alias_type == source_type)
                    else {
                        compatible = false;
                        break;
                    };
                    if !aliases.contains(&alias_index) {
                        aliases.push(alias_index);
                    }
                }
                source_members.push(aliases);
            }
            if compatible {
                correlation.targets.push(CorrelatedUnionTarget {
                    name: alias.to_string(),
                    elements: alias_elements.clone(),
                    source_members,
                });
            }
        }
    }

    fn lower_union_object_rest(
        &self,
        value: HirExpr,
        elements: &[HirType],
        used: &BTreeSet<Symbol>,
    ) -> Result<(HirExpr, HirType), String> {
        let mut rest_types = Vec::new();
        for element in elements {
            let HirType::Object(fields) = element else {
                return Err("object union rest requires object members".into());
            };
            let rest = HirType::Object(
                fields
                    .iter()
                    .filter(|(name, _)| !used.contains(name))
                    .cloned()
                    .collect(),
            );
            if !rest_types.contains(&rest) {
                rest_types.push(rest);
            }
        }
        let result_type = match rest_types.as_slice() {
            [rest] => rest.clone(),
            rests => HirType::Union(rests.to_vec()),
        };
        let parameter = "__thaw_union_rest_value".to_string();
        let mut body = Vec::new();
        for (index, element) in elements.iter().enumerate() {
            let HirType::Object(fields) = element else {
                unreachable!()
            };
            let member = HirExpr::UnionValue(
                Box::new(HirExpr::Var(parameter.clone())),
                index,
                elements.to_vec(),
            );
            let rest = HirExpr::ObjectLit(
                fields
                    .iter()
                    .filter(|(name, _)| !used.contains(name))
                    .map(|(name, _)| {
                        (
                            name.clone(),
                            HirExpr::PropAccess(
                                Box::new(member.clone()),
                                element.clone(),
                                name.clone(),
                            ),
                        )
                    })
                    .collect(),
            );
            let rest = self.coerce_to_declared(&result_type, rest)?;
            if index + 1 == elements.len() {
                body.push(HirStmt::Return(Some(rest)));
            } else {
                body.push(HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(
                            Box::new(HirExpr::Var(parameter.clone())),
                            elements.to_vec(),
                        )),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                    ),
                    vec![HirStmt::Return(Some(rest))],
                    Vec::new(),
                ));
            }
        }
        Ok((
            HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: parameter,
                        ty: HirType::Union(elements.to_vec()),
                    }],
                    result_type.clone(),
                    Box::new(HirExpr::Block(body)),
                )),
                vec![value],
            ),
            result_type,
        ))
    }

    fn lower_dictionary_object_binding_pattern(
        &mut self,
        pattern: &swc_ecma_ast::ObjectPat,
        value: HirExpr,
        element: &HirType,
        statements: &mut Vec<HirStmt>,
    ) -> Result<(), String> {
        let has_rest = pattern
            .props
            .iter()
            .any(|property| matches!(property, ObjectPatProp::Rest(_)));
        let mut used_keys = Vec::new();
        for property in &pattern.props {
            match property {
                ObjectPatProp::Assign(property) => {
                    let key = HirExpr::Lit(HirLit::Str(property.key.id.sym.to_string()));
                    used_keys.push(key.clone());
                    let field_json =
                        HirExpr::JsonKey(Box::new(value.clone()), Box::new(key.clone()));
                    let mut field = Self::dictionary_element_from_json(field_json, element)?;
                    if let Some(default) = &property.value {
                        let default = self.lower_expr(default)?;
                        let default = self.coerce_to_declared(element, default)?;
                        let present = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_has_own".into())),
                            vec![value.clone(), key],
                        );
                        field = self.lower_dictionary_default(
                            present,
                            field,
                            default,
                            element,
                        );
                    }
                    self.lower_binding_pattern(
                        &Pat::Ident(property.key.clone()),
                        field,
                        element,
                        statements,
                    )?;
                }
                ObjectPatProp::KeyValue(property) => {
                    let key = match &property.key {
                        PropName::Ident(key) => HirExpr::Lit(HirLit::Str(key.sym.to_string())),
                        PropName::Str(key) => HirExpr::Lit(HirLit::Str(
                            key.value.to_string_lossy().into_owned(),
                        )),
                        PropName::Num(key) => {
                            self.coerce_primitive_to_string(HirExpr::Lit(HirLit::F64(key.value)))?
                        }
                        PropName::Computed(computed) => {
                            let key = self.lower_expr(&computed.expr)?;
                            self.coerce_primitive_to_string(key)?
                        }
                        _ => return Err("unsupported dictionary destructuring key".into()),
                    };
                    let key = if has_rest {
                        let name = format!("__thaw_destructure_key_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::Str);
                        statements.push(HirStmt::Let(name.clone(), HirType::Str, key));
                        HirExpr::Var(name)
                    } else {
                        key
                    };
                    used_keys.push(key.clone());
                    let field = HirExpr::JsonKey(Box::new(value.clone()), Box::new(key));
                    let field = Self::dictionary_element_from_json(field, element)?;
                    self.lower_binding_pattern(
                        &property.value,
                        field,
                        element,
                        statements,
                    )?;
                }
                ObjectPatProp::Rest(rest) => {
                    let rest_type = HirType::Dictionary(Box::new(element.clone()));
                    let rest_value = self.lower_dictionary_object_rest(
                        value.clone(),
                        &rest_type,
                        &used_keys,
                    );
                    self.lower_binding_pattern(
                        &rest.arg,
                        rest_value,
                        &rest_type,
                        statements,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn dictionary_element_from_json(
        value: HirExpr,
        element: &HirType,
    ) -> Result<HirExpr, String> {
        match element {
            HirType::F64 => Ok(HirExpr::JsonAsNumber(Box::new(value))),
            HirType::Str => Ok(HirExpr::JsonAsString(Box::new(value))),
            HirType::Bool => Ok(HirExpr::JsonAsBool(Box::new(value))),
            HirType::Json => Ok(value),
            HirType::Object(_) | HirType::Array(_) | HirType::Tuple(_) => {
                Ok(HirExpr::JsonAsNative(Box::new(value), element.clone()))
            }
            other => Err(format!(
                "dictionary destructuring does not support element type {other:?}"
            )),
        }
    }

    fn lower_dictionary_default(
        &self,
        present: HirExpr,
        value: HirExpr,
        default: HirExpr,
        result_type: &HirType,
    ) -> HirExpr {
        let body = HirExpr::Block(vec![HirStmt::If(
            present,
            vec![HirStmt::Return(Some(value))],
            vec![HirStmt::Return(Some(default))],
        )]);
        let mut referenced = BTreeSet::new();
        collect_referenced_bindings(&body, &mut referenced);
        let captures = referenced
            .into_iter()
            .filter_map(|name| self.scope.get(&name).cloned().map(|ty| HirParam { name, ty }))
            .collect();
        HirExpr::Call(
            Box::new(HirExpr::Lambda(
                captures,
                Vec::new(),
                result_type.clone(),
                Box::new(body),
            )),
            Vec::new(),
        )
    }

    fn lower_dictionary_object_rest(
        &mut self,
        value: HirExpr,
        dictionary_type: &HirType,
        keys: &[HirExpr],
    ) -> HirExpr {
        let HirType::Dictionary(element) = dictionary_type else {
            unreachable!()
        };
        let copy = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_json_object_assign".into())),
            vec![
                HirExpr::JsonObjectLit(Vec::new(), element.as_ref().clone()),
                value,
            ],
        );
        let copy_name = format!("__thaw_object_rest_copy_{}", self.next_binding);
        self.next_binding += 1;
        let mut parameters = vec![HirParam {
            name: copy_name.clone(),
            ty: dictionary_type.clone(),
        }];
        let mut arguments = vec![copy];
        let mut body = Vec::with_capacity(keys.len() + 1);
        for key in keys {
            let key_name = format!("__thaw_object_rest_key_{}", self.next_binding);
            self.next_binding += 1;
            parameters.push(HirParam {
                name: key_name.clone(),
                ty: HirType::Str,
            });
            arguments.push(key.clone());
            body.push(HirStmt::Expr(HirExpr::JsonDelete(
                Box::new(HirExpr::Var(copy_name.clone())),
                Box::new(HirExpr::Var(key_name)),
            )));
        }
        body.push(HirStmt::Return(Some(HirExpr::Var(copy_name))));
        HirExpr::Call(
            Box::new(HirExpr::Lambda(
                Vec::new(),
                parameters,
                dictionary_type.clone(),
                Box::new(HirExpr::Block(body)),
            )),
            arguments,
        )
    }

}
