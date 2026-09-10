impl<'a> FnLowerer<'a> {
    fn initializer_callback_references(expression: &Expr, name: &str) -> bool {
        use swc_ecma_visit::{Visit, VisitWith};

        struct Finder<'a> {
            name: &'a str,
            found: bool,
        }

        impl Visit for Finder<'_> {
            fn visit_ident(&mut self, identifier: &swc_ecma_ast::Ident) {
                self.found |= identifier.sym == self.name;
            }
        }

        let Expr::Call(call) = expression else {
            return false;
        };
        call.args.iter().any(|argument| {
            if !matches!(argument.expr.as_ref(), Expr::Arrow(_) | Expr::Fn(_)) {
                return false;
            }
            let mut finder = Finder { name, found: false };
            argument.expr.visit_with(&mut finder);
            finder.found
        })
    }

    fn lower_var_decl(&mut self, var_decl: &VarDecl) -> Result<Vec<HirStmt>, String> {
        let mut statements = Vec::new();
        for decl in &var_decl.decls {
            if let Pat::Ident(binding) = &decl.name {
                let name = binding.id.sym.to_string();
                let init = decl
                    .init
                    .as_deref()
                    .ok_or_else(|| format!("`{name}` needs an initializer"))?;
                if let Expr::Yield(yield_expr) = init {
                    let Some((values, element, input, input_type, returns, _)) =
                        self.generator_yields.clone()
                    else {
                        return Err("`yield` is only valid inside a generator function".into());
                    };
                    let (emission, delegated) = self.lower_generator_yield_emission(
                        yield_expr,
                        &values,
                        &element,
                        &input,
                        &returns,
                    )?;
                    let declared = binding
                        .type_ann
                        .as_ref()
                        .map(|annotation| {
                            lower_ts_type(
                                &annotation.type_ann,
                                self.interfaces,
                                self.generic_interfaces,
                            )
                        })
                        .transpose()?
                        .unwrap_or_else(|| {
                            delegated
                                .as_ref()
                                .map(|(_, ty)| ty.clone())
                                .unwrap_or_else(|| input_type.clone())
                        });
                    let resumed = delegated
                        .map(|(value, _)| value)
                        .unwrap_or_else(|| HirExpr::Var(input));
                    let resumed = self.coerce_to_declared(&declared, resumed)?;
                    let hir_name = self.bind_local(&name, declared.clone());
                    statements.extend(emission);
                    statements.push(HirStmt::Let(hir_name, declared, resumed));
                    continue;
                }
                let correlated_alias_source = self.expression_identifier_alias_source(init);
                if let (Expr::Arrow(arrow), Some(annotation)) = (init, binding.type_ann.as_ref()) {
                    if arrow.type_params.is_some() {
                        if let Some((expected, callable_name)) =
                            self.generic_callable_annotation_signature(&annotation.type_ann)?
                        {
                            let actual = self.generic_arrow_signature(arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_arrows.insert(hir_name, arrow.clone());
                            continue;
                        }
                    }
                }
                if let (Expr::Fn(function), Some(annotation)) = (init, binding.type_ann.as_ref()) {
                    if function.function.type_params.is_some() {
                        if let Some((expected, callable_name)) =
                            self.generic_callable_annotation_signature(&annotation.type_ann)?
                        {
                            let arrow = function_expression_as_arrow(function)?;
                            let actual = self.generic_arrow_signature(&arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_arrows.insert(hir_name.clone(), arrow);
                            if named_function_is_recursive(function) {
                                self.generic_arrow_self_names.insert(
                                    hir_name,
                                    function.ident.as_ref().unwrap().sym.to_string(),
                                );
                            }
                            continue;
                        }
                    }
                }
                if let (Expr::Ident(identifier), Some(annotation)) =
                    (init, binding.type_ann.as_ref())
                {
                    if let Some((expected, callable_name)) =
                        self.generic_callable_annotation_signature(&annotation.type_ann)?
                    {
                        if let Some(actual) = self.signatures.get(identifier.sym.as_ref()) {
                            if !actual.generic_type_params.is_empty() {
                                self.validate_generic_callable_shape(
                                    &expected,
                                    actual,
                                    &callable_name,
                                )?;
                                let hir_name = self.bind_local(&name, HirType::Dynamic);
                                self.generic_named_templates
                                    .insert(hir_name, identifier.sym.to_string());
                                continue;
                            }
                        }
                    }
                }
                if let (Expr::Ident(identifier), Some(annotation)) =
                    (init, binding.type_ann.as_ref())
                {
                    if let Some((expected, callable_name)) =
                        self.generic_callable_annotation_signature(&annotation.type_ann)?
                    {
                        let source_name = self.resolve_binding(identifier.sym.as_ref());
                        if let Some(arrow) = self.generic_arrows.get(&source_name).cloned() {
                            let actual = self.generic_arrow_signature(&arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_arrows.insert(hir_name.clone(), arrow);
                            if let Some(self_name) =
                                self.generic_arrow_self_names.get(&source_name).cloned()
                            {
                                self.generic_arrow_self_names.insert(hir_name, self_name);
                            }
                            continue;
                        }
                        if let Some(target) =
                            self.generic_named_templates.get(&source_name).cloned()
                        {
                            let actual = self
                                .signatures
                                .get(&target)
                                .expect("named generic template target");
                            self.validate_generic_callable_shape(
                                &expected,
                                actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_named_templates.insert(hir_name, target);
                            continue;
                        }
                    }
                }
                if let (Expr::Ident(identifier), None) = (init, binding.type_ann.as_ref()) {
                    let source_name = self.resolve_binding(identifier.sym.as_ref());
                    if let Some(arrow) = self.generic_arrows.get(&source_name).cloned() {
                        let hir_name = self.bind_local(&name, HirType::Dynamic);
                        self.generic_arrows.insert(hir_name.clone(), arrow);
                        if let Some(self_name) =
                            self.generic_arrow_self_names.get(&source_name).cloned()
                        {
                            self.generic_arrow_self_names.insert(hir_name, self_name);
                        }
                        continue;
                    }
                    if let Some(target) = self.generic_named_templates.get(&source_name).cloned() {
                        let hir_name = self.bind_local(&name, HirType::Dynamic);
                        self.generic_named_templates.insert(hir_name, target);
                        continue;
                    }
                }
                let annotated = binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        lower_ts_type(
                            &annotation.type_ann,
                            self.interfaces,
                            self.generic_interfaces,
                        )
                    })
                    .transpose()?;
                let recursive_binding_type = if Self::initializer_callback_references(init, &name)
                {
                    annotated.clone().or_else(|| {
                        let Expr::Call(call) = init else {
                            return None;
                        };
                        let Callee::Expr(callee) = &call.callee else {
                            return None;
                        };
                        let Expr::Ident(callee) = callee.as_ref() else {
                            return None;
                        };
                        self.signatures
                            .get(&self.resolve_binding(callee.sym.as_ref()))
                            .map(|signature| signature.ret.clone())
                            .or_else(|| {
                                matches!(
                                    callee.sym.as_ref(),
                                    "setTimeout"
                                        | "setInterval"
                                        | "setImmediate"
                                )
                                .then_some(HirType::JsValue)
                            })
                    })
                } else {
                    None
                };
                let recursive_binding = recursive_binding_type
                    .as_ref()
                    .map(|ty| self.bind_local(&name, ty.clone()));
                if let Expr::Fn(function) = init {
                    if let Some((hir_name, ty, value)) = self.lower_recursive_function_expression(
                        &name,
                        function,
                        annotated.as_ref(),
                    )? {
                        statements.push(HirStmt::Let(hir_name, ty, value));
                        continue;
                    }
                }
                if annotated.is_none()
                    && (matches!(init, Expr::Arrow(arrow) if arrow.type_params.is_some())
                        || matches!(init, Expr::Fn(function) if function.function.type_params.is_some()))
                {
                    let recursive_self = match init {
                        Expr::Fn(function) if named_function_is_recursive(function) => {
                            function.ident.as_ref().map(|name| name.sym.to_string())
                        }
                        _ => None,
                    };
                    let arrow = match init {
                        Expr::Arrow(arrow) => arrow.clone(),
                        Expr::Fn(function) => function_expression_as_arrow(function)?,
                        _ => unreachable!(),
                    };
                    if arrow.is_async && !arrow.is_generator {
                        return Err(
                            "async generic arrow variables are not supported".into(),
                        );
                    }
                    let hir_name = self.bind_local(&name, HirType::Dynamic);
                    self.generic_arrows.insert(hir_name.clone(), arrow);
                    if let Some(self_name) = recursive_self {
                        self.generic_arrow_self_names.insert(hir_name, self_name);
                    }
                    continue;
                }
                let native_method_value = self.native_instance_method_value(init).or_else(|| {
                    let Expr::Ident(identifier) = init else {
                        return None;
                    };
                    self.native_method_values
                        .get(&self.resolve_binding(identifier.sym.as_ref()))
                        .cloned()
                });
                let propagated_discriminants = self.expression_union_discriminants(init);
                let propagated_array_discriminants =
                    self.expression_array_element_discriminants(init);
                let propagated_nested_array_discriminants =
                    self.expression_nested_array_discriminants(init);
                let propagated_object_array_property_discriminants =
                    self.expression_object_array_property_discriminants(init);
                let propagated_object_function_property_discriminants =
                    self.expression_object_function_property_discriminants(init);
                let propagated_function_discriminants =
                    self.expression_function_discriminants(init);
                let propagated_function_array_discriminants =
                    self.expression_function_array_discriminants(init);
                let propagated_function_nested_array_discriminants =
                    self.expression_function_nested_array_discriminants(init);
                let propagated_function_object_array_property_discriminants =
                    self.expression_function_object_array_property_discriminants(init);
                let propagated_function_object_function_property_discriminants =
                    self.expression_function_object_function_property_discriminants(init);
                let dynamic_call = matches!(
                    init,
                    Expr::Call(call)
                        if matches!(
                            &call.callee,
                            Callee::Expr(callee)
                                if match callee.as_ref() {
                                    Expr::Ident(identifier) => self.scope
                                        .get(&self.resolve_binding(identifier.sym.as_ref()))
                                        == Some(&HirType::JsValue),
                                    Expr::Member(member) => self
                                        .infer_member_receiver_type(&member.obj)
                                        == Some(HirType::JsValue),
                                    _ => false,
                                }
                        )
                );
                let inferred_expected = if annotated.is_none() && dynamic_call {
                    if self.awaited_bindings.contains(&name) {
                        Some(HirType::Dynamic)
                    } else if self.member_receiver_bindings.contains(&name) {
                        Some(HirType::JsValue)
                    } else {
                        None
                    }
                } else {
                    None
                };
                let value = match (init, annotated.as_ref()) {
                    (Expr::Arrow(arrow), Some(HirType::Function(params, ret))) => {
                        self.lower_contextual_arrow(arrow, params, Some(ret))?
                    }
                    (Expr::Arrow(arrow), Some(HirType::CallableFunction(params, _, rest, ret))) => {
                        let mut abi_params = params.clone();
                        if let Some(rest) = rest {
                            abi_params.push(HirType::Array(rest.clone()));
                        }
                        self.lower_contextual_arrow(arrow, &abi_params, Some(ret))?
                    }
                    (Expr::Fn(function), Some(HirType::Function(params, ret))) => {
                        let arrow = function_expression_as_arrow(function)?;
                        self.lower_contextual_arrow(&arrow, params, Some(ret))?
                    }
                    (Expr::Fn(function), Some(HirType::CallableFunction(params, _, rest, ret))) => {
                        let arrow = function_expression_as_arrow(function)?;
                        let mut abi_params = params.clone();
                        if let Some(rest) = rest {
                            abi_params.push(HirType::Array(rest.clone()));
                        }
                        self.lower_contextual_arrow(&arrow, &abi_params, Some(ret))?
                    }
                    (
                        Expr::Ident(identifier),
                        Some(HirType::Function(_, _) | HirType::CallableFunction(..)),
                    ) => {
                        let source = self.resolve_binding(identifier.sym.as_ref());
                        if self.scope.contains_key(&source) {
                            self.lower_expr(init)?
                        } else {
                            let signature = self.signatures.get(&source).ok_or_else(|| {
                                format!("unknown function value `{}`", identifier.sym)
                            })?;
                            if signature.is_extern || !signature.generic_type_params.is_empty() {
                                return Err(format!(
                                    "function value `{}` needs a monomorphic implementation",
                                    identifier.sym
                                ));
                            }
                            let ret = if signature.is_async {
                                HirType::Promise(Box::new(signature.ret.clone()))
                            } else {
                                signature.ret.clone()
                            };
                            HirExpr::FunctionRef(source, signature.params.clone(), ret)
                        }
                    }
                    _ => self.lower_expr_with_expected_type(
                        init,
                        annotated.as_ref().or(inferred_expected.as_ref()),
                    )?,
                };

                let actual_type = self.infer_expr_type(&value).map_err(|error| {
                    format!(
                        "cannot infer the type of `{name}`: {error} \
                         (add an explicit type annotation)"
                    )
                })?;
                let ty = match annotated {
                    Some(ty) => ty,
                    None => actual_type.clone(),
                };
                let mut value = self.coerce_to_declared(&ty, value)?;

                let storage_type = if class_name_from_type(&actual_type).is_some()
                    && matches!(&ty, HirType::Object(fields) if fields.iter().any(|(_, field)| *field == HirType::Dynamic))
                {
                    actual_type.clone()
                } else {
                    ty.clone()
                };
                let hir_name = if let Some(hir_name) = recursive_binding {
                    self.scope.insert(hir_name.clone(), storage_type.clone());
                    value = HirExpr::RecursiveClosure(
                        hir_name.clone(),
                        storage_type.clone(),
                        Box::new(value),
                    );
                    hir_name
                } else {
                    self.bind_local(&name, storage_type.clone())
                };
                if class_name_from_type(&actual_type).is_some() && actual_type != storage_type {
                    self.native_class_aliases
                        .insert(hir_name.clone(), actual_type);
                }
                if let Some(source) = correlated_alias_source {
                    self.propagate_destructured_union_alias(&source, &hir_name);
                }
                if let Some(annotation) = &binding.type_ann {
                    let discriminants =
                        object_union_discriminants(&annotation.type_ann, self.generic_interfaces);
                    if !discriminants.is_empty() {
                        self.union_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    let element_discriminants = array_element_union_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !element_discriminants.is_empty() {
                        self.array_element_discriminants
                            .insert(hir_name.clone(), element_discriminants);
                    }
                    let nested_discriminants = nested_array_union_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !nested_discriminants.is_empty() {
                        self.nested_array_discriminants
                            .insert(hir_name.clone(), nested_discriminants);
                    }
                    let property_discriminants = object_array_property_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !property_discriminants.is_empty() {
                        self.object_array_property_discriminants
                            .insert(hir_name.clone(), property_discriminants);
                    }
                    let function_property_discriminants = object_function_property_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !function_property_discriminants.is_empty() {
                        self.object_function_property_discriminants
                            .insert(hir_name.clone(), function_property_discriminants);
                    }
                    let return_discriminants = function_return_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !return_discriminants.is_empty() {
                        self.function_value_discriminants
                            .insert(hir_name.clone(), return_discriminants);
                    }
                    let return_array_discriminants = function_return_array_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !return_array_discriminants.is_empty() {
                        self.function_value_array_discriminants
                            .insert(hir_name.clone(), return_array_discriminants);
                    }
                    let return_nested_discriminants = function_return_nested_array_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !return_nested_discriminants.is_empty() {
                        self.function_value_nested_array_discriminants
                            .insert(hir_name.clone(), return_nested_discriminants);
                    }
                    let return_property_discriminants =
                        function_return_object_array_property_discriminants(
                            &annotation.type_ann,
                            self.generic_interfaces,
                        );
                    if !return_property_discriminants.is_empty() {
                        self.function_value_object_array_property_discriminants
                            .insert(hir_name.clone(), return_property_discriminants);
                    }
                    let return_function_property_discriminants =
                        function_return_object_function_property_discriminants(
                            &annotation.type_ann,
                            self.generic_interfaces,
                        );
                    if !return_function_property_discriminants.is_empty() {
                        self.function_value_object_function_property_discriminants
                            .insert(hir_name.clone(), return_function_property_discriminants);
                    }
                } else if let Some(discriminants) = propagated_discriminants {
                    self.union_discriminants
                        .insert(hir_name.clone(), discriminants);
                }
                if binding.type_ann.is_none() {
                    if let Some(discriminants) = propagated_array_discriminants {
                        self.array_element_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_nested_array_discriminants {
                        self.nested_array_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_object_array_property_discriminants {
                        self.object_array_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_object_function_property_discriminants {
                        self.object_function_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                }
                if binding.type_ann.is_none() {
                    if let Some(discriminants) = propagated_function_discriminants {
                        self.function_value_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_function_array_discriminants {
                        self.function_value_array_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_function_nested_array_discriminants {
                        self.function_value_nested_array_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) =
                        propagated_function_object_array_property_discriminants
                    {
                        self.function_value_object_array_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) =
                        propagated_function_object_function_property_discriminants
                    {
                        self.function_value_object_function_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                }
                if let Some(method) = native_method_value {
                    self.native_method_values.insert(hir_name.clone(), method);
                }
                statements.push(HirStmt::Let(hir_name, storage_type, value));
                continue;
            }

            let init = decl
                .init
                .as_deref()
                .ok_or("destructuring declarations need an initializer")?;
            let propagated_discriminants = self.expression_union_discriminants(init);
            let propagated_property_discriminants =
                self.expression_object_array_property_discriminants(init);
            let propagated_function_property_discriminants =
                self.expression_object_function_property_discriminants(init);
            let mut value = self.lower_expr(init)?;
            let annotation = match &decl.name {
                Pat::Array(pattern) => pattern.type_ann.as_ref(),
                Pat::Object(pattern) => pattern.type_ann.as_ref(),
                _ => None,
            };
            let ty = if let Some(annotation) = annotation {
                let ty = lower_ts_type(
                    &annotation.type_ann,
                    self.interfaces,
                    self.generic_interfaces,
                )?;
                value = self.coerce_to_declared(&ty, value)?;
                ty
            } else if matches!(&decl.name, Pat::Array(_)) {
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
                    if matches!(&decl.name, Pat::Object(_))
                        && elements.iter().all(|element| matches!(element, HirType::Object(_)))
            );
            let destructurable_dictionary =
                matches!((&decl.name, &ty), (Pat::Object(_), HirType::Dictionary(_)));
            if !matches!(ty, HirType::Object(_) | HirType::Tuple(_))
                && !destructurable_union
                && !destructurable_dictionary
            {
                return Err(format!(
                    "destructuring requires a fixed-shape object or tuple, got {ty:?}"
                ));
            }
            let temporary = format!("__thaw_destructure_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(temporary.clone(), ty.clone());
            let declared_discriminants = annotation.and_then(|annotation| {
                let discriminants =
                    object_union_discriminants(&annotation.type_ann, self.generic_interfaces);
                (!discriminants.is_empty()).then_some(discriminants)
            });
            if let Some(discriminants) = declared_discriminants.or(propagated_discriminants) {
                self.union_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            let declared_property_discriminants = annotation.and_then(|annotation| {
                let discriminants = object_array_property_discriminants(
                    &annotation.type_ann,
                    self.generic_interfaces,
                );
                (!discriminants.is_empty()).then_some(discriminants)
            });
            if let Some(discriminants) =
                declared_property_discriminants.or(propagated_property_discriminants)
            {
                self.object_array_property_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            let declared_function_property_discriminants = annotation.and_then(|annotation| {
                let discriminants = object_function_property_discriminants(
                    &annotation.type_ann,
                    self.generic_interfaces,
                );
                (!discriminants.is_empty()).then_some(discriminants)
            });
            if let Some(discriminants) = declared_function_property_discriminants
                .or(propagated_function_property_discriminants)
            {
                self.object_function_property_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            statements.push(HirStmt::Let(temporary.clone(), ty.clone(), value));
            self.lower_binding_pattern(&decl.name, HirExpr::Var(temporary), &ty, &mut statements)?;
        }
        Ok(statements)
    }
}
