impl<'a> FnLowerer<'a> {
    /// Resolves `expr` for use as an assignment's intermediate/target
    /// container, walking through `.field`/`[index]` accesses on a `Json`
    /// (`any`-typed) value via a *mutable* chain instead of an ordinary
    /// read. An ordinary read (`self.lower_expr`, `thaw_json_get`/
    /// `thaw_json_index`) always clones, so a nested assignment
    /// (`a.b.c = x`) would resolve `a.b` to a disconnected copy and write
    /// into that instead of `a`'s own nested object -- silently doing
    /// nothing observable, confirmed against real Node (`a.b.c = x` then
    /// reading `a.b.c` back still shows the old value).
    /// `__thaw_json_get_mut`/`__thaw_json_index_get_mut` instead return a
    /// pointer *into* the parent's own storage, so the final write at the
    /// bottom of the chain reaches the original.
    ///
    /// Bottoms out at a plain variable, or any non-`Json`/non-chain
    /// sub-expression, which `lower_expr` already resolves correctly (a
    /// `Json`-typed local already holds its own pointer -- no clone
    /// happens reading a bare variable -- and every other native value
    /// type is already pointer-identity by construction).
    fn lower_json_mutable_chain(&mut self, expr: &Expr) -> Result<HirExpr, String> {
        let Expr::Member(member) = expr else {
            return self.lower_expr(expr);
        };
        let plain_inner = self.lower_expr(&member.obj)?;
        let inner_type = self.infer_expr_type(&plain_inner)?;
        if inner_type != HirType::Json {
            return self.lower_expr(expr);
        }
        let inner = self.lower_json_mutable_chain(&member.obj)?;
        match &member.prop {
            MemberProp::Ident(prop) => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_get_mut".to_string())),
                vec![inner, HirExpr::Lit(HirLit::Str(prop.sym.to_string()))],
            )),
            MemberProp::Computed(computed) => {
                let key = self.lower_expr(&computed.expr)?;
                if self.infer_expr_type(&key)? == HirType::F64 {
                    Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_index_get_mut".to_string())),
                        vec![inner, key],
                    ))
                } else {
                    let key = self.coerce_primitive_to_string(key)?;
                    Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_json_get_mut".to_string())),
                        vec![inner, key],
                    ))
                }
            }
            _ => self.lower_expr(expr),
        }
    }

    /// Resolves a computed assignment/update target without confusing the
    /// pointer-compatible array, object, string and JSON layouts.
    fn lower_computed_target(
        &mut self,
        member: &MemberExpr,
        computed: &ComputedPropName,
    ) -> Result<Target, String> {
        let object = self.lower_json_mutable_chain(&member.obj)?;
        let object_type = self.infer_expr_type(&object)?;
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
                    HirType::Str => {
                        Ok(Target::Dictionary(object, Box::new(key), HirType::Json))
                    }
                    HirType::F64 => Ok(Target::JsonIndex(object, Box::new(key))),
                    other => Err(format!(
                        "JSON assignment key must be string or number, got {other:?}"
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
                match &member.prop {
                    MemberProp::Computed(computed) => self.lower_computed_target(member, computed),
                    MemberProp::Ident(prop) => {
                        if let Expr::Ident(class) = member.obj.as_ref() {
                            let symbol =
                                class_static_field_symbol(class.sym.as_ref(), prop.sym.as_ref());
                            if self.scope.contains_key(&symbol) {
                                return Ok(Target::Var(symbol));
                            }
                        }
                        let obj = self.lower_json_mutable_chain(&member.obj)?;
                        let obj_ty = self.infer_expr_type(&obj)?;
                        match &obj_ty {
                            HirType::Object(fields)
                                if fields.iter().any(|(n, _)| n == prop.sym.as_str()) =>
                            {
                                Ok(Target::Prop(obj, obj_ty.clone(), prop.sym.to_string()))
                            }
                            HirType::Dictionary(element)
                                if matches!(
                                    element.as_ref(),
                                    HirType::F64 | HirType::Str | HirType::Bool | HirType::Json
                                ) =>
                            {
                                Ok(Target::Dictionary(
                                    obj,
                                    Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                                    *element.clone(),
                                ))
                            }
                            HirType::Json => Ok(Target::Dictionary(
                                obj,
                                Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                                HirType::Json,
                            )),
                            HirType::JsValue | HirType::Dynamic => Ok(Target::DynamicProperty(
                                obj,
                                Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                            )),
                            other => Err(format!(
                                "cannot assign to `.{}` on a value of type {other:?}",
                                prop.sym
                            )),
                        }
                    }
                    _ => Err(
                        "only `arr[i] = ...` / `obj.field = ...` member assignment is supported"
                            .into(),
                    ),
                }
            }
            _ => Err("unsupported assignment target".into()),
        }
    }
}
