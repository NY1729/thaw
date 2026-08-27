impl<'a> FnLowerer<'a> {
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
                let (_, _, base_name) = self
                    .super_initializer
                    .clone()
                    .ok_or("`super` property assignment is only valid in a derived class")?;
                let property = super_property_name(&member.prop)?;
                let symbol = class_setter_symbol(&base_name, &property, self.class_static_context);
                let signature = self.signatures.get(&symbol).cloned().ok_or_else(|| {
                    format!("base class `{base_name}` has no setter `{property}`")
                })?;
                let rhs = self.lower_expr(&assign.right)?;
                let value_index = usize::from(!self.class_static_context);
                let rhs = self.coerce_to_declared(&signature.params[value_index], rhs)?;
                let mut args = if self.class_static_context {
                    Vec::new()
                } else {
                    vec![HirExpr::Var(self.resolve_binding("this"))]
                };
                args.push(rhs);
                return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), args));
            }
            if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left {
                if let (Expr::Ident(receiver), Some(property)) =
                    (member.obj.as_ref(), member_property_name(&member.prop))
                {
                    let static_symbol = class_setter_symbol(receiver.sym.as_ref(), &property, true);
                    let (symbol, receiver_argument) =
                        if self.signatures.contains_key(&static_symbol) {
                            (Some(static_symbol), None)
                        } else {
                            let binding = self.resolve_binding(receiver.sym.as_ref());
                            let instance_symbol = self.scope.get(&binding).and_then(|ty| {
                                class_name_from_type(ty).map(|class_name| {
                                    class_setter_symbol(class_name, &property, false)
                                })
                            });
                            match instance_symbol {
                                Some(symbol) if self.signatures.contains_key(&symbol) => {
                                    (Some(symbol), Some(HirExpr::Var(binding)))
                                }
                                _ => (None, None),
                            }
                        };
                    if let Some(symbol) = symbol {
                        let signature = self.signatures[&symbol].clone();
                        let value_index = usize::from(receiver_argument.is_some());
                        let rhs = self.lower_expr(&assign.right)?;
                        let rhs = self.coerce_to_declared(&signature.params[value_index], rhs)?;
                        let mut args = Vec::with_capacity(value_index + 1);
                        if let Some(receiver) = receiver_argument {
                            args.push(receiver);
                        }
                        args.push(rhs);
                        return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), args));
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
                    let symbol = self.scope.get(&binding).and_then(|ty| {
                        class_name_from_type(ty)
                            .map(|class_name| class_setter_symbol(class_name, &property, false))
                    });
                    if let Some(symbol) =
                        symbol.filter(|symbol| self.signatures.contains_key(symbol))
                    {
                        let signature = self.signatures[&symbol].clone();
                        let rhs = self.lower_expr(&assign.right)?;
                        let rhs = self.coerce_to_declared(&signature.params[1], rhs)?;
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var(symbol)),
                            vec![HirExpr::Var(binding), rhs],
                        ));
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
                            && elements.iter().all(|element| matches!(element, HirType::Tuple(_))))
            );
            let destructurable_dictionary =
                matches!((&pattern, &ty), (Pat::Object(_), HirType::Dictionary(_)));
            if !matches!(ty, HirType::Object(_) | HirType::Tuple(_))
                && !destructurable_union
                && !destructurable_dictionary
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

        let mut target = self.lower_assign_target(&assign.left)?;
        if let Target::Var(name) = &target {
            if self.immutable_bindings.contains(name) {
                return Err(format!("cannot assign to constant `{name}`"));
            }
        }
        let rhs = self.lower_expr(&assign.right)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
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

        if assign.op != AssignOp::Assign {
            target = match target {
                Target::Var(name) => Target::Var(name),
                Target::Index(array, index) => {
                    let array_name = format!("__thaw_assign_array_{}", self.next_binding);
                    self.next_binding += 1;
                    let array_type = HirType::Array(Box::new(HirType::F64));
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
            };
        }

        if assign.op == AssignOp::NullishAssign {
            let current = target_to_read_expr(&target)?;
            let current_type = self.infer_expr_type(&current)?;
            let (payload, absence_kind) = match current_type.clone() {
                HirType::Optional(payload) => (payload, 0),
                HirType::Nullable(payload) => (payload, 1),
                HirType::Nullish(payload) => (payload, 2),
                _ => return self.wrap_call_argument_bindings(current, &bindings),
            };
            let rhs = self.coerce_to_declared(payload.as_ref(), rhs)?;
            let current_name = format!("__thaw_nullish_assign_current_{}", self.next_binding);
            self.next_binding += 1;
            self.scope
                .insert(current_name.clone(), current_type.clone());
            let rhs_name = format!("__thaw_nullish_assign_rhs_{}", self.next_binding);
            self.next_binding += 1;
            self.scope
                .insert(rhs_name.clone(), payload.as_ref().clone());

            let stored = match absence_kind {
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
            };
            let assigned = HirExpr::Block(vec![
                HirStmt::Expr(build_assign(target, stored)),
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
            let result = HirExpr::Block(vec![HirStmt::If(
                is_none,
                vec![HirStmt::Return(Some(assigned))],
                vec![HirStmt::Return(Some(present))],
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

        let value = if assign.op == AssignOp::Assign {
            rhs
        } else if let Some(op) = compound_op(assign.op) {
            let current = target_to_read_expr(&target)?;
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
                HirExpr::BinOp(op, Box::new(current), Box::new(rhs))
            }
        } else {
            return Err(format!(
                "unsupported compound assignment operator {:?}",
                assign.op
            ));
        };

        // Reorder/typecheck an object literal against the target's
        // declared shape, same as a `let`/call-argument assignment --
        // needed now that a field can itself be an object (`p.corner =
        // { y: 2, x: 1 }`), not just a plain variable.
        let value = match &target {
            Target::Var(name) => match self.scope.get(name).cloned() {
                Some(ty) => self.coerce_to_declared(&ty, value)?,
                None => value,
            },
            Target::Prop(_, HirType::Object(fields), field) => {
                match fields.iter().find(|(n, _)| n == field) {
                    Some((_, ty)) => self.coerce_to_declared(&ty.clone(), value)?,
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
            Target::Prop(_, other, field) => {
                return Err(format!(
                    "cannot assign to field `{field}` on value of type {other:?}"
                ));
            }
        };

        let result = build_assign(target, value);
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
                let value = self.coerce_to_declared(&expected, value)?;
                self.invalidate_destructured_union_correlation(&name);
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
                            self.lower_assignment_pattern(
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
                            self.lower_assignment_pattern(
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
                            self.lower_assignment_pattern(
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
                        return self.lower_union_tuple_assignment_pattern(
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
                    let field_json =
                        HirExpr::JsonKey(Box::new(value.clone()), Box::new(key.clone()));
                    let mut field = Self::dictionary_element_from_json(field_json, element)?;
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
                    let field_json =
                        HirExpr::JsonKey(Box::new(value.clone()), Box::new(key.clone()));
                    let mut field = Self::dictionary_element_from_json(field_json, element)?;
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
