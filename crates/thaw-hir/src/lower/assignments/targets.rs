impl<'a> FnLowerer<'a> {
    /// A Json read now shares its native Array/Object allocation (or its
    /// live host lease). Thus an intermediate `.field`/`[index]` read keeps
    /// the same receiver identity for nested assignment. The old mutable
    /// slot ABI cannot represent a QuickJS property, and is unnecessary
    /// with shared native values.
    fn lower_json_mutable_chain(&mut self, expr: &Expr) -> Result<HirExpr, String> {
        self.lower_expr(expr)
    }

    fn lower_computed_target_from_receiver(
        &mut self,
        object: HirExpr,
        object_type: HirType,
        computed: &ComputedPropName,
    ) -> Result<Target, String> {
        if let Some(base @ HirType::Array(_)) = native_base_type(&object_type).cloned() {
            let object = HirExpr::PropAccess(
                Box::new(object),
                object_type,
                NATIVE_BASE_FIELD.to_string(),
            );
            return self.lower_computed_target_from_receiver(object, base, computed);
        }
        match &object_type {
            HirType::Array(_) => {
                let index = self.lower_expr(&computed.expr)?;
                self.expect_type(&HirType::F64, &index, "index expression")?;
                Ok(Target::Index(object, Box::new(index)))
            }
            HirType::Object(fields) => {
                let Expr::Lit(Lit::Str(key)) = computed.expr.as_ref() else {
                    return Err("computed object assignment key must be a string literal".into());
                };
                let key = key.value.to_string_lossy().into_owned();
                if fields.iter().any(|(name, _)| name == &key) {
                    Ok(Target::Prop(object, object_type, key))
                } else {
                    Err(format!("object has no field `{key}`"))
                }
            }
            HirType::Dictionary(element) => {
                let key = self.lower_expr(&computed.expr)?;
                let key = self.coerce_primitive_to_string(key)?;
                // A `T | undefined` dictionary stores its present entries as plain `T`.
                let element = match element.as_ref() {
                    HirType::Optional(inner) if matches!(inner.as_ref(), HirType::F64 | HirType::Str | HirType::Bool) => inner.clone(),
                    _ => element.clone(),
                };
                if !matches!(
                    element.as_ref(),
                    HirType::F64 | HirType::Str | HirType::Bool | HirType::Json
                ) {
                    return Err(format!(
                        "unsupported dictionary value type {:?}",
                        element.as_ref()
                    ));
                }
                Ok(Target::Dictionary(object, Box::new(key), *element.clone()))
            }
            HirType::Json => {
                let key = self.lower_expr(&computed.expr)?;
                match self.infer_expr_type(&key)? {
                    // A symbol is stored under its private string token, like a
                    // dictionary key.
                    HirType::Str | HirType::Symbol => {
                        Ok(Target::Dictionary(object, Box::new(key), HirType::Json))
                    }
                    HirType::F64 => Ok(Target::JsonIndex(object, Box::new(key))),
                    other => Err(format!(
                        "JSON assignment key must be string or number, got {other:?}"
                    )),
                }
            }
            // `err[key] = v` on an Error value (a framed string); a plain string rejects it at run time.
            HirType::Str => {
                let key = self.lower_expr(&computed.expr)?;
                // A numeric key (`str[0] = x`) stays rejected: strings are immutable indexed values.
                match self.infer_expr_type(&key)? {
                    HirType::Str => Ok(Target::ErrorProp(object, Box::new(key))),
                    other => Err(format!(
                        "cannot assign through a computed key of type {other:?} on a value of type {object_type:?}"
                    )),
                }
            }
            HirType::JsValue | HirType::Dynamic => {
                let key = self.lower_expr(&computed.expr)?;
                let key = match self.infer_expr_type(&key)? {
                    HirType::Str | HirType::Symbol | HirType::Dynamic => key,
                    HirType::F64 => self.coerce_primitive_to_string(key)?,
                    other => {
                        return Err(format!(
                            "dynamic assignment key must be string, number, or symbol, got {other:?}"
                        ))
                    }
                };
                Ok(Target::DynamicProperty(object, Box::new(key)))
            }
            _ => Err(format!(
                "cannot assign through a computed key on a value of type {object_type:?}"
            )),
        }
    }


    fn lower_member_target_from_receiver(
        &mut self,
        member: &MemberExpr,
        object: HirExpr,
        object_type: HirType,
    ) -> Result<Target, String> {
        // `stack[i] = v` / `stack.length = n` on a class extending `Array`
        // write through to the wrapped array.
        if let Some(base @ HirType::Array(_)) = native_base_type(&object_type).cloned() {
            let is_own = matches!(&member.prop, MemberProp::Ident(prop)
                if Self::class_receiver_has_own_field(&object_type, prop.sym.as_ref()));
            if !is_own {
                let object = HirExpr::PropAccess(
                    Box::new(object),
                    object_type,
                    NATIVE_BASE_FIELD.to_string(),
                );
                return self.lower_member_target_from_receiver(member, object, base);
            }
        }
        match &member.prop {
            MemberProp::Computed(computed) => {
                self.lower_computed_target_from_receiver(object, object_type, computed)
            }
            MemberProp::Ident(prop) => match &object_type {
                HirType::Object(fields)
                    if fields.iter().any(|(name, _)| name == prop.sym.as_str()) =>
                {
                    Ok(Target::Prop(object, object_type.clone(), prop.sym.to_string()))
                }
                HirType::Dictionary(element)
                    if matches!(element.as_ref(), HirType::F64 | HirType::Str | HirType::Bool | HirType::Json) =>
                {
                    Ok(Target::Dictionary(
                        object,
                        Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                        *element.clone(),
                    ))
                }
                // A `T | undefined` dictionary stores its present entries as plain `T`.
                HirType::Dictionary(element)
                    if matches!(element.as_ref(), HirType::Optional(inner)
                        if matches!(inner.as_ref(), HirType::F64 | HirType::Str | HirType::Bool)) =>
                {
                    let HirType::Optional(inner) = element.as_ref() else { unreachable!() };
                    Ok(Target::Dictionary(
                        object,
                        Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                        inner.as_ref().clone(),
                    ))
                }
                HirType::Json => Ok(Target::Dictionary(
                    object,
                    Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                    HirType::Json,
                )),
                HirType::JsValue | HirType::Dynamic => Ok(Target::DynamicProperty(
                    object,
                    Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                )),
                // `record.stack = text` where the class declares no `stack`: kept in the runtime
                // side table `Error.captureStackTrace` also writes (see `lower_error_prop_assign`).
                HirType::Object(fields)
                    if prop.sym == *"stack"
                        && !fields.iter().any(|(name, _)| {
                            name == "stack" || name == "__thaw_getter_stack" || name == "__thaw_setter_stack"
                        }) =>
                {
                    Ok(Target::ErrorProp(
                        object,
                        Box::new(HirExpr::Lit(HirLit::Str("stack".to_string()))),
                    ))
                }
                // `err.code = x`: an Error is a framed string; its own properties live in the
                // runtime side table (see thaw-runtime `thaw_error_set_prop`).
                HirType::Str => Ok(Target::ErrorProp(
                    object,
                    Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                )),
                // `catch (e: any) { e.k = 1 }`: a native error is a framed string, so first
                // rebind the catch slot to an Error-shaped Json object, then set on that.
                HirType::Union(_)
                    if object_type == crate::caught_exception_carrier_type()
                        && matches!(&object, HirExpr::Var(_)) =>
                {
                    let HirExpr::Var(name) = &object else { unreachable!() };
                    let HirType::Union(members) = &object_type else { unreachable!() };
                    let call = |function: &str, args: Vec<HirExpr>| {
                        HirExpr::Call(Box::new(HirExpr::Var(function.into())), args)
                    };
                    let text = || HirExpr::UnionValue(Box::new(object.clone()), 3, members.clone());
                    // The Json copy of the framed error carries its name, message, stack and props.
                    let error_object = call("__thaw_json_receiver_string", vec![text()]);
                    let is_error_text = HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(Box::new(object.clone()), members.clone())),
                        Box::new(HirExpr::Lit(HirLit::F64(3.0))),
                    );
                    let is_native_object = HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::UnionTag(Box::new(object.clone()), members.clone())),
                        Box::new(HirExpr::Lit(HirLit::F64(6.0))),
                    );
                    let snapshot = self.caught_carrier_as_json(object.clone())?;
                    let rebind = |value: HirExpr| {
                        vec![HirStmt::Expr(HirExpr::Assign(
                            name.clone(),
                            Box::new(HirExpr::UnionInject(Box::new(value), 7, members.clone())),
                        ))]
                    };
                    let promote = HirStmt::If(
                        is_error_text,
                        vec![HirStmt::If(
                            call("__thaw_error_is_error", vec![text()]),
                            rebind(error_object),
                            Vec::new(),
                        )],
                        vec![HirStmt::If(is_native_object, rebind(snapshot), Vec::new())],
                    );
                    let json = self.caught_carrier_as_json(object.clone())?;
                    Ok(Target::Dictionary(
                        HirExpr::Block(vec![promote, HirStmt::Return(Some(json))]),
                        Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                        HirType::Json,
                    ))
                }
                other => Err(format!(
                    "cannot assign to `.{}` on a value of type {other:?}",
                    prop.sym,
                )),
            },
            _ => Err("only `arr[i] = ...` / `obj.field = ...` member assignment is supported".into()),
        }
    }

    fn lower_assign_target(&mut self, target: &AssignTarget) -> Result<Target, String> {
        let AssignTarget::Simple(simple) = target else {
            return Err("destructuring assignment targets are not supported".into());
        };
        match simple {
            SimpleAssignTarget::Ident(binding) => {
                Ok(Target::Var(self.resolve_binding(binding.id.sym.as_ref())))
            }
            SimpleAssignTarget::Member(member) => {
                if matches!(member.obj.as_ref(), Expr::This(_)) && self.class_static_context {
                    if let Some(property) = member_property_name(&member.prop) {
                        let class = self
                            .class_context
                            .as_deref()
                            .expect("static assignment retains its class context");
                        let symbol = class_static_field_symbol(class, &property);
                        if self.scope.contains_key(&symbol) {
                            return Ok(Target::Var(symbol));
                        }
                    }
                }
                if let (Expr::Ident(class), Some(property)) =
                    (member.obj.as_ref(), member_property_name(&member.prop))
                {
                    let symbol = class_static_field_symbol(class.sym.as_ref(), &property);
                    if self.scope.contains_key(&symbol) {
                        return Ok(Target::Var(symbol));
                    }
                }
                let mut object = self.lower_json_mutable_chain(&member.obj)?;
                let mut object_type = self.infer_expr_type(&object)?;
                // `q[i] = v` on an element that came out of an array pattern (`([q]) => ...` over
                // `Promise.all`) is `Optional(Array)`; unwrap it like the read path does.
                if let HirType::Optional(payload) = &object_type {
                    if matches!(payload.as_ref(), HirType::Array(_) | HirType::Bytes) {
                        if let MemberProp::Computed(_) = &member.prop {
                            let reject = self.receiver_requires_narrowing(&member.obj);
                            (object, object_type) = self.unwrap_required_optional_member(object, object_type, "computed property", reject)?;
                            // Index writes are typed as `Array` (see `infer_expr_type`); a buffer element is the same layout.
                            if object_type == HirType::Bytes {
                                object_type = HirType::Array(Box::new(HirType::F64));
                            }
                        }
                    }
                }
                // `c.v = x` / `c.next = n` on a maybe-absent object binding whose narrowing the
                // flow analysis dropped (e.g. `c` is reassigned later in the loop): unwrap it with a
                // runtime `TypeError` guard, like the read path.
                if let (HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload), MemberProp::Ident(property)) = (&object_type, &member.prop) {
                    if matches!(payload.expand(), HirType::Object(_)) {
                        let reject = self.receiver_requires_narrowing(&member.obj);
                        (object, object_type) = self.unwrap_required_optional_member(object, object_type, property.sym.as_ref(), reject)?;
                    }
                }
                if let HirType::NonArrowReceiver(declared) = object_type {
                    let access = match &member.prop {
                        MemberProp::Ident(property) => {
                            NonArrowThisAccess::Write(property.sym.to_string())
                        }
                        MemberProp::Computed(computed) => match computed.expr.as_ref() {
                            Expr::Lit(Lit::Str(key)) => {
                                NonArrowThisAccess::Write(key.value.to_string_lossy().into_owned())
                            }
                            _ => NonArrowThisAccess::Value,
                        },
                        _ => NonArrowThisAccess::Value,
                    };
                    object = HirExpr::NonArrowThis {
                        payload: Box::new(object),
                        declared: *declared.clone(),
                        access,
                    };
                    object_type = *declared;
                }
                let object = if let (MemberProp::Ident(property), HirExpr::NonArrowThis { .. }) = (&member.prop, &object) {
                    object.with_non_arrow_this_access(NonArrowThisAccess::Write(property.sym.to_string()))
                } else {
                    object
                };
                let object_type = match &object {
                    HirExpr::NonArrowThis { declared, .. } => declared.clone(),
                    _ => object_type,
                };
                self.lower_member_target_from_receiver(member, object, object_type)
            }
            _ => Err("unsupported assignment target".into()),
        }
    }
}
