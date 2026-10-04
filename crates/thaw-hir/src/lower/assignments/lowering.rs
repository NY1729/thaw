impl<'a> FnLowerer<'a> {
    fn lower_assignment_target_write(
        &mut self,
        target: Target,
        value: HirExpr,
    ) -> Result<HirExpr, String> {
        let mut bindings = Vec::new();
        // An object-literal setter stores a hidden `__thaw_setter_<field>`
        // closure; a write dispatches to it (receiver + value) instead of
        // storing the field.
        if let Target::Prop(object, HirType::Object(fields), field) = &target {
            let setter = format!("__thaw_setter_{field}");
            if let Some((_, setter_type)) = fields.iter().find(|(name, _)| name == &setter) {
                let parameter_type = match setter_type {
                    HirType::Function(params, _) => params.last().cloned(),
                    HirType::CallableFunction(params, _, _, _) => params.last().cloned(),
                    _ => None,
                }
                .ok_or_else(|| format!("object setter `{field}` has no value parameter"))?;
                let value_type = self.infer_expr_type(&value)?;
                let object_type = HirType::Object(fields.clone());
                let receiver_name = format!("__thaw_setter_receiver_{}", self.next_binding);
                self.next_binding += 1;
                let value_name = format!("__thaw_setter_value_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(receiver_name.clone(), object_type.clone());
                self.scope.insert(value_name.clone(), value_type.clone());
                bindings.push((receiver_name.clone(), object_type.clone(), object.clone()));
                bindings.push((value_name.clone(), value_type, value));
                let receiver = HirExpr::Var(receiver_name);
                let value = HirExpr::Var(value_name);
                let argument = self.coerce_to_declared(&parameter_type, value.clone())?;
                let setter_value = HirExpr::PropAccess(
                    Box::new(receiver.clone()), object_type, setter,
                );
                let result = HirExpr::Block(vec![
                    HirStmt::Expr(HirExpr::Call(
                        Box::new(setter_value), vec![receiver, argument],
                    )),
                    HirStmt::Return(Some(value)),
                ]);
                return self.wrap_call_argument_bindings(result, &bindings);
            }
        }
        Ok(build_assign(target, value))
    }

    fn lower_array_index_operand(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let HirExpr::TypedIndex(source, offset, element) = value else {
            return Ok(value);
        };
        if matches!(element, HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_) | HirType::Union(_) | HirType::Undefined) {
            return Ok(HirExpr::TypedIndex(source, offset, element));
        }
        let source_type = self.infer_expr_type(&source)?;
        if matches!(source_type, HirType::Array(_)) {
            self.lower_array_index(*source, source_type, element, *offset)
        } else {
            Ok(HirExpr::TypedIndex(source, offset, element))
        }
    }

    fn lower_assignment_target_read(&mut self, target: &Target) -> Result<HirExpr, String> {
        if let Target::Prop(object, HirType::Object(fields), property) = target {
            return self.lower_fixed_object_property_read(object.clone(), fields, property);
        }
        let Target::Index(array, index) = target else {
            return target_to_read_expr(target);
        };
        let array_type = self.infer_expr_type(array)?;
        let HirType::Array(element) = &array_type else {
            return Err("index assignment target is not an array".into());
        };
        self.lower_array_index(
            array.clone(), array_type.clone(), element.as_ref().clone(), index.as_ref().clone(),
        )
    }

    fn lower_optional_index_assignment(
        &mut self,
        array: HirExpr,
        index: HirExpr,
        value: HirExpr,
        element: HirType,
    ) -> Result<HirExpr, String> {
        let array_name = format!("__thaw_assign_array_{}", self.next_binding);
        let index_name = format!("__thaw_assign_index_{}", self.next_binding);
        let value_name = format!("__thaw_assign_value_{}", self.next_binding);
        self.next_binding += 1;
        let array_type = HirType::Array(Box::new(element.clone()));
        let optional_type = HirType::Optional(Box::new(element.clone()));
        self.scope.insert(array_name.clone(), array_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope.insert(value_name.clone(), optional_type.clone());
        let array_var = HirExpr::Var(array_name.clone());
        let index_var = HirExpr::Var(index_name.clone());
        let value_var = HirExpr::Var(value_name.clone());
        let result = HirExpr::Block(vec![HirStmt::If(
            HirExpr::OptionalIsNone(Box::new(value_var.clone()), element.clone()),
            vec![
                HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_array_set_undefined".into())),
                    vec![array_var.clone(), index_var.clone()],
                )),
                HirStmt::Return(Some(value_var.clone())),
            ],
            vec![
                HirStmt::Expr(HirExpr::IndexAssign(
                    Box::new(array_var),
                    Box::new(index_var),
                    Box::new(HirExpr::OptionalValue(Box::new(value_var.clone()), element)),
                )),
                HirStmt::Return(Some(value_var)),
            ],
        )]);
        self.wrap_call_argument_bindings(
            result,
            &[
                (array_name, array_type, array),
                (index_name, HirType::F64, index),
                (value_name, optional_type, value),
            ],
        )
    }

    fn lower_array_length_write(
        &mut self,
        array: HirExpr,
        rhs: HirExpr,
    ) -> Result<HirExpr, String> {
        let array_type = self.infer_expr_type(&array)?;
        if let HirType::Union(members) = &array_type {
            if members.iter().all(|member| matches!(member, HirType::Array(_))) {
                let array_name = format!("__thaw_length_union_{}", self.next_binding);
                let value_name = format!("__thaw_length_union_value_{}", self.next_binding);
                self.next_binding += 1;
                let rhs_type = self.infer_expr_type(&rhs)?;
                self.scope.insert(array_name.clone(), array_type.clone());
                self.scope.insert(value_name.clone(), rhs_type.clone());
                let branches = members
                    .iter()
                    .enumerate()
                    .map(|(index, _)| {
                        self.lower_array_length_write(
                            HirExpr::UnionValue(
                                Box::new(HirExpr::Var(array_name.clone())),
                                index,
                                members.clone(),
                            ),
                            HirExpr::Var(value_name.clone()),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let mut statements = Vec::new();
                for (index, branch) in branches.into_iter().enumerate() {
                    let write = HirStmt::Expr(branch);
                    if index + 1 == members.len() {
                        statements.push(write);
                    } else {
                        statements.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(
                                    Box::new(HirExpr::Var(array_name.clone())),
                                    members.clone(),
                                )),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            vec![
                                write,
                                HirStmt::Return(Some(HirExpr::Var(value_name.clone()))),
                            ],
                            Vec::new(),
                        ));
                    }
                }
                statements.push(HirStmt::Return(Some(HirExpr::Var(value_name.clone()))));
                return self.wrap_call_argument_bindings(
                    HirExpr::Block(statements),
                    &[
                        (array_name, array_type, array),
                        (value_name, rhs_type, rhs),
                    ],
                );
            }
        }
        let rhs_type = self.infer_expr_type(&rhs)?;
        let array_name = format!("__thaw_length_array_{}", self.next_binding);
        let value_name = format!("__thaw_length_value_{}", self.next_binding);
        let number_name = format!("__thaw_length_number_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(array_name.clone(), array_type.clone());
        self.scope.insert(value_name.clone(), rhs_type.clone());
        self.scope.insert(number_name.clone(), HirType::F64);
        let value = HirExpr::Var(value_name.clone());
        let number = HirExpr::Var(number_name.clone());
        let invalid = || HirStmt::Throw(HirExpr::Lit(HirLit::Str(
            "\u{1}RangeError\u{1}Invalid array length".into(),
        )));
        let result = HirExpr::Block(vec![
            HirStmt::If(
                HirExpr::BinOp(BinOp::Lt, Box::new(number.clone()), Box::new(HirExpr::Lit(HirLit::F64(0.0)))),
                vec![invalid()], Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(BinOp::GtEq, Box::new(number.clone()), Box::new(HirExpr::Lit(HirLit::F64(4294967296.0)))),
                vec![invalid()], Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(BinOp::EqEqEq, Box::new(number.clone()), Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_math_trunc".into())), vec![number.clone()],
                ))),
                Vec::new(), vec![invalid()],
            ),
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_resize".into())),
                vec![HirExpr::Var(array_name.clone()), number],
            )),
            HirStmt::Return(Some(value)),
        ]);
        let numeric = self.coerce_primitive_to_number(HirExpr::Var(value_name.clone()))?;
        self.wrap_call_argument_bindings(result, &[
            (array_name, array_type, array),
            (value_name, rhs_type, rhs),
            (number_name, HirType::F64, numeric),
        ])
    }

    fn lower_array_length_read(
        &mut self,
        array: HirExpr,
        array_type: &HirType,
    ) -> Result<HirExpr, String> {
        match array_type {
            HirType::Array(_) => Ok(HirExpr::ArrayLen(Box::new(array))),
            HirType::Union(members)
                if members.iter().all(|member| matches!(member, HirType::Array(_))) =>
            {
                self.lower_union_array_length(array, members)
            }
            _ => Err(format!("cannot write `.length` on {array_type:?}")),
        }
    }

    fn lower_union_array_index_write(
        &mut self,
        array: HirExpr,
        members: &[HirType],
        index: HirExpr,
        value: HirExpr,
    ) -> Result<HirExpr, String> {
        let array_type = HirType::Union(members.to_vec());
        let value_type = self.infer_expr_type(&value)?;
        let array_name = format!("__thaw_union_write_array_{}", self.next_binding);
        let index_name = format!("__thaw_union_write_index_{}", self.next_binding);
        let value_name = format!("__thaw_union_write_value_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(array_name.clone(), array_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope.insert(value_name.clone(), value_type.clone());

        let mut statements = Vec::new();
        for (member_index, member) in members.iter().enumerate() {
            let HirType::Array(element) = member else { unreachable!() };
            let stored = self
                .coerce_to_declared(element, HirExpr::Var(value_name.clone()))
                .map_err(|_| {
                    format!(
                        "cannot write {value_type:?} through union array member {member:?}; the value must be representable by every member"
                    )
                })?;
            let write = HirStmt::Expr(HirExpr::IndexAssign(
                Box::new(HirExpr::UnionValue(
                    Box::new(HirExpr::Var(array_name.clone())),
                    member_index,
                    members.to_vec(),
                )),
                Box::new(HirExpr::Var(index_name.clone())),
                Box::new(stored),
            ));
            if member_index + 1 == members.len() {
                statements.push(write);
            } else {
                statements.push(HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(
                            Box::new(HirExpr::Var(array_name.clone())),
                            members.to_vec(),
                        )),
                        Box::new(HirExpr::Lit(HirLit::F64(member_index as f64))),
                    ),
                    vec![
                        write,
                        HirStmt::Return(Some(HirExpr::Var(value_name.clone()))),
                    ],
                    Vec::new(),
                ));
            }
        }
        statements.push(HirStmt::Return(Some(HirExpr::Var(value_name.clone()))));
        self.wrap_call_argument_bindings(
            HirExpr::Block(statements),
            &[
                (array_name, array_type, array),
                (index_name, HirType::F64, index),
                (value_name, value_type, value),
            ],
        )
    }

    fn ensure_union_array_write_type(
        &mut self,
        members: &[HirType],
        value_type: &HirType,
    ) -> Result<(), String> {
        for member in members {
            let HirType::Array(element) = member else { unreachable!() };
            if self
                .coerce_to_declared(
                    element,
                    HirExpr::TypedClosure(
                        value_type.clone(),
                        Box::new(HirExpr::Lit(HirLit::Undefined)),
                    ),
                )
                .is_err()
            {
                return Err(format!(
                    "cannot write {value_type:?} through union array member {member:?}; the value must be representable by every member"
                ));
            }
        }
        Ok(())
    }

    fn lower_union_array_numeric_read(
        &mut self,
        array_name: &str,
        members: &[HirType],
        index: HirExpr,
    ) -> Result<HirExpr, String> {
        let branches = members
            .iter()
            .enumerate()
            .map(|(member_index, member)| {
                let HirType::Array(element) = member else { unreachable!() };
                let value = self.lower_array_index(
                    HirExpr::UnionValue(
                        Box::new(HirExpr::Var(array_name.into())),
                        member_index,
                        members.to_vec(),
                    ),
                    member.clone(),
                    element.as_ref().clone(),
                    index.clone(),
                )?;
                self.coerce_primitive_to_number(value)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.merge_union_array_method_branches(array_name, members, branches)
    }

    fn lower_assign(&mut self, assign: &swc_ecma_ast::AssignExpr) -> Result<HirExpr, String> {
        let assigned_function_property = if assign.op == AssignOp::Assign {
            match &assign.left {
                AssignTarget::Simple(SimpleAssignTarget::Member(member)) => {
                    match (
                        self.expression_static_property_path(&member.obj),
                        member_property_name(&member.prop),
                    ) {
                        (Some((object, mut path)), Some(property)) => {
                            path.push(property);
                            Some((
                                object,
                                path,
                                self.expression_function_property_result_discriminants(
                                    &assign.right,
                                ),
                                self.expression_object_function_property_discriminants(
                                    &assign.right,
                                ),
                            ))
                        }
                        _ => None,
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        if self.unbound_this_context {
            if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left {
                if matches!(member.obj.as_ref(), Expr::This(_)) {
                    let property = member_property_name(&member.prop)
                        .ok_or("unbound `this` assignment requires a statically known property")?;
                    if assign.op != AssignOp::Assign {
                        return self.lower_unbound_this_member(&property);
                    }
                    let value = self.lower_expr(&assign.right)?;
                    let value_type = self.infer_expr_type(&value)?;
                    let value_name = format!("__thaw_unbound_assignment_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(value_name.clone(), value_type.clone());
                    let result = HirExpr::ThrowValue(
                        Box::new(HirExpr::Lit(HirLit::Str(format!(
                            "Cannot set properties of undefined (setting '{property}')"
                        )))),
                        Box::new(HirExpr::Var(value_name.clone())),
                    );
                    return self
                        .wrap_call_argument_bindings(result, &[(value_name, value_type, value)]);
                }
            }
        }
        if self.class_static_context {
            if let AssignTarget::Simple(SimpleAssignTarget::SuperProp(member)) = &assign.left {
                let (_, _, base_name) = self
                    .super_initializer
                    .clone()
                    .ok_or("`super` property assignment is only valid in a derived class")?;
                let property = super_property_name(&member.prop)?;
                let storage = class_static_field_symbol(&base_name, &property);
                if let Some(expected) = self.scope.get(&storage).cloned() {
                    if self.immutable_bindings.contains(&storage) {
                        return Err(format!(
                            "cannot assign to readonly static member `super.{property}`"
                        ));
                    }
                    let rhs = self.lower_expr(&assign.right)?;
                    let value = if assign.op == AssignOp::Assign {
                        rhs
                    } else if let Some(operator) = compound_op(assign.op) {
                        let current = HirExpr::Var(storage.clone());
                        if assign.op == AssignOp::AddAssign
                            && (expected == HirType::Str
                                || self.infer_expr_type(&rhs)? == HirType::Str)
                        {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                                vec![
                                    self.coerce_primitive_to_string(current)?,
                                    self.coerce_primitive_to_string(rhs)?,
                                ],
                            )
                        } else {
                            HirExpr::BinOp(operator, Box::new(current), Box::new(rhs))
                        }
                    } else {
                        return Err(format!(
                            "unsupported super static-field assignment operator {:?}",
                            assign.op
                        ));
                    };
                    let value = self.coerce_to_declared(&expected, value)?;
                    return Ok(HirExpr::Assign(storage, Box::new(value)));
                }
            }
        }
        if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left {
            let static_class = match member.obj.as_ref() {
                Expr::Ident(receiver) => Some(receiver.sym.to_string()),
                Expr::This(_) if self.class_static_context => self.class_context.clone(),
                _ => None,
            };
            if let (Some(receiver), Some(property)) =
                (static_class, member_property_name(&member.prop))
            {
                let getter = class_getter_symbol(&receiver, &property, true);
                let setter = class_setter_symbol(&receiver, &property, true);
                let has_getter = self.signatures.contains_key(&getter);
                let has_setter = self.signatures.contains_key(&setter);
                if has_getter && !has_setter {
                    return Err(format!(
                        "cannot assign to readonly static member `{}.{}`",
                        receiver, property
                    ));
                }
                if assign.op != AssignOp::Assign && has_setter {
                    let signature = self.signatures[&setter].clone();
                    let rhs = self.lower_expr(&assign.right)?;
                    let current = HirExpr::Call(Box::new(HirExpr::Var(getter)), Vec::new());
                    let value = if let Some(operator) = compound_op(assign.op) {
                        if assign.op == AssignOp::AddAssign
                            && (self.infer_expr_type(&current)? == HirType::Str
                                || self.infer_expr_type(&rhs)? == HirType::Str)
                        {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                                vec![
                                    self.coerce_primitive_to_string(current)?,
                                    self.coerce_primitive_to_string(rhs)?,
                                ],
                            )
                        } else {
                            HirExpr::BinOp(operator, Box::new(current), Box::new(rhs))
                        }
                    } else {
                        return Err(format!(
                            "unsupported inherited static-field assignment operator {:?}",
                            assign.op
                        ));
                    };
                    let value = self.coerce_to_declared(&signature.params[0], value)?;
                    return Ok(HirExpr::Call(Box::new(HirExpr::Var(setter)), vec![value]));
                }
            }
        }
        if assign.op == AssignOp::Assign {
            if let AssignTarget::Simple(SimpleAssignTarget::SuperProp(member)) = &assign.left {
                let (_, base_type, base_name) = self
                    .super_initializer
                    .clone()
                    .ok_or("`super` property assignment is only valid in a derived class")?;
                let property = super_property_name(&member.prop)?;
                let owner = if self.class_static_context {
                    base_name
                } else {
                    self.class_super_instance_accessor_owner(&base_type, &property)
                        .ok_or_else(|| format!("base class `{base_name}` has no accessor `{property}`"))?
                };
                let symbol = class_setter_symbol(&owner, &property, self.class_static_context);
                let signature = self.signatures.get(&symbol).cloned()
                    .filter(|signature| self.class_static_context || signature.accessor_owner.as_deref() == Some(owner.as_str()))
                    .ok_or_else(|| format!("base class `{owner}` has no setter `{property}`"))?;
                let rhs = self.lower_expr(&assign.right)?;
                let value_index = usize::from(!self.class_static_context);
                let rhs = self.coerce_to_declared(&signature.params[value_index], rhs)?;
                if self.class_static_context {
                    return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), vec![rhs]));
                }
                return self.call_class_instance_setter(
                    &owner, symbol, HirExpr::Var(self.resolve_binding("this")), rhs,
                );
            }
            if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left {
                if let (Expr::Ident(receiver), Some(property)) =
                    (member.obj.as_ref(), member_property_name(&member.prop))
                {
                    let static_symbol = class_setter_symbol(receiver.sym.as_ref(), &property, true);
                    if let Some(signature) = self.signatures.get(&static_symbol).cloned() {
                        let rhs = self.lower_expr(&assign.right)?;
                        let rhs = self.coerce_to_declared(&signature.params[0], rhs)?;
                        return Ok(HirExpr::Call(Box::new(HirExpr::Var(static_symbol)), vec![rhs]));
                    }
                    let binding = self.resolve_binding(receiver.sym.as_ref());
                    if let Some(owner) = self.scope.get(&binding)
                        .and_then(|ty| self.class_instance_accessor_owner(ty, &property))
                    {
                        let symbol = class_setter_symbol(&owner, &property, false);
                        if let Some(signature) = self.signatures.get(&symbol).cloned()
                            .filter(|signature| signature.accessor_owner.as_deref() == Some(owner.as_str()))
                        {
                            let rhs = self.lower_expr(&assign.right)?;
                            let rhs = self.coerce_to_declared(&signature.params[1], rhs)?;
                            return self.call_class_instance_setter(
                                &owner, symbol, HirExpr::Var(binding), rhs,
                            );
                        }
                        return Err(format!("cannot assign to readonly instance accessor `{owner}.{property}`"));
                    }
                }
                if let (Expr::This(_), Some(property)) =
                    (member.obj.as_ref(), member_property_name(&member.prop))
                {
                    if self.class_static_context {
                        let class = self
                            .class_context
                            .as_deref()
                            .expect("static setter retains its class context");
                        let symbol = class_setter_symbol(class, &property, true);
                        if let Some(signature) = self.signatures.get(&symbol).cloned() {
                            let rhs = self.lower_expr(&assign.right)?;
                            let rhs = self.coerce_to_declared(&signature.params[0], rhs)?;
                            return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), vec![rhs]));
                        }
                    }
                    let binding = self.resolve_binding("this");
                    if let Some(owner) = self.scope.get(&binding)
                        .and_then(|ty| self.class_instance_accessor_owner(ty, &property))
                    {
                        let symbol = class_setter_symbol(&owner, &property, false);
                        if let Some(signature) = self.signatures.get(&symbol).cloned()
                            .filter(|signature| signature.accessor_owner.as_deref() == Some(owner.as_str()))
                        {
                            let rhs = self.lower_expr(&assign.right)?;
                            let rhs = self.coerce_to_declared(&signature.params[1], rhs)?;
                            return self.call_class_instance_setter(
                                &owner, symbol, HirExpr::Var(binding), rhs,
                            );
                        }
                        return Err(format!("cannot assign to readonly instance accessor `{owner}.{property}`"));
                    }
                }
            }
        }
        if let AssignTarget::Pat(pattern) = &assign.left {
            if assign.op != AssignOp::Assign {
                return Err("destructuring only supports simple `=` assignment".into());
            }
            let pattern = match pattern {
                swc_ecma_ast::AssignTargetPat::Array(pattern) => Pat::Array(pattern.clone()),
                swc_ecma_ast::AssignTargetPat::Object(pattern) => Pat::Object(pattern.clone()),
                swc_ecma_ast::AssignTargetPat::Invalid(_) => {
                    return Err("invalid destructuring assignment target".into())
                }
            };
            let propagated_discriminants = self.expression_union_discriminants(&assign.right);
            let propagated_function_property_discriminants =
                self.expression_object_function_property_discriminants(&assign.right);
            let value = self.lower_expr(&assign.right)?;
            let ty = if matches!(pattern, Pat::Array(_)) {
                if let HirExpr::ArrayLit(elements) = &value {
                    HirType::Tuple(
                        elements
                            .iter()
                            .map(|element| self.infer_expr_type(element))
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                } else {
                    self.infer_expr_type(&value)?
                }
            } else {
                self.infer_expr_type(&value)?
            };
            let destructurable_union = matches!(
                &ty,
                HirType::Union(elements)
                    if (matches!(pattern, Pat::Object(_))
                        && elements.iter().all(|element| matches!(element, HirType::Object(_))))
                        || (matches!(pattern, Pat::Array(_))
                            && elements.iter().all(|element| matches!(element, HirType::Tuple(_) | HirType::Array(_))))
            );
            let destructurable_dictionary =
                matches!((&pattern, &ty), (Pat::Object(_), HirType::Dictionary(_) | HirType::Json));
            if !matches!(ty, HirType::Object(_) | HirType::Tuple(_) | HirType::Array(_))
                && !destructurable_union
                && !destructurable_dictionary
                && ty != HirType::Json
            {
                return Err(format!(
                    "destructuring assignment requires a fixed-shape object, tuple, or destructurable union, got {ty:?}"
                ));
            }
            let temporary = format!("__thaw_destructure_assign_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(temporary.clone(), ty.clone());
            if let Some(discriminants) = propagated_discriminants {
                self.union_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            if let Some(discriminants) = propagated_function_property_discriminants {
                self.object_function_property_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            let mut statements = Vec::new();
            self.lower_assignment_pattern(
                &pattern,
                HirExpr::Var(temporary.clone()),
                &ty,
                &mut statements,
            )?;
            statements.push(HirStmt::Return(Some(HirExpr::Var(temporary.clone()))));
            return self.wrap_call_argument_bindings(
                HirExpr::Block(statements),
                &[(temporary, ty, value)],
            );
        }

        if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left {
            if member_property_name(&member.prop).as_deref() == Some("length") {
                let array = self.lower_expr(&member.obj)?;
                let array_type = self.infer_expr_type(&array)?;
                if matches!(&array_type, HirType::Array(_))
                    || matches!(&array_type, HirType::Union(members) if members.iter().all(|member| matches!(member, HirType::Array(_))))
                {
                    if assign.op == AssignOp::NullishAssign {
                        // `.length` is always a plain number, never
                        // null/undefined, so `??=` never assigns --
                        // matching real ECMAScript's short-circuit, this
                        // never evaluates the RHS at all (its side
                        // effects, if any, don't run).
                        return self.lower_array_length_read(array, &array_type);
                    }
                    if matches!(assign.op, AssignOp::AndAssign | AssignOp::OrAssign) {
                        let array_name = format!("__thaw_length_target_{}", self.next_binding);
                        let current_name = format!("__thaw_length_old_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(array_name.clone(), array_type.clone());
                        self.scope.insert(current_name.clone(), HirType::F64);
                        // Lowered only here, inside the branch that needs
                        // it -- matching real ECMAScript, `&&=`/`||=`
                        // evaluate the RHS only when they actually assign.
                        let rhs = self.lower_expr(&assign.right)?;
                        let assigned = self.lower_array_length_write(
                            HirExpr::Var(array_name.clone()),
                            rhs,
                        )?;
                        let current = HirExpr::Var(current_name.clone());
                        let condition = self.truthiness_expr(current.clone(), &HirType::F64)?;
                        let (then_branch, else_branch) = if assign.op == AssignOp::AndAssign {
                            (assigned, current)
                        } else {
                            (current, assigned)
                        };
                        let result = HirExpr::Block(vec![HirStmt::If(
                            condition,
                            vec![HirStmt::Return(Some(then_branch))],
                            vec![HirStmt::Return(Some(else_branch))],
                        )]);
                        let old_length = self.lower_array_length_read(
                            HirExpr::Var(array_name.clone()),
                            &array_type,
                        )?;
                        return self.wrap_call_argument_bindings(result, &[
                            (array_name, array_type, array),
                            (current_name, HirType::F64, old_length),
                        ]);
                    }
                    let rhs = self.lower_expr(&assign.right)?;
                    if assign.op == AssignOp::Assign {
                        return self.lower_array_length_write(array, rhs);
                    }
                    let Some(operator) = compound_op(assign.op) else {
                        return Err(format!("unsupported array length assignment operator {:?}", assign.op));
                    };
                    let array_name = format!("__thaw_length_target_{}", self.next_binding);
                    let current_name = format!("__thaw_length_old_{}", self.next_binding);
                    let rhs_name = format!("__thaw_length_rhs_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(array_name.clone(), array_type.clone());
                    self.scope.insert(current_name.clone(), HirType::F64);
                    let rhs_type = self.infer_expr_type(&rhs)?;
                    self.scope.insert(rhs_name.clone(), rhs_type.clone());
                    let old = HirExpr::Var(current_name.clone());
                    let value = HirExpr::Var(rhs_name.clone());
                    let updated = if assign.op == AssignOp::AddAssign && rhs_type == HirType::Str {
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".into())),
                            vec![self.coerce_primitive_to_string(old)?, value],
                        )
                    } else {
                        HirExpr::BinOp(operator,
                            Box::new(self.coerce_primitive_to_number(old)?),
                            Box::new(self.coerce_primitive_to_number(value)?),
                        )
                    };
                    let result = self.lower_array_length_write(HirExpr::Var(array_name.clone()), updated)?;
                    let old_length = self.lower_array_length_read(
                        HirExpr::Var(array_name.clone()),
                        &array_type,
                    )?;
                    return self.wrap_call_argument_bindings(result, &[
                        (array_name, array_type, array),
                        (current_name, HirType::F64, old_length),
                        (rhs_name, rhs_type, rhs),
                    ]);
                }
            }
        }
        if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left {
            if let MemberProp::Computed(computed) = &member.prop {
                let array = self.lower_expr(&member.obj)?;
                if let HirType::Union(members) = self.infer_expr_type(&array)? {
                    if members.iter().all(|member| matches!(member, HirType::Array(_))) {
                        let index = self.lower_expr(&computed.expr)?;
                        self.expect_type(&HirType::F64, &index, "array index")?;
                        let rhs = self.lower_expr(&assign.right)?;
                        if assign.op == AssignOp::Assign {
                            return self.lower_union_array_index_write(
                                array, &members, index, rhs,
                            );
                        }
                        let Some(operator) = compound_op(assign.op) else {
                            return Err(format!(
                                "unsupported union array index assignment operator {:?}",
                                assign.op
                            ));
                        };
                        let array_name = format!("__thaw_union_assign_array_{}", self.next_binding);
                        let index_name = format!("__thaw_union_assign_index_{}", self.next_binding);
                        let old_name = format!("__thaw_union_assign_old_{}", self.next_binding);
                        let rhs_name = format!("__thaw_union_assign_rhs_{}", self.next_binding);
                        self.next_binding += 1;
                        let union_type = HirType::Union(members.clone());
                        self.scope.insert(array_name.clone(), union_type.clone());
                        self.scope.insert(index_name.clone(), HirType::F64);
                        let current = self.lower_union_array_numeric_read(
                            &array_name,
                            &members,
                            HirExpr::Var(index_name.clone()),
                        )?;
                        let current_type = self.infer_expr_type(&current)?;
                        self.scope.insert(old_name.clone(), current_type.clone());
                        let rhs_type = self.infer_expr_type(&rhs)?;
                        self.scope.insert(rhs_name.clone(), rhs_type.clone());
                        let result_type = if assign.op == AssignOp::AddAssign
                            && members.iter().all(|member| {
                                matches!(member, HirType::Array(element) if element.as_ref() == &HirType::Str)
                            })
                            && rhs_type == HirType::Str
                        {
                            HirType::Str
                        } else {
                            HirType::F64
                        };
                        self.ensure_union_array_write_type(&members, &result_type)?;
                        let old = HirExpr::Var(old_name.clone());
                        let rhs_value = HirExpr::Var(rhs_name.clone());
                        let updated = if assign.op == AssignOp::AddAssign
                            && (current_type == HirType::Str || rhs_type == HirType::Str)
                        {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_concat".into())),
                                vec![
                                    self.coerce_primitive_to_string(old)?,
                                    self.coerce_primitive_to_string(rhs_value)?,
                                ],
                            )
                        } else {
                            HirExpr::BinOp(
                                operator,
                                Box::new(self.coerce_primitive_to_number(old)?),
                                Box::new(self.coerce_primitive_to_number(rhs_value)?),
                            )
                        };
                        let write = self.lower_union_array_index_write(
                            HirExpr::Var(array_name.clone()),
                            &members,
                            HirExpr::Var(index_name.clone()),
                            updated,
                        )?;
                        return self.wrap_call_argument_bindings(
                            write,
                            &[
                                (array_name, union_type, array),
                                (index_name, HirType::F64, index),
                                (old_name, current_type, current),
                                (rhs_name, rhs_type, rhs),
                            ],
                        );
                    }
                }
            }
        }
        let mut target = self.lower_assign_target(&assign.left)?;
        if let Target::Var(name) = &target {
            if self.immutable_bindings.contains(name) {
                return Err(format!("cannot assign to constant `{name}`"));
            }
        }
        let rhs = self.lower_expr(&assign.right)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        if let Target::DynamicProperty(object, key) = &target {
            if assign.op != AssignOp::Assign {
                return Err("dynamic properties currently support simple `=` assignment only".into());
            }
            let object_name = format!("__thaw_dynamic_assign_object_{}", self.next_binding);
            self.next_binding += 1;
            let key_name = format!("__thaw_dynamic_assign_key_{}", self.next_binding);
            self.next_binding += 1;
            let value_name = format!("__thaw_dynamic_assign_value_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(object_name.clone(), HirType::JsValue);
            let key_type = self.infer_expr_type(key)?;
            self.scope.insert(key_name.clone(), key_type.clone());
            self.scope.insert(value_name.clone(), rhs_type.clone());
            let encoded = self.coerce_to_declared(
                &HirType::Json,
                HirExpr::Var(value_name.clone()),
            )?;
            let result = HirExpr::Block(vec![
                HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("setDynamicPropertyJson".into())),
                    vec![
                        HirExpr::Var(object_name.clone()),
                        HirExpr::Var(key_name.clone()),
                        encoded,
                    ],
                )),
                HirStmt::Return(Some(HirExpr::Var(value_name.clone()))),
            ]);
            return self.wrap_call_argument_bindings(
                result,
                &[
                    (object_name, HirType::JsValue, object.clone()),
                    (key_name, key_type, key.as_ref().clone()),
                    (value_name, rhs_type, rhs),
                ],
            );
        }
        let assigned_variable = match &target {
            Target::Var(name) => Some(name.clone()),
            _ => None,
        };
        let assigned_method_value = (assign.op == AssignOp::Assign)
            .then(|| {
                self.native_instance_method_value(&assign.right)
                    .or_else(|| {
                        let Expr::Ident(identifier) = assign.right.as_ref() else {
                            return None;
                        };
                        self.native_method_values
                            .get(&self.resolve_binding(identifier.sym.as_ref()))
                            .cloned()
                    })
            })
            .flatten();
        let assigned_function_metadata =
            (assign.op == AssignOp::Assign && assigned_variable.is_some()).then(|| {
                (
                    self.expression_function_discriminants(&assign.right),
                    self.expression_function_array_discriminants(&assign.right),
                    self.expression_function_nested_array_discriminants(&assign.right),
                    self.expression_function_object_array_property_discriminants(&assign.right),
                    self.expression_function_object_function_property_discriminants(&assign.right),
                )
            });
        let mut bindings = Vec::new();
        if let Some(name) = assigned_variable.as_ref() {
            self.record_binding_write(name);
        }

        if assign.op != AssignOp::Assign {
            target = match target {
                Target::Var(name) => Target::Var(name),
                Target::Index(array, index) => {
                    let array_name = format!("__thaw_assign_array_{}", self.next_binding);
                    self.next_binding += 1;
                    let array_type = self.infer_expr_type(&array)?;
                    self.scope.insert(array_name.clone(), array_type.clone());
                    bindings.push((array_name.clone(), array_type, array));

                    let index_name = format!("__thaw_assign_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(index_name.clone(), HirType::F64);
                    bindings.push((index_name.clone(), HirType::F64, *index));
                    Target::Index(HirExpr::Var(array_name), Box::new(HirExpr::Var(index_name)))
                }
                Target::Prop(object, object_type, field) => {
                    let object_name = format!("__thaw_assign_object_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(object_name.clone(), object_type.clone());
                    bindings.push((object_name.clone(), object_type.clone(), object));
                    Target::Prop(HirExpr::Var(object_name), object_type, field)
                }
                Target::Dictionary(object, key, element) => {
                    let object_name = format!("__thaw_assign_dictionary_{}", self.next_binding);
                    self.next_binding += 1;
                    let object_type = HirType::Dictionary(Box::new(element.clone()));
                    self.scope.insert(object_name.clone(), object_type.clone());
                    bindings.push((object_name.clone(), object_type, object));

                    let key_name = format!("__thaw_assign_key_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(key_name.clone(), HirType::Str);
                    bindings.push((key_name.clone(), HirType::Str, *key));
                    Target::Dictionary(
                        HirExpr::Var(object_name),
                        Box::new(HirExpr::Var(key_name)),
                        element,
                    )
                }
                Target::JsonIndex(object, index) => {
                    let object_name = format!("__thaw_assign_json_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(object_name.clone(), HirType::Json);
                    bindings.push((object_name.clone(), HirType::Json, object));

                    let index_name = format!("__thaw_assign_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(index_name.clone(), HirType::F64);
                    bindings.push((index_name.clone(), HirType::F64, *index));
                    Target::JsonIndex(
                        HirExpr::Var(object_name),
                        Box::new(HirExpr::Var(index_name)),
                    )
                }
                Target::DynamicProperty(_, _) => unreachable!(),
            };
        }

        if matches!(assign.op, AssignOp::NullishAssign | AssignOp::AndAssign | AssignOp::OrAssign)
            && matches!(&target, Target::Prop(_, HirType::Object(fields), field)
                if fields.iter().any(|(name, _)| name == &format!("__thaw_setter_{field}")))
        {
            let current = self.lower_assignment_target_read(&target)?;
            let current_type = self.infer_expr_type(&current)?;
            if matches!(current_type, HirType::Undefined | HirType::Null) {
                let current_name = format!("__thaw_setter_absent_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(current_name.clone(), current_type.clone());
                bindings.push((current_name.clone(), current_type, current));
                let result = if assign.op == AssignOp::AndAssign {
                    HirExpr::Var(current_name)
                } else {
                    self.lower_assignment_target_write(target, rhs)?
                };
                return self.wrap_call_argument_bindings(result, &bindings);
            }
        }

        if assign.op == AssignOp::NullishAssign {
            let current = self.lower_assignment_target_read(&target)?;
            let current_type = self.infer_expr_type(&current)?;
            // `x ??= y` where `x` is a dynamic (`any`-typed) target --
            // unlike `Optional`/`Nullable`/`Nullish` below, whose absent
            // state is a compile-time-known tag, a `Json` value's
            // nullish-ness is only knowable at runtime
            // (`__thaw_json_is_nullish`, the same check `??`'s own
            // lowering uses). Previously fell straight through to this
            // match's `_` arm, silently never assigning even when `x`
            // was really `null`/`undefined` (real trigger: `let m: any =
            // null; m ??= "default";` leaving `m` as `null`).
            //
            // Scoped to a plain `Json`-typed target (a variable or
            // `.field`) -- an `any[]` array element read as an
            // assignment target comes back `Optional<Json>` instead (the
            // conservative-sparse-array tagging every array read target
            // gets), a different, untagged-here combination; still
            // silently no-ops for `arr[i] ??= y`.
            if current_type == HirType::Json {
                let rhs = self.coerce_to_declared(&HirType::Json, rhs)?;
                let current_name = format!("__thaw_nullish_assign_current_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(current_name.clone(), HirType::Json);
                let rhs_name = format!("__thaw_nullish_assign_rhs_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(rhs_name.clone(), HirType::Json);
                let assigned = HirExpr::Block(vec![
                    HirStmt::Expr(self.lower_assignment_target_write(target, HirExpr::Var(rhs_name.clone()))?),
                    HirStmt::Return(Some(HirExpr::Var(rhs_name.clone()))),
                ]);
                let assigned = self
                    .wrap_call_argument_bindings(assigned, &[(rhs_name, HirType::Json, rhs)])?;
                let current_value = HirExpr::Var(current_name.clone());
                let is_nullish = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_is_nullish".to_string())),
                    vec![current_value.clone()],
                );
                let result = HirExpr::Block(vec![HirStmt::If(
                    is_nullish,
                    vec![HirStmt::Return(Some(assigned))],
                    vec![HirStmt::Return(Some(current_value))],
                )]);
                bindings.push((current_name, HirType::Json, current));
                return self.wrap_call_argument_bindings(result, &bindings);
            }
            let (payload, absence_kind) = match current_type.clone() {
                HirType::Optional(payload) => (payload, 0),
                HirType::Nullable(payload) => (payload, 1),
                HirType::Nullish(payload) => (payload, 2),
                _ => return self.wrap_call_argument_bindings(current, &bindings),
            };
            if let Target::Index(array, index) = &target {
                if self.infer_expr_type(array)? == HirType::Array(payload.clone()) {
                    let indexed_rhs = self.lower_array_index_operand(rhs.clone())?;
                    if self.infer_expr_type(&indexed_rhs)? == current_type {
                        let current_name = format!("__thaw_nullish_assign_current_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(current_name.clone(), current_type.clone());
                        let value = HirExpr::Var(current_name.clone());
                        let assigned = self.lower_optional_index_assignment(
                            array.clone(), index.as_ref().clone(), indexed_rhs, payload.as_ref().clone(),
                        )?;
                        // A present (non-hole) `any[]` slot can itself
                        // hold a JS-nullish value (`null`/`undefined`
                        // written explicitly, not a never-set hole) --
                        // `OptionalIsNone` only sees the hole/presence
                        // bit, so `arr[i] ??= y` previously no-op'd for
                        // `arr[i] === null`. Check the loaded Json value
                        // itself too when present, same runtime check
                        // `??`'s own Json branch uses.
                        let else_branch = if payload.as_ref() == &HirType::Json {
                            let inner_nullish = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_is_nullish".to_string())),
                                vec![HirExpr::OptionalValue(
                                    Box::new(value.clone()),
                                    payload.as_ref().clone(),
                                )],
                            );
                            vec![HirStmt::If(
                                inner_nullish,
                                vec![HirStmt::Return(Some(assigned.clone()))],
                                vec![HirStmt::Return(Some(value.clone()))],
                            )]
                        } else {
                            vec![HirStmt::Return(Some(value.clone()))]
                        };
                        let result = HirExpr::Block(vec![HirStmt::If(
                            HirExpr::OptionalIsNone(Box::new(value), payload.as_ref().clone()),
                            vec![HirStmt::Return(Some(assigned))],
                            else_branch,
                        )]);
                        bindings.push((current_name, current_type, current));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                }
            }
            let rhs = self.coerce_to_declared(payload.as_ref(), rhs)?;
            let current_name = format!("__thaw_nullish_assign_current_{}", self.next_binding);
            self.next_binding += 1;
            self.scope
                .insert(current_name.clone(), current_type.clone());
            let rhs_name = format!("__thaw_nullish_assign_rhs_{}", self.next_binding);
            self.next_binding += 1;
            self.scope
                .insert(rhs_name.clone(), payload.as_ref().clone());

            let direct_index = if let Target::Index(array, _) = &target {
                self.infer_expr_type(array)? == HirType::Array(payload.clone())
            } else {
                false
            };
            let stored = if direct_index {
                HirExpr::Var(rhs_name.clone())
            } else { match absence_kind {
                0 => HirExpr::OptionalSome(
                    Box::new(HirExpr::Var(rhs_name.clone())),
                    payload.as_ref().clone(),
                ),
                1 => HirExpr::NullableSome(
                    Box::new(HirExpr::Var(rhs_name.clone())),
                    payload.as_ref().clone(),
                ),
                2 => HirExpr::NullishSome(
                    Box::new(HirExpr::Var(rhs_name.clone())),
                    payload.as_ref().clone(),
                ),
                _ => unreachable!(),
            }};
            let assigned = HirExpr::Block(vec![
                HirStmt::Expr(self.lower_assignment_target_write(target, stored)?),
                HirStmt::Return(Some(HirExpr::Var(rhs_name.clone()))),
            ]);
            let assigned = self.wrap_call_argument_bindings(
                assigned,
                &[(rhs_name, payload.as_ref().clone(), rhs)],
            )?;
            let current_value = HirExpr::Var(current_name.clone());
            let is_none = match absence_kind {
                0 => HirExpr::OptionalIsNone(
                    Box::new(current_value.clone()),
                    payload.as_ref().clone(),
                ),
                1 => HirExpr::NullableIsNone(
                    Box::new(current_value.clone()),
                    payload.as_ref().clone(),
                ),
                2 => HirExpr::NullishIsNone(
                    Box::new(current_value.clone()),
                    payload.as_ref().clone(),
                ),
                _ => unreachable!(),
            };
            let present = match absence_kind {
                0 => HirExpr::OptionalValue(Box::new(current_value), payload.as_ref().clone()),
                1 => HirExpr::NullableValue(Box::new(current_value), payload.as_ref().clone()),
                2 => HirExpr::NullishValue(Box::new(current_value), payload.as_ref().clone()),
                _ => unreachable!(),
            };
            // Same "a present slot can itself hold a nullish Json value"
            // gap as the `TypedIndex`-rhs special case above, reached
            // here for the far more common shape (`arr[i] ??= <plain
            // value>` -- `lower_array_index_operand` only transforms an
            // rhs that's itself an array read, so any other rhs, e.g. a
            // literal, falls through to this general path instead).
            let else_branch = if payload.as_ref() == &HirType::Json {
                let inner_nullish = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_is_nullish".to_string())),
                    vec![present.clone()],
                );
                vec![HirStmt::If(
                    inner_nullish,
                    vec![HirStmt::Return(Some(assigned.clone()))],
                    vec![HirStmt::Return(Some(present))],
                )]
            } else {
                vec![HirStmt::Return(Some(present))]
            };
            let result = HirExpr::Block(vec![HirStmt::If(
                is_none,
                vec![HirStmt::Return(Some(assigned))],
                else_branch,
            )]);
            bindings.push((current_name, current_type, current));
            if let Some(name) = assigned_variable {
                if absence_kind == 1 {
                    self.nullable_narrowings
                        .insert(name, payload.as_ref().clone());
                } else if absence_kind == 0 {
                    self.narrowings.insert(name, payload.as_ref().clone());
                } else if absence_kind == 2 {
                    self.nullish_narrowings
                        .insert(name, payload.as_ref().clone());
                }
            }
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        if matches!(assign.op, AssignOp::AndAssign | AssignOp::OrAssign) {
            let current = self.lower_assignment_target_read(&target)?;
            let current_type = self.infer_expr_type(&current)?;
            let rhs = if matches!(target, Target::Index(_, _)) {
                self.lower_array_index_operand(rhs)?
            } else {
                rhs
            };
            let rhs = self.coerce_to_declared(&current_type, rhs)?;
            let current_name = format!("__thaw_logical_assign_current_{}", self.next_binding);
            self.next_binding += 1;
            self.scope
                .insert(current_name.clone(), current_type.clone());
            let rhs_name = format!("__thaw_logical_assign_rhs_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(rhs_name.clone(), current_type.clone());
            let assigned = if let Target::Index(array, index) = &target {
                let array_type = self.infer_expr_type(array)?;
                if let HirType::Array(element) = array_type {
                    if current_type == HirType::Optional(element.clone()) {
                        self.lower_optional_index_assignment(
                            array.clone(), index.as_ref().clone(),
                            HirExpr::Var(rhs_name.clone()), element.as_ref().clone(),
                        )?
                    } else {
                        HirExpr::Block(vec![
                            HirStmt::Expr(self.lower_assignment_target_write(target, HirExpr::Var(rhs_name.clone()))?),
                            HirStmt::Return(Some(HirExpr::Var(rhs_name.clone()))),
                        ])
                    }
                } else {
                    return Err("index assignment target is not an array".into());
                }
            } else {
                HirExpr::Block(vec![
                    HirStmt::Expr(self.lower_assignment_target_write(target, HirExpr::Var(rhs_name.clone()))?),
                    HirStmt::Return(Some(HirExpr::Var(rhs_name.clone()))),
                ])
            };
            let assigned = self.wrap_call_argument_bindings(
                assigned,
                &[(rhs_name, current_type.clone(), rhs)],
            )?;
            let present = HirExpr::Var(current_name.clone());
            let condition = self.truthiness_expr(present.clone(), &current_type)?;
            let (then_branch, else_branch) = if assign.op == AssignOp::AndAssign {
                (assigned, present)
            } else {
                (present, assigned)
            };
            let result = HirExpr::Block(vec![HirStmt::If(
                condition,
                vec![HirStmt::Return(Some(then_branch))],
                vec![HirStmt::Return(Some(else_branch))],
            )]);
            bindings.push((current_name, current_type, current));
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        let value = if assign.op == AssignOp::Assign {
            rhs
        } else if let Some(op) = compound_op(assign.op) {
            let current = self.lower_assignment_target_read(&target)?;
            let current_type = self.infer_expr_type(&current)?;
            let rhs = if matches!(target, Target::Index(_, _)) {
                self.lower_array_index_operand(rhs)?
            } else {
                rhs
            };
            if assign.op == AssignOp::AddAssign
                && self.infer_expr_type(&rhs)? == HirType::JsValue
                && (current_type == HirType::Json
                    || current_type == HirType::Optional(Box::new(HirType::Json)))
            {
                self.lower_add_with_live_json(current, rhs)?
            } else if assign.op == AssignOp::AddAssign
                && self.infer_expr_type(&rhs)? != HirType::JsValue
                && (current_type == HirType::Json
                    || current_type == HirType::Optional(Box::new(HirType::Json))
                    || self.infer_expr_type(&rhs)? == HirType::Json)
            {
                self.lower_add_with_to_primitive(current, rhs)?
            } else if assign.op == AssignOp::AddAssign
                && (current_type == HirType::Str
                    || current_type == HirType::Optional(Box::new(HirType::Str))
                    || self.infer_expr_type(&rhs)? == HirType::Str)
            {
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                    vec![
                        self.coerce_primitive_to_string(current)?,
                        self.coerce_primitive_to_string(rhs)?,
                    ],
                )
            } else {
                // Coerces *both* operands unconditionally, the same way
                // the ordinary (non-compound) arithmetic `BinOp` lowering
                // already does (`coerce_primitive_to_number` is a no-op
                // for an already-`F64` value) -- the previous version
                // only coerced `rhs` when `current` (the property being
                // read back) was already known to be `F64`, so a
                // `Json`-typed field (any `any`/`Json`-typed value,
                // ubiquitous in real code: `const draft: any = {...};
                // draft.count += 1`) hit "arithmetic requires F64
                // operands, got Json and F64" instead, since `current`
                // itself was the one needing conversion, not `rhs`.
                HirExpr::BinOp(
                    op,
                    Box::new(self.coerce_primitive_to_number(current)?),
                    Box::new(self.coerce_primitive_to_number(rhs)?),
                )
            }
        } else {
            return Err(format!(
                "unsupported compound assignment operator {:?}",
                assign.op
            ));
        };

        if assign.op == AssignOp::Assign {
            if let Target::Index(array, index) = &target {
                self.expect_type(&HirType::F64, index, "array index")?;
                if let HirType::Array(element) = self.infer_expr_type(array)? {
                    let value = self.lower_array_index_operand(value.clone())?;
                    if self.infer_expr_type(&value)? == HirType::Optional(element.clone()) {
                        return self.lower_optional_index_assignment(
                            array.clone(), index.as_ref().clone(), value, element.as_ref().clone(),
                        );
                    }
                }
            }
        }

        if let Target::Prop(_, HirType::Object(fields), field) = &target {
            if fields.iter().any(|(name, _)| name == &format!("__thaw_setter_{field}")) {
                let result = self.lower_assignment_target_write(target, value)?;
                return self.wrap_call_argument_bindings(result, &bindings);
            }
        }

        // Reorder/typecheck an object literal against the target's
        // declared shape, same as a `let`/call-argument assignment --
        // needed now that a field can itself be an object (`p.corner =
        // { y: 2, x: 1 }`), not just a plain variable.
        let value = match &target {
            Target::Var(name) => match self.scope.get(name).cloned() {
                Some(ty) if assign.op == AssignOp::Assign
                    && self.is_primitive_array_index(&assign.right, &ty) => {
                    self.coerce_primitive_array_argument(value, &ty)?
                }
                Some(ty) => self.coerce_to_declared(&ty, value)?,
                None => value,
            },
            Target::Prop(_, HirType::Object(fields), field) => {
                match fields.iter().find(|(n, _)| n == field) {
                    Some((_, ty)) if assign.op == AssignOp::Assign
                        && self.is_primitive_array_index(&assign.right, ty) => {
                        self.coerce_primitive_array_argument(value, ty)?
                    }
                    Some((_, ty)) => self.coerce_to_declared(ty, value)?,
                    None => value,
                }
            }
            Target::Index(array, index) => {
                self.expect_type(&HirType::F64, index, "array index")?;
                let HirType::Array(element) = self.infer_expr_type(array)? else {
                    return Err("index assignment target is not an array".into());
                };
                self.coerce_to_declared(&element, value)?
            }
            Target::Dictionary(_, _, element) => self.coerce_to_declared(element, value)?,
            Target::JsonIndex(_, index) => {
                self.expect_type(&HirType::F64, index, "JSON array index")?;
                self.coerce_to_declared(&HirType::Json, value)?
            }
            Target::DynamicProperty(_, _) => unreachable!(),
            Target::Prop(_, other, field) => {
                return Err(format!(
                    "cannot assign to field `{field}` on value of type {other:?}"
                ));
            }
        };

        let result = self.lower_assignment_target_write(target, value)?;
        if let Some((object, path, value, nested)) = assigned_function_property {
            let metadata = self
                .object_function_property_discriminants
                .entry(object.clone())
                .or_default();
            metadata.retain(|existing, _| !existing.starts_with(&path));
            if let Some(value) = value {
                metadata.insert(path.clone(), value);
            }
            if let Some(nested) = nested {
                metadata.extend(nested.into_iter().map(|(suffix, discriminants)| {
                    let mut nested_path = path.clone();
                    nested_path.extend(suffix);
                    (nested_path, discriminants)
                }));
            }
            if metadata.is_empty() {
                self.object_function_property_discriminants.remove(&object);
            }
        }
        if assign.op == AssignOp::Assign {
            if let Some(name) = assigned_variable.as_ref() {
                if let Some(method) = assigned_method_value {
                    self.native_method_values.insert(name.clone(), method);
                } else {
                    self.native_method_values.remove(name);
                }
                let (value, array, nested_array, object, functions) =
                    assigned_function_metadata.expect("simple assignment metadata was collected");
                replace_metadata(&mut self.function_value_discriminants, name, value);
                replace_metadata(&mut self.function_value_array_discriminants, name, array);
                replace_metadata(
                    &mut self.function_value_nested_array_discriminants,
                    name,
                    nested_array,
                );
                replace_metadata(
                    &mut self.function_value_object_array_property_discriminants,
                    name,
                    object,
                );
                replace_metadata(
                    &mut self.function_value_object_function_property_discriminants,
                    name,
                    functions,
                );
            }
        }
        if let Some(name) = assigned_variable.as_ref() {
            self.invalidate_destructured_union_correlation(name);
        }
        if let Some(name) = assigned_variable {
            if let Some(HirType::Optional(payload)) = self.scope.get(&name) {
                if rhs_type == **payload || assign.op != AssignOp::Assign {
                    self.narrowings.insert(name, payload.as_ref().clone());
                } else {
                    self.narrowings.remove(&name);
                }
            } else if let Some(HirType::Nullable(payload)) = self.scope.get(&name) {
                if rhs_type == **payload || assign.op != AssignOp::Assign {
                    self.nullable_narrowings
                        .insert(name, payload.as_ref().clone());
                } else {
                    self.nullable_narrowings.remove(&name);
                }
            } else if let Some(HirType::Nullish(payload)) = self.scope.get(&name) {
                if rhs_type == **payload || assign.op != AssignOp::Assign {
                    self.nullish_narrowings
                        .insert(name, payload.as_ref().clone());
                } else {
                    self.nullish_narrowings.remove(&name);
                }
            } else if let Some(HirType::Union(elements)) = self.scope.get(&name) {
                if let Some(index) = elements.iter().position(|member| member == &rhs_type) {
                    self.union_narrowings
                        .insert(name, (vec![index], elements.clone()));
                } else {
                    self.union_narrowings.remove(&name);
                }
            }
        }
        self.wrap_call_argument_bindings(result, &bindings)
    }

    fn lower_assignment_pattern(
        &mut self,
        pattern: &Pat,
        value: HirExpr,
        ty: &HirType,
        statements: &mut Vec<HirStmt>,
    ) -> Result<(), String> {
        let value = if matches!(pattern, Pat::Object(_) | Pat::Array(_)) {
            self.bind_destructure_source_once(value, ty, statements)
        } else {
            value
        };
        if matches!(pattern, Pat::Object(_) | Pat::Array(_)) {
            if let HirType::Optional(payload) = ty {
                statements.push(HirStmt::If(
                    HirExpr::OptionalIsNone(Box::new(value.clone()), payload.as_ref().clone()),
                    vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                        "Cannot destructure undefined".into(),
                    )))],
                    Vec::new(),
                ));
                return self.lower_assignment_pattern(
                    pattern,
                    HirExpr::OptionalValue(Box::new(value), payload.as_ref().clone()),
                    payload,
                    statements,
                );
            }
        }
        match pattern {
            Pat::Ident(binding) => {
                let function = self.hir_function_property_discriminants(&value);
                let name = self.resolve_binding(binding.id.sym.as_ref());
                if self.immutable_bindings.contains(&name) {
                    return Err(format!("cannot assign to constant `{name}`"));
                }
                let expected = self
                    .scope
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| format!("assignment to unknown binding `{name}`"))?;
                // A plain `number[]`/`string[]` (no `noUncheckedIndexedAccess`)
                // types each element read as `F64`/`Str`, not `Optional<F64>`/
                // `Optional<Str>`, matching real tsc's own lax default array
                // indexing -- but `[a, b] = arr` (array-destructuring
                // *assignment*, not a fresh `const`/`let` declaration) reads
                // each position via the array-destructuring `Pat::Array` arm
                // below, whose own `array_read_type` deliberately wraps every
                // primitive element in `Optional` (an under-length array at
                // *runtime* legitimately yields `undefined` there, matching
                // real JS). `a`/`b`'s own *already-declared* type (`F64`/
                // `Str`, from wherever they were first declared) can't accept
                // that `Optional` value directly. `coerce_primitive_array_
                // argument` (`arrays/transformations.rs`) already has this
                // exact widening for the identical "declared F64/Str,
                // resolved Optional<F64>/Optional<Str>" shape at a function
                // call's own argument-marshaling site -- reused directly here
                // instead of duplicating it, a pure superset of `coerce_to_
                // declared` (every other declared type, or a non-Optional
                // source, behaves identically to before).
                let value = self.coerce_primitive_array_argument(value, &expected)?;
                self.invalidate_destructured_union_correlation(&name);
                self.record_binding_write(&name);
                let (result, array, nested_array, object, functions) = function
                    .map(|metadata| {
                        (
                            metadata.value,
                            metadata.array,
                            (!metadata.nested_array.is_empty()).then_some(metadata.nested_array),
                            (!metadata.object.is_empty()).then_some(metadata.object),
                            (!metadata.functions.is_empty()).then_some(metadata.functions),
                        )
                    })
                    .unwrap_or_default();
                replace_metadata(&mut self.function_value_discriminants, &name, result);
                replace_metadata(&mut self.function_value_array_discriminants, &name, array);
                replace_metadata(
                    &mut self.function_value_nested_array_discriminants,
                    &name,
                    nested_array,
                );
                replace_metadata(
                    &mut self.function_value_object_array_property_discriminants,
                    &name,
                    object,
                );
                replace_metadata(
                    &mut self.function_value_object_function_property_discriminants,
                    &name,
                    functions,
                );
                statements.push(HirStmt::Expr(HirExpr::Assign(name, Box::new(value))));
                Ok(())
            }
            Pat::Object(pattern) => {
                if let HirType::Dictionary(element) = ty {
                    return self.lower_dictionary_object_assignment_pattern(
                        pattern,
                        value,
                        element,
                        statements,
                    );
                }
                if ty == &HirType::Json {
                    return self.lower_dictionary_object_assignment_pattern(
                        pattern,
                        value,
                        &HirType::Json,
                        statements,
                    );
                }
                if let HirType::Union(elements) = ty {
                    return self.lower_union_object_assignment_pattern(
                        pattern, value, elements, statements,
                    );
                }
                let HirType::Object(fields) = ty else {
                    return Err(format!("object pattern cannot destructure {ty:?}"));
                };
                let mut used = BTreeSet::new();
                for property in &pattern.props {
                    match property {
                        ObjectPatProp::Assign(property) => {
                            let key = property.key.id.sym.to_string();
                            let field_type =
                                Self::fixed_object_property_read_type(fields, &key)?;
                            used.insert(key.clone());
                            let mut field_value = self
                                .lower_fixed_object_property_read(value.clone(), fields, &key)?;
                            let mut binding_type = field_type.clone();
                            if let Some(default) = &property.value {
                                let default = self.lower_expr(default)?;
                                field_value = self.lower_undefined_default(field_value, default)?;
                                binding_type = self.infer_expr_type(&field_value)?;
                            }
                            self.lower_assignment_pattern(
                                &Pat::Ident(property.key.clone()),
                                field_value,
                                &binding_type,
                                statements,
                            )?;
                        }
                        ObjectPatProp::KeyValue(property) => {
                            let key = self
                                .static_object_property_name(&property.key)
                                .ok_or_else(|| "unsupported object destructuring key".to_string())?;
                            let field_type =
                                Self::fixed_object_property_read_type(fields, &key)?;
                            used.insert(key.clone());
                            let field_value = self
                                .lower_fixed_object_property_read(value.clone(), fields, &key)?;
                            self.lower_assignment_pattern(
                                &property.value,
                                field_value,
                                &field_type,
                                statements,
                            )?;
                        }
                        ObjectPatProp::Rest(rest) => {
                            let omitted = used.iter().cloned().collect::<Vec<_>>();
                            let (rest_value, rest_type) = self.lower_fixed_object_rest_copy(
                                value.clone(), fields, &omitted,
                            )?;
                            self.lower_assignment_pattern(
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
            Pat::Array(pattern) => {
                if let HirType::Union(elements) = ty {
                    if elements
                        .iter()
                        .all(|element| matches!(element, HirType::Tuple(_)))
                    {
                        return self.lower_union_tuple_assignment_pattern(
                            pattern, value, elements, statements,
                        );
                    }
                    if elements
                        .iter()
                        .all(|element| matches!(element, HirType::Array(_)))
                    {
                        let (flattened, flattened_type) =
                            self.lower_union_array_sequence(value, elements)?;
                        let name = format!("__thaw_assign_union_array_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), flattened_type.clone());
                        statements.push(HirStmt::Let(name.clone(), flattened_type.clone(), flattened));
                        return self.lower_assignment_pattern(
                            &Pat::Array(pattern.clone()),
                            HirExpr::Var(name),
                            &flattened_type,
                            statements,
                        );
                    }
                }
                if let HirType::Array(element) = ty {
                    let array_type = ty.clone();
                    for (index, element_pattern) in pattern.elems.iter().enumerate() {
                        let Some(element_pattern) = element_pattern else { continue };
                        if let Pat::Rest(rest) = element_pattern {
                            let rest_value = HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_slice".into())),
                                vec![
                                    value.clone(),
                                    HirExpr::Lit(HirLit::F64(index as f64)),
                                    HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                                ],
                            );
                            self.lower_assignment_pattern(
                                &rest.arg,
                                rest_value,
                                ty,
                                statements,
                            )?;
                            break;
                        }
                        let read_type = Self::array_read_type(element);
                        let read = self.lower_array_index(
                            value.clone(),
                            array_type.clone(),
                            element.as_ref().clone(),
                            HirExpr::Lit(HirLit::F64(index as f64)),
                        )?;
                        self.lower_assignment_pattern(
                            element_pattern,
                            read,
                            &read_type,
                            statements,
                        )?;
                    }
                    return Ok(());
                }
                if *ty == HirType::Json {
                    for (index, element_pattern) in pattern.elems.iter().enumerate() {
                        let Some(element_pattern) = element_pattern else {
                            continue;
                        };
                        if let Pat::Rest(rest) = element_pattern {
                            self.lower_assignment_pattern(
                                &rest.arg,
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_json_array_slice".into())),
                                    vec![
                                        value.clone(),
                                        HirExpr::Lit(HirLit::F64(index as f64)),
                                    ],
                                ),
                                &HirType::Json,
                                statements,
                            )?;
                            break;
                        }
                        self.lower_assignment_pattern(
                            element_pattern,
                            HirExpr::JsonIndex(
                                Box::new(value.clone()),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            &HirType::Json,
                            statements,
                        )?;
                    }
                    return Ok(());
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
                        self.lower_assignment_pattern(
                            &rest.arg, rest_value, &rest_type, statements,
                        )?;
                        break;
                    }
                    let element_type = elements
                        .get(index)
                        .cloned()
                        .ok_or_else(|| format!("tuple pattern index {index} is out of bounds"))?;
                    self.lower_assignment_pattern(
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
                self.lower_assignment_pattern(&assign.left, value, &value_type, statements)?;
                if let Pat::Ident(binding) = assign.left.as_ref() {
                    self.destructuring_default_types
                        .insert(self.resolve_binding(binding.id.sym.as_ref()), default_type);
                }
                Ok(())
            }
            Pat::Rest(_) => Err("rest patterns are only valid inside object/array patterns".into()),
            _ => Err("unsupported destructuring assignment target".into()),
        }
    }

    fn lower_dictionary_object_assignment_pattern(
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
                    let mut field =
                        self.typed_dictionary_read(value.clone(), key.clone(), element)?;
                    if let Some(default) = &property.value {
                        let default = self.lower_expr(default)?;
                        let default = self.coerce_to_declared(element, default)?;
                        field = self.lower_dictionary_default(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_has_own".into())),
                                vec![value.clone(), key],
                            ),
                            field,
                            default,
                            element,
                        );
                    }
                    self.lower_assignment_pattern(
                        &Pat::Ident(property.key.clone()),
                        field,
                        element,
                        statements,
                    )?;
                }
                ObjectPatProp::KeyValue(property) => {
                    let key = match self.static_object_property_name(&property.key) {
                        Some(key) => HirExpr::Lit(HirLit::Str(key)),
                        None => {
                            let PropName::Computed(computed) = &property.key else {
                                return Err("unsupported dictionary destructuring key".into());
                            };
                            let key = self.lower_expr(&computed.expr)?;
                            self.coerce_primitive_to_string(key)?
                        }
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
                    let mut field =
                        self.typed_dictionary_read(value.clone(), key.clone(), element)?;
                    let target = if let Pat::Assign(assign) = property.value.as_ref() {
                        let default = self.lower_expr(&assign.right)?;
                        let default = self.coerce_to_declared(element, default)?;
                        field = self.lower_dictionary_default(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_has_own".into())),
                                vec![value.clone(), key],
                            ),
                            field,
                            default,
                            element,
                        );
                        assign.left.as_ref()
                    } else {
                        &property.value
                    };
                    self.lower_assignment_pattern(target, field, element, statements)?;
                }
                ObjectPatProp::Rest(rest) => {
                    let rest_type = HirType::Dictionary(Box::new(element.clone()));
                    let rest_value =
                        self.lower_dictionary_object_rest(value.clone(), &rest_type, &used_keys);
                    self.lower_assignment_pattern(
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

}
