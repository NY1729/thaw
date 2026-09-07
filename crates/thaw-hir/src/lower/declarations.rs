#[allow(clippy::too_many_arguments)]
fn lower_callable_default_wrappers(
    symbol: &str,
    params: &[HirParam],
    patterns: &[Pat],
    receiver_count: usize,
    unbound_context: Option<(&str, bool)>,
    ret: &HirType,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
) -> Result<Vec<HirFunction>, String> {
    let Some(default_start) = trailing_omittable_start(patterns) else {
        return Ok(Vec::new());
    };
    let is_async = signatures
        .get(symbol)
        .or_else(|| {
            symbol
                .strip_suffix("__thaw_unbound")
                .and_then(|original| signatures.get(original))
        })
        .ok_or_else(|| format!("missing class wrapper signature for `{symbol}`"))?
        .is_async;
    let mut wrappers = Vec::new();
    for arity in default_start..patterns.len() {
        let total_arity = receiver_count + arity;
        let wrapper_params = params[..total_arity].to_vec();
        let mut lowerer = FnLowerer::new(
            signatures,
            interfaces,
            generic_interfaces,
            enum_values,
            enum_reverse_values,
            ret.clone(),
            None,
        );
        seed_global_scope(&mut lowerer, global_types, immutable_globals);
        if let Some((class, is_static)) = unbound_context {
            lowerer.class_context = Some(class.to_string());
            lowerer.class_static_context = is_static;
            lowerer.unbound_this_context = true;
        }
        for parameter in &wrapper_params {
            lowerer
                .scope
                .insert(parameter.name.clone(), parameter.ty.clone());
            lowerer
                .bindings
                .entry(parameter.name.clone())
                .or_default()
                .push(parameter.name.clone());
        }
        if receiver_count == 1 {
            lowerer
                .bindings
                .entry("this".into())
                .or_default()
                .push(params[0].name.clone());
        }
        let mut body = Vec::new();
        for (index, pattern) in patterns.iter().enumerate().skip(arity) {
            let parameter = &params[receiver_count + index];
            let value = match pattern {
                Pat::Assign(default) => {
                    let value = lowerer.lower_expr(&default.right)?;
                    lowerer.coerce_to_declared(&parameter.ty, value)?
                }
                Pat::Ident(binding) if binding.id.optional => match &parameter.ty {
                    HirType::Optional(payload) => {
                        HirExpr::OptionalNone(payload.as_ref().clone())
                    }
                    HirType::Nullish(payload) => {
                        HirExpr::NullishUndefined(payload.as_ref().clone())
                    }
                    other => {
                        return Err(format!(
                            "optional parameter `{}` of `{symbol}` needs an undefined-capable type, got {other:?}",
                            parameter.name
                        ))
                    }
                },
                _ => {
                    return Err(format!(
                        "omittable parameters of `{symbol}` must form a trailing sequence"
                    ))
                }
            };
            body.push(HirStmt::Let(
                parameter.name.clone(),
                parameter.ty.clone(),
                value,
            ));
            lowerer
                .scope
                .insert(parameter.name.clone(), parameter.ty.clone());
            lowerer
                .bindings
                .entry(parameter.name.clone())
                .or_default()
                .push(parameter.name.clone());
        }
        let call = HirExpr::Call(
            Box::new(HirExpr::Var(symbol.to_string())),
            params
                .iter()
                .map(|parameter| HirExpr::Var(parameter.name.clone()))
                .collect(),
        );
        let call = if is_async {
            HirExpr::Await(Box::new(call))
        } else {
            call
        };
        if *ret == HirType::Void {
            body.push(HirStmt::Expr(call));
            body.push(HirStmt::Return(None));
        } else {
            body.push(HirStmt::Return(Some(call)));
        }
        wrappers.push(HirFunction {
            name: default_arity_symbol(symbol, total_arity),
            params: wrapper_params,
            ret: ret.clone(),
            is_async,
            body,
        });
    }
    Ok(wrappers)
}

#[allow(clippy::too_many_arguments)]
fn lower_callable_omitted_parameter_wrappers(
    symbol: &str,
    params: &[HirParam],
    patterns: &[Pat],
    receiver_count: usize,
    unbound_context: Option<(&str, bool)>,
    ret: &HirType,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
) -> Result<Vec<HirFunction>, String> {
    let masks = omitted_parameter_masks(patterns, receiver_count)?;
    if masks.is_empty() {
        return Ok(Vec::new());
    }
    let is_async = signatures
        .get(symbol)
        .or_else(|| {
            symbol
                .strip_suffix("__thaw_unbound")
                .and_then(|original| signatures.get(original))
        })
        .ok_or_else(|| format!("missing class wrapper signature for `{symbol}`"))?
        .is_async;
    let mut wrappers = Vec::new();
    for mask in masks {
        let wrapper_params = params
            .iter()
            .enumerate()
            .filter_map(|(index, parameter)| {
                (mask & (1usize << index) == 0).then_some(parameter.clone())
            })
            .collect::<Vec<_>>();
        let mut lowerer = FnLowerer::new(
            signatures,
            interfaces,
            generic_interfaces,
            enum_values,
            enum_reverse_values,
            ret.clone(),
            None,
        );
        seed_global_scope(&mut lowerer, global_types, immutable_globals);
        if let Some((class, is_static)) = unbound_context {
            lowerer.class_context = Some(class.to_string());
            lowerer.class_static_context = is_static;
            lowerer.unbound_this_context = true;
        }
        if receiver_count == 1 {
            lowerer
                .scope
                .insert(params[0].name.clone(), params[0].ty.clone());
            lowerer
                .bindings
                .entry("this".into())
                .or_default()
                .push(params[0].name.clone());
        }
        let mut body = Vec::new();
        for (index, pattern) in patterns.iter().enumerate() {
            let parameter_index = receiver_count + index;
            let parameter = &params[parameter_index];
            if mask & (1usize << parameter_index) == 0 {
                lowerer
                    .scope
                    .insert(parameter.name.clone(), parameter.ty.clone());
                lowerer
                    .bindings
                    .entry(parameter.name.clone())
                    .or_default()
                    .push(parameter.name.clone());
                continue;
            }
            let value = match pattern {
                Pat::Assign(default) => {
                    let value = lowerer.lower_expr(&default.right)?;
                    lowerer.coerce_to_declared(&parameter.ty, value)?
                }
                Pat::Ident(binding) if binding.id.optional => match &parameter.ty {
                    HirType::Optional(payload) => {
                        HirExpr::OptionalNone(payload.as_ref().clone())
                    }
                    HirType::Nullish(payload) => {
                        HirExpr::NullishUndefined(payload.as_ref().clone())
                    }
                    other => {
                        return Err(format!(
                            "optional parameter `{}` of `{symbol}` needs an undefined-capable type, got {other:?}",
                            parameter.name
                        ))
                    }
                },
                _ => unreachable!("omission masks contain only omittable parameters"),
            };
            body.push(HirStmt::Let(
                parameter.name.clone(),
                parameter.ty.clone(),
                value,
            ));
            lowerer
                .scope
                .insert(parameter.name.clone(), parameter.ty.clone());
            lowerer
                .bindings
                .entry(parameter.name.clone())
                .or_default()
                .push(parameter.name.clone());
        }
        let call = HirExpr::Call(
            Box::new(HirExpr::Var(symbol.to_string())),
            params
                .iter()
                .map(|parameter| HirExpr::Var(parameter.name.clone()))
                .collect(),
        );
        let call = if is_async {
            HirExpr::Await(Box::new(call))
        } else {
            call
        };
        if *ret == HirType::Void {
            body.push(HirStmt::Expr(call));
            body.push(HirStmt::Return(None));
        } else {
            body.push(HirStmt::Return(Some(call)));
        }
        wrappers.push(HirFunction {
            name: omitted_parameter_symbol(symbol, mask),
            params: wrapper_params,
            ret: ret.clone(),
            is_async,
            body,
        });
    }
    Ok(wrappers)
}

#[allow(clippy::too_many_arguments)]
fn lower_class_constructor(
    declaration: &ClassDecl,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
) -> Result<Vec<HirFunction>, String> {
    let class_name = declaration.ident.sym.to_string();
    let constructor_symbol = class_constructor_symbol(&class_name);
    let initializer_symbol = class_initializer_symbol(&class_name);
    let instance_type = interfaces[&class_name].clone();
    let constructor = declaration
        .class
        .body
        .iter()
        .find_map(|member| match member {
            ClassMember::Constructor(constructor) => Some(constructor),
            _ => None,
        });
    let source_params = constructor
        .map(|constructor| constructor.params.as_slice())
        .unwrap_or_default();
    let has_implicit_constructor = constructor.is_none();
    let mut params = Vec::with_capacity(source_params.len());
    for (index, parameter) in source_params.iter().enumerate() {
        let pattern = class_constructor_param_pattern(parameter);
        let mut parameter = lower_param(
            &pattern,
            interfaces,
            generic_interfaces,
            false,
            &HashMap::new(),
        )?;
        parameter.ty = signatures[&constructor_symbol].params[index].clone();
        params.push(parameter);
    }
    if constructor.is_none() && declaration.class.super_class.is_some() {
        params = signatures[&constructor_symbol]
            .params
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__thaw_implicit_super_arg_{index}"),
                ty: ty.clone(),
            })
            .collect();
    }

    let this_name = "__thaw_this".to_string();
    let mut initializer_params = vec![HirParam {
        name: this_name.clone(),
        ty: instance_type.clone(),
    }];
    initializer_params.extend(params.iter().cloned());

    let mut lowerer = FnLowerer::new(
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        instance_type.clone(),
        None,
    );
    seed_global_scope(&mut lowerer, global_types, immutable_globals);
    if let Some(base) = &declaration.class.super_class {
        let Expr::Ident(base) = base.as_ref() else {
            unreachable!("class layout validation accepts identifier bases only")
        };
        lowerer.super_initializer = Some((
            class_initializer_symbol(base.sym.as_ref()),
            interfaces.get(base.sym.as_ref()).cloned().unwrap_or(HirType::Void),
            base.sym.to_string(),
        ));
    }
    lowerer
        .bindings
        .entry("this".into())
        .or_default()
        .push(this_name.clone());
    for parameter in &initializer_params {
        lowerer
            .scope
            .insert(parameter.name.clone(), parameter.ty.clone());
        lowerer
            .bindings
            .entry(parameter.name.clone())
            .or_default()
            .push(parameter.name.clone());
    }

    let mut own_initializers = Vec::new();
    for member in &declaration.class.body {
        let ClassMember::ClassProp(property) = member else {
            continue;
        };
        if property.is_static {
            continue;
        }
        let field = class_property_name(&property.key)?;
        let HirType::Object(fields) = &instance_type else {
            unreachable!("native class layouts are fixed objects")
        };
        let expected = fields
            .iter()
            .find_map(|(name, ty)| (name == &field).then(|| ty.clone()))
            .ok_or_else(|| format!("class `{class_name}` has no field `{field}`"))?;
        let value = match property.value.as_deref() {
            Some(initializer) => {
                let value = lowerer.lower_expr(initializer)?;
                Some(lowerer.coerce_to_declared(&expected, value)?)
            }
            None => match &expected {
                HirType::Optional(payload) => Some(HirExpr::OptionalNone(payload.as_ref().clone())),
                HirType::Nullish(payload) => {
                    Some(HirExpr::NullishUndefined(payload.as_ref().clone()))
                }
                _ => None,
            },
        };
        if let Some(value) = value {
            own_initializers.push(HirStmt::Expr(HirExpr::PropAssign(
                Box::new(HirExpr::Var(this_name.clone())),
                instance_type.clone(),
                field,
                Box::new(value),
            )));
        }
    }
    for (source, parameter) in source_params.iter().zip(&params) {
        let ParamOrTsParamProp::TsParamProp(property) = source else {
            continue;
        };
        let field = parameter_property_binding(property)?.id.sym.to_string();
        own_initializers.push(HirStmt::Expr(HirExpr::PropAssign(
            Box::new(HirExpr::Var(this_name.clone())),
            instance_type.clone(),
            field,
            Box::new(HirExpr::Var(parameter.name.clone())),
        )));
    }
    let implicit_own_initializers = own_initializers.clone();
    let mut initializer_body = Vec::new();
    if declaration.class.super_class.is_some() {
        if let Some(constructor) = constructor {
            let block = constructor.body.as_ref().ok_or_else(|| {
                format!("class `{class_name}` constructor needs an implementation body")
            })?;
            let mut called_super = false;
            for statement in &block.stmts {
                let is_super = matches!(statement, Stmt::Expr(expression) if matches!(expression.expr.as_ref(), Expr::Call(call) if matches!(call.callee, Callee::Super(_))));
                initializer_body.extend(lowerer.lower_stmt_seq(statement)?);
                if is_super {
                    if called_super {
                        return Err(format!(
                            "derived class `{class_name}` constructor calls super more than once"
                        ));
                    }
                    called_super = true;
                    initializer_body.append(&mut own_initializers);
                }
            }
            if !called_super {
                return Err(format!(
                    "derived class `{class_name}` constructor must call super(...)"
                ));
            }
        } else {
            let (base_initializer, _, _) = lowerer
                .super_initializer
                .clone()
                .expect("derived class has a base initializer");
            let signature = &signatures[&base_initializer];
            if signature.params.len() != params.len() + 1 {
                return Err(format!(
                    "derived class `{class_name}` cannot forward its implicit constructor arguments to the base"
                ));
            }
            let mut args = vec![HirExpr::Var(this_name.clone())];
            args.extend(
                params
                    .iter()
                    .map(|parameter| HirExpr::Var(parameter.name.clone())),
            );
            initializer_body.push(HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var(base_initializer)),
                args,
            )));
            initializer_body.append(&mut own_initializers);
        }
    } else {
        initializer_body.append(&mut own_initializers);
        if let Some(constructor) = constructor {
            let block = constructor.body.as_ref().ok_or_else(|| {
                format!("class `{class_name}` constructor needs an implementation body")
            })?;
            initializer_body.extend(lowerer.lower_stmts(&block.stmts)?);
        }
    }
    initializer_body.push(HirStmt::Return(Some(HirExpr::Var(this_name.clone()))));

    let mut initialize_args = vec![HirExpr::Var(this_name.clone())];
    initialize_args.extend(
        params
            .iter()
            .map(|parameter| HirExpr::Var(parameter.name.clone())),
    );
    let constructor = HirFunction {
        name: constructor_symbol.clone(),
        params: params.clone(),
        ret: instance_type.clone(),
        is_async: false,
        body: vec![
            HirStmt::Let(
                this_name.clone(),
                instance_type.clone(),
                HirExpr::ObjectAlloc(instance_type.clone()),
            ),
            HirStmt::Return(Some(HirExpr::Call(
                Box::new(HirExpr::Var(initializer_symbol.clone())),
                initialize_args,
            ))),
        ],
    };
    let initializer = HirFunction {
        name: initializer_symbol.clone(),
        params: initializer_params,
        ret: instance_type.clone(),
        is_async: false,
        body: initializer_body,
    };
    let patterns = source_params
        .iter()
        .map(class_constructor_param_pattern)
        .collect::<Vec<_>>();
    let mut functions = vec![constructor, initializer];
    functions.extend(lower_callable_default_wrappers(
        &constructor_symbol,
        &params,
        &patterns,
        0,
        None,
        &instance_type,
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        global_types,
        immutable_globals,
    )?);
    functions.extend(lower_callable_omitted_parameter_wrappers(
        &constructor_symbol,
        &params,
        &patterns,
        0,
        None,
        &instance_type,
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        global_types,
        immutable_globals,
    )?);
    let mut initializer_wrapper_params = vec![HirParam {
        name: this_name,
        ty: instance_type.clone(),
    }];
    initializer_wrapper_params.extend(params.iter().cloned());
    functions.extend(lower_callable_default_wrappers(
        &initializer_symbol,
        &initializer_wrapper_params,
        &patterns,
        1,
        None,
        &instance_type,
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        global_types,
        immutable_globals,
    )?);
    functions.extend(lower_callable_omitted_parameter_wrappers(
        &initializer_symbol,
        &initializer_wrapper_params,
        &patterns,
        1,
        None,
        &instance_type,
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        global_types,
        immutable_globals,
    )?);
    if has_implicit_constructor {
        if let Some(Expr::Ident(base)) = declaration.class.super_class.as_deref() {
            for arity in 0..params.len() {
                let derived_constructor_wrapper = default_arity_symbol(&constructor_symbol, arity);
                let derived_initializer_wrapper =
                    default_arity_symbol(&initializer_symbol, arity + 1);
                if !signatures.contains_key(&derived_constructor_wrapper)
                    || !signatures.contains_key(&derived_initializer_wrapper)
                {
                    continue;
                }
                let wrapper_params = params[..arity].to_vec();
                let wrapper_this = "__thaw_this".to_string();
                let mut initialize_args = vec![HirExpr::Var(wrapper_this.clone())];
                initialize_args.extend(
                    wrapper_params
                        .iter()
                        .map(|parameter| HirExpr::Var(parameter.name.clone())),
                );
                functions.push(HirFunction {
                    name: derived_constructor_wrapper,
                    params: wrapper_params.clone(),
                    ret: instance_type.clone(),
                    is_async: false,
                    body: vec![
                        HirStmt::Let(
                            wrapper_this.clone(),
                            instance_type.clone(),
                            HirExpr::ObjectAlloc(instance_type.clone()),
                        ),
                        HirStmt::Return(Some(HirExpr::Call(
                            Box::new(HirExpr::Var(derived_initializer_wrapper.clone())),
                            initialize_args,
                        ))),
                    ],
                });

                let mut initializer_params = vec![HirParam {
                    name: wrapper_this.clone(),
                    ty: instance_type.clone(),
                }];
                initializer_params.extend(wrapper_params);
                let base_initializer_wrapper =
                    default_arity_symbol(&class_initializer_symbol(base.sym.as_ref()), arity + 1);
                let mut base_args = vec![HirExpr::Var(wrapper_this.clone())];
                base_args.extend(
                    initializer_params[1..]
                        .iter()
                        .map(|parameter| HirExpr::Var(parameter.name.clone())),
                );
                let mut body = vec![HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var(base_initializer_wrapper)),
                    base_args,
                ))];
                body.extend(implicit_own_initializers.iter().cloned());
                body.push(HirStmt::Return(Some(HirExpr::Var(wrapper_this))));
                functions.push(HirFunction {
                    name: derived_initializer_wrapper,
                    params: initializer_params,
                    ret: instance_type.clone(),
                    is_async: false,
                    body,
                });
            }
            if params.len() <= 16 {
                for mask in 1usize..(1usize << params.len()) {
                    let derived_constructor_wrapper =
                        omitted_parameter_symbol(&constructor_symbol, mask);
                    let initializer_mask = mask << 1;
                    let derived_initializer_wrapper =
                        omitted_parameter_symbol(&initializer_symbol, initializer_mask);
                    if !signatures.contains_key(&derived_constructor_wrapper)
                        || !signatures.contains_key(&derived_initializer_wrapper)
                    {
                        continue;
                    }
                    let wrapper_params = params
                        .iter()
                        .enumerate()
                        .filter_map(|(index, parameter)| {
                            (mask & (1usize << index) == 0).then_some(parameter.clone())
                        })
                        .collect::<Vec<_>>();
                    let wrapper_this = "__thaw_this".to_string();
                    let mut initialize_args = vec![HirExpr::Var(wrapper_this.clone())];
                    initialize_args.extend(
                        wrapper_params
                            .iter()
                            .map(|parameter| HirExpr::Var(parameter.name.clone())),
                    );
                    functions.push(HirFunction {
                        name: derived_constructor_wrapper,
                        params: wrapper_params.clone(),
                        ret: instance_type.clone(),
                        is_async: false,
                        body: vec![
                            HirStmt::Let(
                                wrapper_this.clone(),
                                instance_type.clone(),
                                HirExpr::ObjectAlloc(instance_type.clone()),
                            ),
                            HirStmt::Return(Some(HirExpr::Call(
                                Box::new(HirExpr::Var(derived_initializer_wrapper.clone())),
                                initialize_args,
                            ))),
                        ],
                    });

                    let mut initializer_params = vec![HirParam {
                        name: wrapper_this.clone(),
                        ty: instance_type.clone(),
                    }];
                    initializer_params.extend(wrapper_params);
                    let base_initializer_wrapper = omitted_parameter_symbol(
                        &class_initializer_symbol(base.sym.as_ref()),
                        initializer_mask,
                    );
                    let mut base_args = vec![HirExpr::Var(wrapper_this.clone())];
                    base_args.extend(
                        initializer_params[1..]
                            .iter()
                            .map(|parameter| HirExpr::Var(parameter.name.clone())),
                    );
                    let mut body = vec![HirStmt::Expr(HirExpr::Call(
                        Box::new(HirExpr::Var(base_initializer_wrapper)),
                        base_args,
                    ))];
                    body.extend(implicit_own_initializers.iter().cloned());
                    body.push(HirStmt::Return(Some(HirExpr::Var(wrapper_this))));
                    functions.push(HirFunction {
                        name: derived_initializer_wrapper,
                        params: initializer_params,
                        ret: instance_type.clone(),
                        is_async: false,
                        body,
                    });
                }
            }
        }
    }
    Ok(functions)
}

#[allow(clippy::too_many_arguments)]
fn lower_class_methods(
    declaration: &ClassDecl,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
) -> Result<Vec<HirFunction>, String> {
    let class_name = declaration.ident.sym.to_string();
    let instance_type = interfaces[&class_name].clone();
    let mut functions = Vec::new();
    for member in &declaration.class.body {
        let ClassMember::Method(method) = member else {
            continue;
        };
        if !matches!(
            method.kind,
            MethodKind::Method | MethodKind::Getter | MethodKind::Setter
        ) {
            continue;
        }
        let method_name = class_property_name(&method.key)?;
        let symbol = match method.kind {
            MethodKind::Getter => class_getter_symbol(&class_name, &method_name, method.is_static),
            MethodKind::Setter => class_setter_symbol(&class_name, &method_name, method.is_static),
            MethodKind::Method if method.is_static => {
                class_static_method_symbol(&class_name, &method_name)
            }
            MethodKind::Method => class_method_symbol(&class_name, &method_name),
        };
        let signature = &signatures[&symbol];
        if method.is_abstract || (declaration.class.is_abstract && !method.is_static) {
            continue;
        }
        let mut params = if method.is_static {
            Vec::new()
        } else {
            vec![HirParam {
                name: "__thaw_this".into(),
                ty: instance_type.clone(),
            }]
        };
        let receiver_offset = usize::from(!method.is_static);
        for (index, parameter) in method.function.params.iter().enumerate() {
            let mut parameter = lower_param(
                &parameter.pat,
                interfaces,
                generic_interfaces,
                false,
                &HashMap::new(),
            )?;
            parameter.ty = signature.params[index + receiver_offset].clone();
            params.push(parameter);
        }
        let body =
            method.function.body.as_ref().ok_or_else(|| {
                format!("class `{class_name}` method `{method_name}` needs a body")
            })?;
        let mut lowerer = FnLowerer::new(
            signatures,
            interfaces,
            generic_interfaces,
            enum_values,
            enum_reverse_values,
            signature.ret.clone(),
            None,
        );
        seed_global_scope(&mut lowerer, global_types, immutable_globals);
        lowerer.class_static_context = method.is_static;
        lowerer.class_context = Some(class_name.clone());
        if let Some(base) = &declaration.class.super_class {
            let Expr::Ident(base) = base.as_ref() else {
                unreachable!("class layout validation accepts identifier bases only")
            };
            lowerer.super_initializer = Some((
                class_initializer_symbol(base.sym.as_ref()),
                interfaces.get(base.sym.as_ref()).cloned().unwrap_or(HirType::Void),
                base.sym.to_string(),
            ));
        }
        for parameter in &params {
            lowerer
                .scope
                .insert(parameter.name.clone(), parameter.ty.clone());
            lowerer
                .bindings
                .entry(parameter.name.clone())
                .or_default()
                .push(parameter.name.clone());
        }
        if !method.is_static {
            lowerer
                .bindings
                .entry("this".into())
                .or_default()
                .push("__thaw_this".into());
        }
        let mut lowered_body = lowerer.lower_stmts(&body.stmts)?;
        if method.kind == MethodKind::Setter {
            lowered_body.push(HirStmt::Return(Some(HirExpr::Var(
                params.last().expect("setter value parameter").name.clone(),
            ))));
        }
        functions.push(HirFunction {
            name: symbol.clone(),
            params: params.clone(),
            ret: signature.ret.clone(),
            is_async: signature.is_async,
            body: lowered_body,
        });
        if method.kind == MethodKind::Method && signature.uses_this {
            let unbound_symbol = unbound_class_method_symbol(&symbol);
            let unbound_signature = signatures.get(&unbound_symbol).unwrap_or(signature);
            let unbound_params = params[receiver_offset..].to_vec();
            let mut unbound = FnLowerer::new(
                signatures,
                interfaces,
                generic_interfaces,
                enum_values,
                enum_reverse_values,
                unbound_signature.ret.clone(),
                None,
            );
            seed_global_scope(&mut unbound, global_types, immutable_globals);
            unbound.class_static_context = method.is_static;
            unbound.class_context = Some(class_name.clone());
            unbound.unbound_this_context = true;
            if let Some(base) = &declaration.class.super_class {
                let Expr::Ident(base) = base.as_ref() else {
                    unreachable!("class layout validation accepts identifier bases only")
                };
                unbound.super_initializer = Some((
                    class_initializer_symbol(base.sym.as_ref()),
                    interfaces.get(base.sym.as_ref()).cloned().unwrap_or(HirType::Void),
                    base.sym.to_string(),
                ));
            }
            for parameter in &unbound_params {
                unbound
                    .scope
                    .insert(parameter.name.clone(), parameter.ty.clone());
                unbound
                    .bindings
                    .entry(parameter.name.clone())
                    .or_default()
                    .push(parameter.name.clone());
            }
            functions.push(HirFunction {
                name: unbound_symbol,
                params: unbound_params,
                ret: unbound_signature.ret.clone(),
                is_async: unbound_signature.is_async,
                body: unbound.lower_stmts(&body.stmts)?,
            });
        }
        let patterns = method
            .function
            .params
            .iter()
            .map(|parameter| parameter.pat.clone())
            .collect::<Vec<_>>();
        functions.extend(lower_callable_default_wrappers(
            &symbol,
            &params,
            &patterns,
            receiver_offset,
            None,
            &signature.ret,
            signatures,
            interfaces,
            generic_interfaces,
            enum_values,
            enum_reverse_values,
            global_types,
            immutable_globals,
        )?);
        functions.extend(lower_callable_omitted_parameter_wrappers(
            &symbol,
            &params,
            &patterns,
            receiver_offset,
            None,
            &signature.ret,
            signatures,
            interfaces,
            generic_interfaces,
            enum_values,
            enum_reverse_values,
            global_types,
            immutable_globals,
        )?);
        if method.kind == MethodKind::Method && signature.uses_this {
            let unbound_symbol = unbound_class_method_symbol(&symbol);
            let unbound_params = params[receiver_offset..].to_vec();
            let unbound_context = Some((class_name.as_str(), method.is_static));
            functions.extend(lower_callable_default_wrappers(
                &unbound_symbol,
                &unbound_params,
                &patterns,
                0,
                unbound_context,
                &signature.ret,
                signatures,
                interfaces,
                generic_interfaces,
                enum_values,
                enum_reverse_values,
                global_types,
                immutable_globals,
            )?);
            functions.extend(lower_callable_omitted_parameter_wrappers(
                &unbound_symbol,
                &unbound_params,
                &patterns,
                0,
                unbound_context,
                &signature.ret,
                signatures,
                interfaces,
                generic_interfaces,
                enum_values,
                enum_reverse_values,
                global_types,
                immutable_globals,
            )?);
        }
    }
    Ok(functions)
}

#[allow(clippy::too_many_arguments)]
fn lower_fn_decl(
    fn_decl: &FnDecl,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
    call_constraints: Option<&RefCell<Vec<CallConstraint>>>,
) -> Result<HirFunction, String> {
    let name = fn_decl.ident.sym.to_string();
    let func = &fn_decl.function;
    let type_substitution = function_type_substitution(func);

    let mut params = func
        .params
        .iter()
        .map(|param| {
            lower_param(
                &param.pat,
                interfaces,
                generic_interfaces,
                true,
                &type_substitution,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (param, inferred) in params.iter_mut().zip(&signatures[&name].params) {
        param.ty = inferred.clone();
    }

    let declared_ret = signatures[&name].ret.clone();

    let body_block = func
        .body
        .as_ref()
        .ok_or_else(|| format!("function `{name}` has no body (ambient/overload decl?)"))?;

    let mut lowerer = FnLowerer::new(
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        declared_ret.clone(),
        call_constraints,
    );
    seed_global_scope(&mut lowerer, global_types, immutable_globals);
    for (source, param) in func.params.iter().zip(&params) {
        lowerer.immutable_bindings.remove(&param.name);
        lowerer.scope.insert(param.name.clone(), param.ty.clone());
        lowerer
            .bindings
            .entry(param.name.clone())
            .or_default()
            .push(param.name.clone());
        let annotation = match &source.pat {
            Pat::Ident(binding) => binding.type_ann.as_ref(),
            Pat::Object(pattern) => pattern.type_ann.as_ref(),
            Pat::Array(pattern) => pattern.type_ann.as_ref(),
            _ => None,
        };
        if let Some(annotation) = annotation {
            let discriminants =
                object_union_discriminants(&annotation.type_ann, generic_interfaces);
            if !discriminants.is_empty() {
                lowerer
                    .union_discriminants
                    .insert(param.name.clone(), discriminants);
            }
            let element_discriminants =
                array_element_union_discriminants(&annotation.type_ann, generic_interfaces);
            if !element_discriminants.is_empty() {
                lowerer
                    .array_element_discriminants
                    .insert(param.name.clone(), element_discriminants);
            }
            let nested_discriminants =
                nested_array_union_discriminants(&annotation.type_ann, generic_interfaces);
            if !nested_discriminants.is_empty() {
                lowerer
                    .nested_array_discriminants
                    .insert(param.name.clone(), nested_discriminants);
            }
            let property_discriminants =
                object_array_property_discriminants(&annotation.type_ann, generic_interfaces);
            if !property_discriminants.is_empty() {
                lowerer
                    .object_array_property_discriminants
                    .insert(param.name.clone(), property_discriminants);
            }
            let function_property_discriminants =
                object_function_property_discriminants(&annotation.type_ann, generic_interfaces);
            if !function_property_discriminants.is_empty() {
                lowerer
                    .object_function_property_discriminants
                    .insert(param.name.clone(), function_property_discriminants);
            }
            let return_discriminants =
                function_return_discriminants(&annotation.type_ann, generic_interfaces);
            if !return_discriminants.is_empty() {
                lowerer
                    .function_value_discriminants
                    .insert(param.name.clone(), return_discriminants);
            }
            let return_array_discriminants =
                function_return_array_discriminants(&annotation.type_ann, generic_interfaces);
            if !return_array_discriminants.is_empty() {
                lowerer
                    .function_value_array_discriminants
                    .insert(param.name.clone(), return_array_discriminants);
            }
            let return_property_discriminants = function_return_object_array_property_discriminants(
                &annotation.type_ann,
                generic_interfaces,
            );
            if !return_property_discriminants.is_empty() {
                lowerer
                    .function_value_object_array_property_discriminants
                    .insert(param.name.clone(), return_property_discriminants);
            }
            let return_function_property_discriminants =
                function_return_object_function_property_discriminants(
                    &annotation.type_ann,
                    generic_interfaces,
                );
            if !return_function_property_discriminants.is_empty() {
                lowerer
                    .function_value_object_function_property_discriminants
                    .insert(param.name.clone(), return_function_property_discriminants);
            }
        }
    }
    let mut body = Vec::new();
    for (source, param) in func.params.iter().zip(&params) {
        if !matches!(source.pat, Pat::Ident(_) | Pat::Rest(_)) {
            lowerer.lower_binding_pattern(
                &source.pat,
                HirExpr::Var(param.name.clone()),
                &param.ty,
                &mut body,
            )?;
        }
    }
    body.extend(lowerer.lower_stmts(&body_block.stmts)?);
    let ret = if declared_ret == HirType::Dynamic {
        lowerer.infer_return_type(&body)?
    } else {
        declared_ret
    };

    Ok(HirFunction {
        name,
        params,
        ret,
        is_async: func.is_async,
        body,
    })
}

fn seed_global_scope(
    lowerer: &mut FnLowerer<'_>,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
) {
    for (name, ty) in global_types {
        lowerer.scope.insert(name.clone(), ty.clone());
        lowerer
            .bindings
            .entry(name.clone())
            .or_default()
            .push(name.clone());
    }
    lowerer
        .immutable_bindings
        .extend(immutable_globals.iter().cloned());
}

/// Computes a function's *unwrapped* return type: `async function`s must be
/// declared as returning `Promise<T>`, and this returns `T` -- V1
/// async/await erases `Promise` entirely at lowering time (see
/// docs/design/async-await.md). Non-async functions are unaffected.
fn lower_fn_return_type(
    is_async: bool,
    return_type: &Option<Box<swc_ecma_ast::TsTypeAnn>>,
    fn_name: &str,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    type_substitution: &HashMap<Symbol, HirType>,
) -> Result<HirType, String> {
    let declared = match return_type {
        Some(ann) => resolve_ts_type_with_substitution(
            &ann.type_ann,
            type_substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )?,
        None => HirType::Void,
    };
    if !is_async {
        return Ok(declared);
    }
    match declared {
        HirType::Promise(inner) => Ok(*inner),
        other => Err(format!(
            "async function `{fn_name}` must be declared as returning `Promise<T>`, found {other:?}"
        )),
    }
}

fn lower_param(
    pat: &Pat,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    allow_inference: bool,
    type_substitution: &HashMap<Symbol, HirType>,
) -> Result<HirParam, String> {
    if let Pat::Assign(assignment) = pat {
        return lower_param(
            &assignment.left,
            interfaces,
            generic_interfaces,
            allow_inference,
            type_substitution,
        );
    }
    if let Pat::Rest(rest) = pat {
        let mut argument = rest.arg.as_ref().clone();
        match &mut argument {
            Pat::Ident(binding) => binding.type_ann = rest.type_ann.clone(),
            Pat::Array(pattern) => pattern.type_ann = rest.type_ann.clone(),
            Pat::Object(pattern) => pattern.type_ann = rest.type_ann.clone(),
            _ => {}
        }
        return lower_param(
            &argument,
            interfaces,
            generic_interfaces,
            allow_inference,
            type_substitution,
        );
    }
    let (name, type_ann, optional) = match pat {
        Pat::Ident(binding) => (
            binding.id.sym.to_string(),
            binding.type_ann.as_ref(),
            binding.id.optional,
        ),
        Pat::Object(pattern) => (
            format!("__thaw_param_{}", pattern.span.lo.0),
            pattern.type_ann.as_ref(),
            false,
        ),
        Pat::Array(pattern) => (
            format!("__thaw_param_{}", pattern.span.lo.0),
            pattern.type_ann.as_ref(),
            false,
        ),
        _ => return Err("unsupported function parameter pattern".into()),
    };
    let mut ty = match type_ann {
        Some(ann) => resolve_ts_type_with_substitution(
            &ann.type_ann,
            type_substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )?,
        None if allow_inference => HirType::Dynamic,
        None => {
            return Err(format!(
            "parameter `{name}` needs an explicit type annotation (no type inference for params)"
        ))
        }
    };
    if optional {
        ty = optional_parameter_type(ty);
    }
    Ok(HirParam { name, ty })
}
