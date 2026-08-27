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
                    let one = HirExpr::Lit(HirLit::F64(1.0));
                    if update.prefix {
                        let updated = HirExpr::BinOp(operator, Box::new(current), Box::new(one));
                        return Ok(HirExpr::Call(Box::new(HirExpr::Var(setter)), vec![updated]));
                    }
                    let old_name = format!("__thaw_static_update_old_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(old_name.clone(), HirType::F64);
                    let old = HirExpr::Var(old_name.clone());
                    let updated = HirExpr::BinOp(operator, Box::new(old.clone()), Box::new(one));
                    let result = HirExpr::Block(vec![
                        HirStmt::Expr(HirExpr::Call(Box::new(HirExpr::Var(setter)), vec![updated])),
                        HirStmt::Return(Some(old)),
                    ]);
                    return self
                        .wrap_call_argument_bindings(result, &[(old_name, HirType::F64, current)]);
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
                                self.lower_computed_target(member, computed)?
                            }
                            MemberProp::Ident(prop) => {
                                let object = self.lower_expr(&member.obj)?;
                                let object_type = self.infer_expr_type(&object)?;
                                match &object_type {
                                    HirType::Object(fields)
                                        if fields.iter().any(|(name, ty)| {
                                            name == prop.sym.as_str() && *ty == HirType::F64
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
                            self.lower_computed_target(member, computed)?
                        }
                        MemberProp::Ident(prop) => {
                            let object = self.lower_expr(&member.obj)?;
                            let object_type = self.infer_expr_type(&object)?;
                            match &object_type {
                                HirType::Object(fields)
                                    if fields.iter().any(|(name, ty)| {
                                        name == prop.sym.as_str() && *ty == HirType::F64
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

        let op = match update.op {
            UpdateOp::PlusPlus => BinOp::Add,
            UpdateOp::MinusMinus => BinOp::Sub,
        };
        let one = HirExpr::Lit(HirLit::F64(1.0));
        let current = target_to_read_expr(&target)?;
        self.expect_type(&HirType::F64, &current, "update operand")?;
        if update.prefix {
            let value = HirExpr::BinOp(op, Box::new(current), Box::new(one));
            return Ok(build_assign(target, value));
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
        };
        let old_name = format!("__thaw_update_old_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(old_name.clone(), HirType::F64);
        bindings.push((
            old_name.clone(),
            HirType::F64,
            target_to_read_expr(&target)?,
        ));
        let old = HirExpr::Var(old_name);
        let updated = HirExpr::BinOp(op, Box::new(old.clone()), Box::new(one));
        let result = HirExpr::Block(vec![
            HirStmt::Expr(build_assign(target, updated)),
            HirStmt::Return(Some(old)),
        ]);
        self.wrap_call_argument_bindings(result, &bindings)
    }
}
