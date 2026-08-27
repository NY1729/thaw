impl<'a> FnLowerer<'a> {
    /// Resolves a computed assignment/update target without confusing the
    /// pointer-compatible array, object, string and JSON layouts.
    fn lower_computed_target(
        &mut self,
        member: &MemberExpr,
        computed: &ComputedPropName,
    ) -> Result<Target, String> {
        let object = self.lower_expr(&member.obj)?;
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
                        let obj = self.lower_expr(&member.obj)?;
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
