impl<'a> FnLowerer<'a> {
    /// Preserve JavaScript's property-update semantics for an `any` (Json) member.
    fn lower_json_member_update(
        &mut self,
        update: &swc_ecma_ast::UpdateExpr,
        target: Target,
    ) -> Result<HirExpr, String> {
        let mut bindings = Vec::new();
        let (target, guard_nullish) = match target {
            Target::Dictionary(object, key, HirType::Json) => {
                let object_name = format!("__thaw_update_json_object_{}", self.next_binding);
                self.next_binding += 1;
                let object_type = self.infer_expr_type(&object)?;
                let guard_nullish = object_type == HirType::Json;
                self.scope.insert(object_name.clone(), object_type.clone());
                bindings.push((object_name.clone(), object_type, object));

                let key_name = format!("__thaw_update_json_key_{}", self.next_binding);
                self.next_binding += 1;
                let key_type = self.infer_expr_type(&key)?;
                self.scope.insert(key_name.clone(), key_type.clone());
                bindings.push((key_name.clone(), key_type, *key));
                (
                    Target::Dictionary(
                        HirExpr::Var(object_name),
                        Box::new(HirExpr::Var(key_name)),
                        HirType::Json,
                    ),
                    guard_nullish,
                )
            }
            Target::JsonIndex(object, index) => {
                let object_name = format!("__thaw_update_json_object_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(object_name.clone(), HirType::Json);
                bindings.push((object_name.clone(), HirType::Json, object));

                let index_name = format!("__thaw_update_json_index_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(index_name.clone(), HirType::F64);
                bindings.push((index_name.clone(), HirType::F64, *index));
                (
                    Target::JsonIndex(
                        HirExpr::Var(object_name),
                        Box::new(HirExpr::Var(index_name)),
                    ),
                    true,
                )
            }
            Target::DynamicProperty(object, key) => {
                let object_name = format!("__thaw_update_dynamic_object_{}", self.next_binding);
                self.next_binding += 1;
                let object_type = self.infer_expr_type(&object)?;
                self.scope.insert(object_name.clone(), object_type.clone());
                bindings.push((object_name.clone(), object_type, object));

                let key_name = format!("__thaw_update_dynamic_key_{}", self.next_binding);
                self.next_binding += 1;
                let key_type = self.infer_expr_type(&key)?;
                self.scope.insert(key_name.clone(), key_type.clone());
                bindings.push((key_name.clone(), key_type, *key));
                (
                    Target::DynamicProperty(
                        HirExpr::Var(object_name),
                        Box::new(HirExpr::Var(key_name)),
                    ),
                    false,
                )
            }
            Target::Prop(object, object_type, field) => {
                let is_json_field = matches!(
                    &object_type,
                    HirType::Object(fields)
                        if fields.iter().any(|(name, ty)| name == &field && ty == &HirType::Json)
                );
                if !is_json_field {
                    return Err("unsupported JSON update target".into());
                }
                let object_name = format!("__thaw_update_object_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(object_name.clone(), object_type.clone());
                bindings.push((object_name.clone(), object_type.clone(), object));
                (
                    Target::Prop(HirExpr::Var(object_name), object_type, field),
                    false,
                )
            }
            _ => return Err("unsupported JSON update target".into()),
        };
        let current = match &target {
            Target::DynamicProperty(object, key) => {
                let value = HirExpr::Call(
                    Box::new(HirExpr::Var("getDynamicProperty".into())),
                    vec![object.clone(), key.as_ref().clone()],
                );
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_from_consumed_dynamic".into())),
                    vec![value],
                )
            }
            _ => self.lower_assignment_target_read(&target)?,
        };
        let current = self.coerce_to_declared_aliasing(&HirType::Json, current)?;
        let increment = self.coerce_to_declared_aliasing(
            &HirType::Json,
            HirExpr::Lit(HirLit::Bool(update.op == swc_ecma_ast::UpdateOp::PlusPlus)),
        )?;
        let args = self.wrap_native_value_as_json(
            HirExpr::ArrayLit(vec![current, increment]),
            HirType::Array(Box::new(HirType::Json)),
        )?;
        let updated_pair_name = format!("__thaw_update_json_pair_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(updated_pair_name.clone(), HirType::Json);
        let update_helper = HirExpr::Call(
            Box::new(HirExpr::Var("getDynamicValue".into())),
            vec![HirExpr::Lit(HirLit::Str(
                "__thaw_update_dynamic_value".into(),
            ))],
        );
        let updated_pair_handle = HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueHandle".into())),
            vec![
                update_helper,
                args,
            ],
        );
        let updated_pair = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_json_from_consumed_dynamic".into())),
            vec![updated_pair_handle],
        );
        let pair_binding = (updated_pair_name.clone(), HirType::Json, updated_pair);
        let pair = HirExpr::Var(updated_pair_name);
        let old = HirExpr::JsonIndex(
            Box::new(pair.clone()),
            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
        );
        let updated = HirExpr::JsonIndex(
            Box::new(pair),
            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
        );
        let write = match &target {
            Target::DynamicProperty(object, key) => HirExpr::Call(
                Box::new(HirExpr::Var("setDynamicPropertyJson".into())),
                vec![object.clone(), key.as_ref().clone(), updated.clone()],
            ),
            _ => self.lower_assignment_target_write(target.clone(), updated.clone())?,
        };
        let result = HirExpr::Block(vec![
            HirStmt::Expr(write),
            HirStmt::Return(Some(if update.prefix { updated } else { old })),
        ]);
        let result = if guard_nullish {
            let object = match &target {
                Target::Dictionary(object, _, _) | Target::JsonIndex(object, _) => object.clone(),
                _ => unreachable!(),
            };
            let key = match &target {
                Target::Dictionary(_, key, _) => key.as_ref().clone(),
                Target::JsonIndex(_, index) => index.as_ref().clone(),
                _ => unreachable!(),
            };
            let key = self.coerce_primitive_to_string(key)?;
            let type_error = |kind: &str| {
                let concat = |left, right| {
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_concat".into())),
                        vec![left, right],
                    )
                };
                let message = concat(
                    concat(
                        HirExpr::Lit(HirLit::Str(format!(
                        "\u{1}TypeError\u{1}Cannot read properties of {kind} (reading '",
                    ))),
                        key.clone(),
                    ),
                    HirExpr::Lit(HirLit::Str("')".into())),
                );
                HirStmt::Throw(message)
            };
            let is_nullish = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_is_nullish".into())),
                vec![object.clone()],
            );
            let is_null = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_is_null".into())),
                vec![object],
            );
            HirExpr::Block(vec![
                HirStmt::If(
                    is_nullish,
                    vec![HirStmt::If(
                        is_null,
                        vec![type_error("null")],
                        vec![type_error("undefined")],
                    )],
                    Vec::new(),
                ),
                HirStmt::Return(Some(self.wrap_call_argument_bindings(
                    result,
                    &[pair_binding],
                )?)),
            ])
        } else {
            self.wrap_call_argument_bindings(result, &[pair_binding])?
        };
        self.wrap_call_argument_bindings(result, &bindings)
    }
}

impl<'a> FnLowerer<'a> {
    fn lower_update(&mut self, update: &swc_ecma_ast::UpdateExpr) -> Result<HirExpr, String> {
        if self.unbound_this_context {
            if let Expr::Member(member) = update.arg.as_ref() {
                if matches!(member.obj.as_ref(), Expr::This(_)) {
                    let property = member_property_name(&member.prop)
                        .ok_or("unbound `this` update requires a statically known property")?;
                    return self.lower_unbound_this_member(&property);
                }
            }
        }
        if let Expr::Member(member) = update.arg.as_ref() {
            let static_class = match member.obj.as_ref() {
                Expr::Ident(receiver) => Some(receiver.sym.to_string()),
                Expr::This(_) if self.class_static_context => self.class_context.clone(),
                _ => None,
            };
            if let (Some(receiver), Some(property)) =
                (static_class, member_property_name(&member.prop))
            {
                let getter = class_getter_symbol(&receiver, &property, true);
                if let Some(getter_signature) = self.signatures.get(&getter).cloned() {
                    let setter = class_setter_symbol(&receiver, &property, true);
                    if !self.signatures.contains_key(&setter) {
                        return Err(format!(
                            "cannot update readonly static member `{}.{}`",
                            receiver, property
                        ));
                    }
                    if getter_signature.ret != HirType::F64 {
                        return Err(format!(
                            "cannot apply ++/-- to non-number static member `{}.{}`",
                            receiver, property
                        ));
                    }
                    let operator = match update.op {
                        UpdateOp::PlusPlus => BinOp::Add,
                        UpdateOp::MinusMinus => BinOp::Sub,
                    };
                    let current = HirExpr::Call(Box::new(HirExpr::Var(getter)), Vec::new());
                    let old_name = format!("__thaw_static_update_old_{}", self.next_binding);
                    self.next_binding += 1;
                    let updated_name = format!("__thaw_static_update_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(old_name.clone(), HirType::F64);
                    self.scope.insert(updated_name.clone(), HirType::F64);
                    let old = HirExpr::Var(old_name.clone());
                    let updated = HirExpr::BinOp(
                        operator,
                        Box::new(old.clone()),
                        Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                    );
                    let result = HirExpr::Block(vec![
                        HirStmt::Expr(HirExpr::Call(
                            Box::new(HirExpr::Var(setter)),
                            vec![HirExpr::Var(updated_name.clone())],
                        )),
                        HirStmt::Return(Some(if update.prefix {
                            HirExpr::Var(updated_name.clone())
                        } else {
                            old
                        })),
                    ]);
                    return self.wrap_call_argument_bindings(
                        result,
                        &[
                            (old_name, HirType::F64, current),
                            (updated_name, HirType::F64, updated),
                        ],
                    );
                }
            }
        }
        if let Expr::Member(member) = update.arg.as_ref() {
            if let Some(property) = member_property_name(&member.prop) {
                let instance_receiver = match member.obj.as_ref() {
                    Expr::This(_) => !self.class_static_context,
                    Expr::Ident(identifier) => self.scope
                        .get(&self.resolve_binding(identifier.sym.as_ref()))
                        .and_then(class_name_from_type).is_some(),
                    Expr::New(_) | Expr::Call(_) => true,
                    _ => false,
                };
                if instance_receiver {
                    let receiver = self.lower_member_receiver(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if let Some(owner) = self.class_instance_accessor_owner(&receiver_type, &property) {
                        let getter = class_getter_symbol(&owner, &property, false);
                        let setter = class_setter_symbol(&owner, &property, false);
                        let own_getter = self.signatures.get(&getter)
                            .filter(|signature| signature.accessor_owner.as_deref() == Some(owner.as_str()));
                        let own_setter = self.signatures.get(&setter)
                            .filter(|signature| signature.accessor_owner.as_deref() == Some(owner.as_str()));
                        let getter_signature = own_getter.ok_or_else(||
                            format!("cannot update write-only instance accessor `{owner}.{property}`"))?;
                        if getter_signature.ret != HirType::F64 {
                            return Err(format!("cannot apply ++/-- to non-number instance accessor `{owner}.{property}`"));
                        }
                        if own_setter.is_none() {
                            return Err(format!("cannot update readonly instance accessor `{owner}.{property}`"));
                        }
                        if own_setter.and_then(|signature| signature.params.get(1)) != Some(&HirType::F64) {
                            return Err(format!("cannot update instance accessor `{owner}.{property}` with non-number setter"));
                        }
                        let receiver_name = format!("__thaw_instance_update_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        let old_name = format!("__thaw_instance_update_old_{}", self.next_binding);
                        self.next_binding += 1;
                        let updated_name = format!("__thaw_instance_update_value_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), receiver_type.clone());
                        self.scope.insert(old_name.clone(), HirType::F64);
                        self.scope.insert(updated_name.clone(), HirType::F64);
                        let current = HirExpr::Call(Box::new(HirExpr::Var(getter)), vec![
                            self.assert_class_accessor_receiver(HirExpr::Var(receiver_name.clone()), &owner),
                        ]);
                        let old = HirExpr::Var(old_name.clone());
                        let updated = HirExpr::BinOp(
                            match update.op { UpdateOp::PlusPlus => BinOp::Add, UpdateOp::MinusMinus => BinOp::Sub },
                            Box::new(old.clone()), Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        );
                        let result = HirExpr::Block(vec![
                            HirStmt::Expr(HirExpr::Call(Box::new(HirExpr::Var(setter)), vec![
                                self.assert_class_accessor_receiver(HirExpr::Var(receiver_name.clone()), &owner),
                                HirExpr::Var(updated_name.clone()),
                            ])),
                            HirStmt::Return(Some(if update.prefix { HirExpr::Var(updated_name.clone()) } else { old })),
                        ]);
                        return self.wrap_call_argument_bindings(result, &[
                            (receiver_name, receiver_type, receiver),
                            (old_name, HirType::F64, current),
                            (updated_name, HirType::F64, updated),
                        ]);
                    }
                }
            }
        }
        if let Expr::SuperProp(member) = update.arg.as_ref() {
            let (_, base_type, base_name) = self.super_initializer.clone()
                .ok_or("`super` update is only valid in a derived class")?;
            let property = super_property_name(&member.prop)?;
            let owner = if self.class_static_context { base_name } else {
                self.class_super_instance_accessor_owner(&base_type, &property)
                    .ok_or_else(|| format!("base class `{base_name}` has no accessor `{property}`"))?
            };
            let getter = class_getter_symbol(&owner, &property, self.class_static_context);
            let setter = class_setter_symbol(&owner, &property, self.class_static_context);
            let getter_signature = self.signatures.get(&getter)
                .filter(|signature| self.class_static_context || signature.accessor_owner.as_deref() == Some(owner.as_str()))
                .ok_or_else(|| format!("cannot update write-only super accessor `{owner}.{property}`"))?;
            if getter_signature.ret != HirType::F64 {
                return Err(format!("cannot apply ++/-- to non-number super accessor `{owner}.{property}`"));
            }
            let setter_signature = self.signatures.get(&setter)
                .filter(|signature| self.class_static_context || signature.accessor_owner.as_deref() == Some(owner.as_str()))
                .ok_or_else(|| format!("cannot update readonly super accessor `{owner}.{property}`"))?;
            let value_index = usize::from(!self.class_static_context);
            if setter_signature.params.get(value_index) != Some(&HirType::F64) {
                return Err(format!("cannot update super accessor `{owner}.{property}` with non-number setter"));
            }
            let receiver = HirExpr::Var(self.resolve_binding("this"));
            let getter_args = if self.class_static_context { Vec::new() } else {
                vec![self.assert_class_accessor_receiver(receiver.clone(), &owner)]
            };
            let old_name = format!("__thaw_super_update_old_{}", self.next_binding);
            self.next_binding += 1;
            let updated_name = format!("__thaw_super_update_value_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(old_name.clone(), HirType::F64);
            self.scope.insert(updated_name.clone(), HirType::F64);
            let old = HirExpr::Var(old_name.clone());
            let updated = HirExpr::BinOp(
                match update.op { UpdateOp::PlusPlus => BinOp::Add, UpdateOp::MinusMinus => BinOp::Sub },
                Box::new(old.clone()), Box::new(HirExpr::Lit(HirLit::F64(1.0))),
            );
            let setter_args = if self.class_static_context {
                vec![HirExpr::Var(updated_name.clone())]
            } else {
                vec![self.assert_class_accessor_receiver(receiver, &owner), HirExpr::Var(updated_name.clone())]
            };
            let result = HirExpr::Block(vec![
                HirStmt::Expr(HirExpr::Call(Box::new(HirExpr::Var(setter)), setter_args)),
                HirStmt::Return(Some(if update.prefix { HirExpr::Var(updated_name.clone()) } else { old })),
            ]);
            return self.wrap_call_argument_bindings(result, &[
                (old_name, HirType::F64, HirExpr::Call(Box::new(HirExpr::Var(getter)), getter_args)),
                (updated_name, HirType::F64, updated),
            ]);
        }
        if let Expr::Member(member) = update.arg.as_ref() {
            if member_property_name(&member.prop).as_deref() == Some("length") {
                let array = self.lower_expr(&member.obj)?;
                let array_type = self.infer_expr_type(&array)?;
                if matches!(&array_type, HirType::Array(_))
                    || matches!(&array_type, HirType::Union(members) if members.iter().all(|member| matches!(member, HirType::Array(_))))
                {
                    let array_name = format!("__thaw_length_update_array_{}", self.next_binding);
                    let old_name = format!("__thaw_length_update_old_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(array_name.clone(), array_type.clone());
                    self.scope.insert(old_name.clone(), HirType::F64);
                    let old = HirExpr::Var(old_name.clone());
                    let op = match update.op {
                        UpdateOp::PlusPlus => BinOp::Add,
                        UpdateOp::MinusMinus => BinOp::Sub,
                    };
                    let updated = HirExpr::BinOp(op, Box::new(old.clone()), Box::new(HirExpr::Lit(HirLit::F64(1.0))));
                    let assigned = self.lower_array_length_write(HirExpr::Var(array_name.clone()), updated)?;
                    let result = if update.prefix {
                        assigned
                    } else {
                        HirExpr::Block(vec![HirStmt::Expr(assigned), HirStmt::Return(Some(old))])
                    };
                    let current = self.lower_array_length_read(
                        HirExpr::Var(array_name.clone()),
                        &array_type,
                    )?;
                    return self.wrap_call_argument_bindings(result, &[
                        (array_name, array_type, array),
                        (old_name, HirType::F64, current),
                    ]);
                }
            }
        }
        if let Expr::Member(member) = update.arg.as_ref() {
            if let MemberProp::Computed(computed) = &member.prop {
                let array = self.lower_expr(&member.obj)?;
                if let HirType::Union(members) = self.infer_expr_type(&array)? {
                    if members.iter().all(|member| matches!(member, HirType::Array(_))) {
                        self.ensure_union_array_write_type(&members, &HirType::F64)?;
                        let index = self.lower_expr(&computed.expr)?;
                        self.expect_type(&HirType::F64, &index, "array index")?;
                        let array_name = format!("__thaw_union_update_array_{}", self.next_binding);
                        let index_name = format!("__thaw_union_update_index_{}", self.next_binding);
                        let old_name = format!("__thaw_union_update_old_{}", self.next_binding);
                        self.next_binding += 1;
                        let union_type = HirType::Union(members.clone());
                        self.scope.insert(array_name.clone(), union_type.clone());
                        self.scope.insert(index_name.clone(), HirType::F64);
                        let current = self.lower_union_array_numeric_read(
                            &array_name,
                            &members,
                            HirExpr::Var(index_name.clone()),
                        )?;
                        let current = self.coerce_primitive_to_number(current)?;
                        self.scope.insert(old_name.clone(), HirType::F64);
                        let old = HirExpr::Var(old_name.clone());
                        let operator = match update.op {
                            UpdateOp::PlusPlus => BinOp::Add,
                            UpdateOp::MinusMinus => BinOp::Sub,
                        };
                        let updated = HirExpr::BinOp(
                            operator,
                            Box::new(old.clone()),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        );
                        let write = self.lower_union_array_index_write(
                            HirExpr::Var(array_name.clone()),
                            &members,
                            HirExpr::Var(index_name.clone()),
                            updated,
                        )?;
                        let result = if update.prefix {
                            write
                        } else {
                            HirExpr::Block(vec![
                                HirStmt::Expr(write),
                                HirStmt::Return(Some(old)),
                            ])
                        };
                        return self.wrap_call_argument_bindings(
                            result,
                            &[
                                (array_name, union_type, array),
                                (index_name, HirType::F64, index),
                                (old_name, HirType::F64, current),
                            ],
                        );
                    }
                }
            }
        }
        let target = match update.arg.as_ref() {
            Expr::Ident(ident) => Target::Var(self.resolve_binding(ident.sym.as_ref())),
            Expr::Member(member) => {
                let static_this_target = (matches!(member.obj.as_ref(), Expr::This(_))
                    && self.class_static_context)
                    .then(|| {
                        let class = self
                            .class_context
                            .as_deref()
                            .expect("static update retains its class context");
                        member_property_name(&member.prop)
                            .map(|property| class_static_field_symbol(class, &property))
                    })
                    .flatten()
                    .filter(|symbol| self.scope.contains_key(symbol));
                if let Some(symbol) = static_this_target {
                    Target::Var(symbol)
                } else if let (Expr::Ident(class), Some(property)) =
                    (member.obj.as_ref(), member_property_name(&member.prop))
                {
                    let symbol = class_static_field_symbol(class.sym.as_ref(), &property);
                    if self.scope.contains_key(&symbol) {
                        Target::Var(symbol)
                    } else {
                        match &member.prop {
                            MemberProp::Computed(computed) => {
                                let object = self.lower_json_mutable_chain(&member.obj)?;
                                let object_type = self.infer_expr_type(&object)?;
                                self.lower_computed_target_from_receiver(
                                    object,
                                    object_type,
                                    computed,
                                )?
                            }
                            MemberProp::Ident(prop) => {
                                let object = self.lower_expr(&member.obj)?;
                                let object = if matches!(member.obj.as_ref(), Expr::This(_)) {
                                    object.with_non_arrow_this_access(NonArrowThisAccess::Read(prop.sym.to_string()))
                                } else {
                                    object
                                };
                                let object_type = match &object {
                                    HirExpr::NonArrowThis { declared, .. } => declared.clone(),
                                    _ => self.infer_expr_type(&object)?,
                                };
                                match &object_type {
                                    HirType::Object(fields)
                                        if fields.iter().any(|(name, ty)| {
                                            name == prop.sym.as_str() && matches!(ty, HirType::F64 | HirType::Json)
                                        }) =>
                                    {
                                        Target::Prop(object, object_type, prop.sym.to_string())
                                    }
                                    HirType::Dictionary(element)
                                        if element.as_ref() == &HirType::F64 =>
                                    {
                                        Target::Dictionary(
                                            object,
                                            Box::new(HirExpr::Lit(HirLit::Str(
                                                prop.sym.to_string(),
                                            ))),
                                            HirType::F64,
                                        )
                                    }
                                    HirType::JsValue | HirType::Dynamic => {
                                        Target::DynamicProperty(
                                            object,
                                            Box::new(HirExpr::Lit(HirLit::Str(
                                                prop.sym.to_string(),
                                            ))),
                                        )
                                    }
                                    HirType::Json => Target::Dictionary(
                                        object,
                                        Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                                        HirType::Json,
                                    ),
                                    _ => {
                                        return Err(format!(
                                            "cannot apply ++/-- to non-number field `.{}` on {object_type:?}",
                                            prop.sym
                                        ))
                                    }
                                }
                            }
                            _ => return Err("unsupported ++/-- target".into()),
                        }
                    }
                } else {
                    match &member.prop {
                        MemberProp::Computed(computed) => {
                            let object = self.lower_json_mutable_chain(&member.obj)?;
                            let object_type = self.infer_expr_type(&object)?;
                            self.lower_computed_target_from_receiver(
                                object,
                                object_type,
                                computed,
                            )?
                        }
                        MemberProp::Ident(prop) => {
                            let object = self.lower_expr(&member.obj)?;
                            let object = if matches!(member.obj.as_ref(), Expr::This(_)) {
                                object.with_non_arrow_this_access(NonArrowThisAccess::Read(prop.sym.to_string()))
                            } else {
                                object
                            };
                            let object_type = match &object {
                                HirExpr::NonArrowThis { declared, .. } => declared.clone(),
                                _ => self.infer_expr_type(&object)?,
                            };
                            match &object_type {
                                HirType::Object(fields)
                                    if fields.iter().any(|(name, ty)| {
                                        name == prop.sym.as_str() && matches!(ty, HirType::F64 | HirType::Json)
                                    }) =>
                                {
                                    Target::Prop(object, object_type, prop.sym.to_string())
                                }
                                HirType::Dictionary(element)
                                    if element.as_ref() == &HirType::F64 =>
                                {
                                    Target::Dictionary(
                                        object,
                                        Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                                        HirType::F64,
                                    )
                                }
                                HirType::JsValue | HirType::Dynamic => Target::DynamicProperty(
                                    object,
                                    Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                                ),
                                HirType::Json => Target::Dictionary(
                                    object,
                                    Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                                    HirType::Json,
                                ),
                                _ => {
                                    return Err(format!(
                                "cannot apply ++/-- to non-number field `.{}` on {object_type:?}",
                                prop.sym
                            ))
                                }
                            }
                        }
                        _ => return Err("unsupported ++/-- target".into()),
                    }
                }
            }
            _ => return Err("unsupported ++/-- target".into()),
        };
        if let Target::Var(name) = &target {
            if self.immutable_bindings.contains(name) {
                return Err(format!("cannot update constant `{name}`"));
            }
        }

        // A normalized constructor-function static field starts absent and
        // therefore has Optional(F64) storage.  Update its numeric value at
        // the original expression, without using scalar-only PostfixUpdate.
        if let (Expr::Member(member), Target::Var(symbol)) = (update.arg.as_ref(), &target) {
            let class = match member.obj.as_ref() {
                Expr::Ident(class) => Some(class.sym.to_string()),
                Expr::This(_) if self.class_static_context => self.class_context.clone(),
                _ => None,
            };
            let is_static_field = class.and_then(|class| member_property_name(&member.prop)
                .map(|property| class_static_field_symbol(&class, &property)))
                .as_deref() == Some(symbol.as_str());
            if is_static_field && self.scope.get(symbol)
                == Some(&HirType::Optional(Box::new(HirType::F64))) {
                self.record_binding_write(symbol);
                let old_name = format!("__thaw_static_optional_old_{}", self.next_binding);
                self.next_binding += 1;
                let updated_name = format!("__thaw_static_optional_updated_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(old_name.clone(), HirType::F64);
                self.scope.insert(updated_name.clone(), HirType::F64);
                let current = self.coerce_primitive_to_number(HirExpr::Var(symbol.clone()))?;
                let updated = HirExpr::BinOp(
                    match update.op { UpdateOp::PlusPlus => BinOp::Add, UpdateOp::MinusMinus => BinOp::Sub },
                    Box::new(HirExpr::Var(old_name.clone())),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                );
                let result = HirExpr::Block(vec![
                    HirStmt::Expr(HirExpr::Assign(symbol.clone(), Box::new(HirExpr::OptionalSome(
                        Box::new(HirExpr::Var(updated_name.clone())), HirType::F64,
                    )))),
                    HirStmt::Return(Some(HirExpr::Var(if update.prefix {
                        updated_name.clone()
                    } else {
                        old_name.clone()
                    }))),
                ]);
                return self.wrap_call_argument_bindings(result, &[
                    (old_name, HirType::F64, current),
                    (updated_name, HirType::F64, updated),
                ]);
            }
        }

        let has_json_update_target = match &target {
            Target::JsonIndex(..)
            | Target::Dictionary(_, _, HirType::Json)
            | Target::DynamicProperty(..) => true,
            Target::Prop(_, HirType::Object(fields), field) => fields
                .iter()
                .any(|(name, ty)| name == field && ty == &HirType::Json),
            _ => false,
        };
        if has_json_update_target {
            return self.lower_json_member_update(update, target);
        }
        let op = match update.op {
            UpdateOp::PlusPlus => BinOp::Add,
            UpdateOp::MinusMinus => BinOp::Sub,
        };
        let one = HirExpr::Lit(HirLit::F64(1.0));
        let current = self.lower_assignment_target_read(&target)?;
        // An array element read is `T | undefined`; `undefined++` is NaN, so coerce it.
        let current = if matches!(&target, Target::Index(..))
            || matches!(&target, Target::Prop(_, HirType::Object(fields), field)
            if fields.iter().any(|(name, _)| name == &format!("__thaw_setter_{field}")))
        {
            self.coerce_primitive_to_number(current)?
        } else {
            current
        };
        self.expect_type(&HirType::F64, &current, "update operand")?;
        if let Target::Var(name) = &target {
            self.record_binding_write(name);
        }
        if update.prefix {
            if let Target::Index(array, index) = target {
                let array_name = format!("__thaw_update_array_{}", self.next_binding);
                let index_name = format!("__thaw_update_index_{}", self.next_binding);
                self.next_binding += 1;
                let array_type = HirType::Array(Box::new(HirType::F64));
                self.scope.insert(array_name.clone(), array_type.clone());
                self.scope.insert(index_name.clone(), HirType::F64);
                let array_var = HirExpr::Var(array_name.clone());
                let index_var = HirExpr::Var(index_name.clone());
                let current = self.lower_array_index(
                    array_var.clone(), array_type.clone(), HirType::F64, index_var.clone(),
                )?;
                let value = HirExpr::BinOp(
                    op,
                    Box::new(self.coerce_primitive_to_number(current)?),
                    Box::new(one),
                );
                return self.wrap_call_argument_bindings(
                    build_assign(Target::Index(array_var, Box::new(index_var)), value),
                    &[(array_name, array_type, array), (index_name, HirType::F64, *index)],
                );
            }
            if let Target::Var(name) = target {
                let value = HirExpr::BinOp(op, Box::new(current), Box::new(one));
                return Ok(build_assign(Target::Var(name), value));
            }
        }
        if let Target::Var(name) = target {
            return Ok(HirExpr::PostfixUpdate(name, op));
        }

        let mut bindings = Vec::new();
        let target = match target {
            Target::Var(name) => Target::Var(name),
            Target::Index(array, index) => {
                let array_name = format!("__thaw_update_array_{}", self.next_binding);
                self.next_binding += 1;
                let array_type = HirType::Array(Box::new(HirType::F64));
                self.scope.insert(array_name.clone(), array_type.clone());
                bindings.push((array_name.clone(), array_type, array));

                let index_name = format!("__thaw_update_index_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(index_name.clone(), HirType::F64);
                bindings.push((index_name.clone(), HirType::F64, *index));
                Target::Index(HirExpr::Var(array_name), Box::new(HirExpr::Var(index_name)))
            }
            Target::Prop(object, object_type, field) => {
                let object_name = format!("__thaw_update_object_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(object_name.clone(), object_type.clone());
                bindings.push((object_name.clone(), object_type.clone(), object));
                Target::Prop(HirExpr::Var(object_name), object_type, field)
            }
            Target::Dictionary(object, key, element) => {
                let object_name = format!("__thaw_update_dictionary_{}", self.next_binding);
                self.next_binding += 1;
                let object_type = HirType::Dictionary(Box::new(element.clone()));
                self.scope.insert(object_name.clone(), object_type.clone());
                bindings.push((object_name.clone(), object_type, object));

                let key_name = format!("__thaw_update_key_{}", self.next_binding);
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
                let object_name = format!("__thaw_update_json_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(object_name.clone(), HirType::Json);
                bindings.push((object_name.clone(), HirType::Json, object));

                let index_name = format!("__thaw_update_index_{}", self.next_binding);
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
        let old_name = format!("__thaw_update_old_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(old_name.clone(), HirType::F64);
        let current = if let Target::Index(array, index) = &target {
            self.lower_array_index(
                array.clone(), HirType::Array(Box::new(HirType::F64)), HirType::F64,
                index.as_ref().clone(),
            )?
        } else {
            self.lower_assignment_target_read(&target)?
        };
        bindings.push((old_name.clone(), HirType::F64, self.coerce_primitive_to_number(current)?));
        let old = HirExpr::Var(old_name);
        let updated = HirExpr::BinOp(op, Box::new(old.clone()), Box::new(one));
        let result = if update.prefix {
            let updated_name = format!("__thaw_update_new_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(updated_name.clone(), HirType::F64);
            bindings.push((updated_name.clone(), HirType::F64, updated));
            HirExpr::Block(vec![
                HirStmt::Expr(self.lower_assignment_target_write(target, HirExpr::Var(updated_name.clone()))?),
                HirStmt::Return(Some(HirExpr::Var(updated_name))),
            ])
        } else {
            HirExpr::Block(vec![
                HirStmt::Expr(self.lower_assignment_target_write(target, updated)?),
                HirStmt::Return(Some(old)),
            ])
        };
        self.wrap_call_argument_bindings(result, &bindings)
    }
}
