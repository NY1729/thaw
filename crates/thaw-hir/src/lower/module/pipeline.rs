pub fn lower_module(module: &Module) -> Result<HirProgram, String> {
    let normalized = normalize_top_level_destructuring(module)?;
    let normalized = normalize_top_level_class_expressions(&normalized)?;
    let normalized = normalize_static_computed_class_members(&normalized);
    let normalized = normalize_private_class_members(&normalized);
    lower_normalized_module(&normalized)
}

fn lower_normalized_module(module: &Module) -> Result<HirProgram, String> {
    let (mut interfaces, generic_interfaces) = resolve_interfaces(module)?;
    let specialized = specialize_generic_classes(module, &interfaces, &generic_interfaces)
        .map_err(|error| {
            if error.contains("generics are not supported yet") {
                format!(
                    "recursive or unresolved generic class type cannot use Thaw's fixed-size native layout: {error}"
                )
            } else {
                error
            }
        })?;
    if let Some(specialized) = specialized {
        return lower_normalized_module(&specialized).map_err(|error| {
            if error.contains("generics are not supported yet") {
                format!(
                    "recursive or unresolved generic class type cannot use Thaw's fixed-size native layout: {error}"
                )
            } else {
                error
            }
        });
    }
    let (enum_values, enum_reverse_values, enum_types) = collect_enums(module)?;
    for (name, ty) in enum_types {
        if interfaces.insert(name.clone(), ty).is_some() {
            return Err(format!(
                "enum `{name}` conflicts with an interface declaration"
            ));
        }
    }
    let class_decls = collect_native_classes(module, &mut interfaces, &generic_interfaces)?;
    if let Some(specialized) =
        specialize_generic_class_methods(module, &interfaces, &generic_interfaces)?
    {
        return lower_normalized_module(&specialized);
    }

    let mut signatures: HashMap<Symbol, FnSignature> = HashMap::new();
    let mut fn_decls = Vec::new();
    let mut global_decls = Vec::new();
    let mut generic_instantiations: HashMap<Symbol, Vec<Vec<HirType>>> = HashMap::new();

    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(fn_decl))) => {
                let name = fn_decl.ident.sym.to_string();
                let func = &fn_decl.function;
                let is_extern = func.body.is_none();
                if is_extern && func.is_async {
                    return Err(format!("ambient function `{name}` cannot be async"));
                }
                let type_substitution = function_type_substitution(func);
                let generic_type_params = validate_generic_function(fn_decl)?;
                let generic_type_constraints = func
                    .type_params
                    .as_ref()
                    .map(|parameters| {
                        parameters
                            .params
                            .iter()
                            .map(|parameter| parameter.constraint.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let generic_type_defaults = func
                    .type_params
                    .as_ref()
                    .map(|parameters| {
                        parameters
                            .params
                            .iter()
                            .map(|parameter| parameter.default.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let generic_substitutions = if generic_type_params.is_empty() {
                    None
                } else {
                    Some(
                        generic_type_params
                            .iter()
                            .map(|name| (name.clone(), GenericTypePattern::Variable(name.clone())))
                            .collect::<HashMap<_, _>>(),
                    )
                };
                let generic_param_patterns = match &generic_substitutions {
                    None => Vec::new(),
                    Some(substitutions) => func
                        .params
                        .iter()
                        .map(|param| match &param.pat {
                            Pat::Ident(binding) => binding
                                .type_ann
                                .as_ref()
                                .ok_or_else(|| format!("generic function `{name}` needs parameter type annotations"))
                                .and_then(|ann| generic_type_pattern(
                                    &ann.type_ann,
                                    substitutions,
                                    &interfaces,
                                    &generic_interfaces,
                                    &mut Vec::new(),
                                )),
                            _ => Err(format!("generic function `{name}` requires identifier parameters")),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                };
                // Best-effort only (never aborts the signature over it,
                // unlike the parameter patterns above): a fallback source
                // for inferring a type parameter that appears in no
                // parameter at all, e.g. `nanoid<Type extends string>
                // (size?: number): Type`. See
                // `infer_generic_type_tuple`/`lower_expr_with_expected_type`.
                let generic_return_pattern = generic_substitutions.as_ref().and_then(|substitutions| {
                    func.return_type.as_ref().and_then(|ann| {
                        generic_type_pattern(
                            &ann.type_ann,
                            substitutions,
                            &interfaces,
                            &generic_interfaces,
                            &mut Vec::new(),
                        )
                        .ok()
                    })
                });
                let variadic = if is_extern {
                    func.params.last().and_then(|param| match &param.pat {
                        Pat::Rest(rest) => Some(rest),
                        _ => None,
                    }).map(|rest| {
                        let annotation = rest.type_ann.as_ref().ok_or_else(|| {
                            format!("ambient variadic function `{name}` needs a rest parameter type annotation")
                        })?;
                        let ty = resolve_ts_type_with_substitution(
                            &annotation.type_ann,
                            &type_substitution,
                            &interfaces,
                            &generic_interfaces,
                            &mut Vec::new(),
                        )?;
                        match ty {
                            HirType::Array(element)
                                if supports_ffi_variadic_element(&element) => Ok(*element),
                            other => Err(format!(
                                "ambient variadic function `{name}` has unsupported rest element layout {other:?}"
                            )),
                        }
                    }).transpose()?
                } else {
                    None
                };
                let fixed_param_count = func.params.len() - usize::from(variadic.is_some());
                let params = func
                    .params
                    .iter()
                    .take(fixed_param_count)
                    .map(|p| {
                        lower_param(
                            &p.pat,
                            &interfaces,
                            &generic_interfaces,
                            !is_extern,
                            &type_substitution,
                        )
                        .map(|p| p.ty)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let ret = if func.return_type.is_none() && !is_extern {
                    HirType::Dynamic
                } else {
                    lower_fn_return_type(
                        func.is_async,
                        &func.return_type,
                        &name,
                        &interfaces,
                        &generic_interfaces,
                        &type_substitution,
                    )?
                };
                let generates_call_wrappers = !is_extern && generic_type_params.is_empty();
                // A plain (non-ambient, non-generic) function's trailing
                // `...rest: T[]` parameter reuses the exact same
                // `native_rest` mechanism a native class constructor's
                // rest parameter already does -- `params` here still holds
                // it as a regular `Array(T)`-typed last entry (unlike the
                // ambient/FFI `variadic` mechanism above, which excludes it
                // from `params` for a real C variadic ABI), so every call
                // site that already packs trailing arguments into that
                // slot for a constructor needs no changes to also do it
                // here. Ambient and generic functions are excluded: the
                // former already has its own FFI-specific `variadic`
                // handling, and the latter would need `native_rest`
                // reconstructed per monomorphization in `generic_calls.rs`,
                // which still hardcodes `None`.
                let native_rest = if is_extern || !generic_type_params.is_empty() {
                    None
                } else {
                    let patterns = func
                        .params
                        .iter()
                        .map(|parameter| parameter.pat.clone())
                        .collect::<Vec<_>>();
                    native_rest_element(&patterns, &params)?
                };
                signatures.insert(
                    name.clone(),
                    FnSignature {
                        params,
                        variadic,
                        native_rest,
                        abstract_class_constructor: false,
                        ret,
                        is_async: func.is_async,
                        uses_this: false,
                        is_extern,
                        source_range: (func.span.lo.0, func.span.hi.0),
                        generic_type_params,
                        generic_type_constraints,
                        generic_type_defaults,
                        generic_param_patterns,
                        generic_param_optional: func
                            .params
                            .iter()
                            .map(|parameter| {
                                matches!(&parameter.pat, Pat::Ident(binding) if binding.id.optional)
                            })
                            .collect(),
                        generic_return_type: func.return_type.as_ref().map(|ann| ann.type_ann.clone()),
                        generic_return_pattern,
                    },
                );
                if generates_call_wrappers {
                    let patterns = func
                        .params
                        .iter()
                        .map(|parameter| parameter.pat.clone())
                        .collect::<Vec<_>>();
                    if let Some(default_start) = trailing_omittable_start(&patterns) {
                        for arity in default_start..patterns.len() {
                            let mut wrapper = signatures[&name].clone();
                            wrapper.params.truncate(arity);
                            signatures.insert(default_arity_symbol(&name, arity), wrapper);
                        }
                    }
                    insert_omitted_parameter_signatures(&mut signatures, &name, &patterns, 0)?;
                }
                if !is_extern {
                    fn_decls.push(fn_decl);
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(class_decl))) => {
                let name = class_decl.ident.sym.to_string();
                let instance_type = interfaces[&name].clone();
                let constructors = class_decl
                    .class
                    .body
                    .iter()
                    .filter_map(|member| match member {
                        ClassMember::Constructor(constructor) => Some(constructor),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if constructors.len() > 1 {
                    return Err(format!(
                        "class `{name}` has multiple constructor implementations"
                    ));
                }
                let params = constructors
                    .first()
                    .map(|constructor| {
                        constructor
                            .params
                            .iter()
                            .map(|parameter| {
                                lower_param(
                                    &class_constructor_param_pattern(parameter),
                                    &interfaces,
                                    &generic_interfaces,
                                    false,
                                    &HashMap::new(),
                                )
                                .map(|parameter| parameter.ty)
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                signatures.insert(
                    class_constructor_symbol(&name),
                    FnSignature {
                        params,
                        variadic: None,
                        native_rest: None,
                        abstract_class_constructor: class_decl.class.is_abstract,
                        ret: instance_type.clone(),
                        is_async: false,
                        uses_this: false,
                        is_extern: false,
                        source_range: (class_decl.class.span.lo.0, class_decl.class.span.hi.0),
                        generic_type_params: Vec::new(),
                        generic_type_constraints: Vec::new(),
                        generic_type_defaults: Vec::new(),
                        generic_param_patterns: Vec::new(),
                        generic_param_optional: Vec::new(),
                        generic_return_type: None,
                        generic_return_pattern: None,
                    },
                );
                let mut initializer_params = vec![instance_type.clone()];
                initializer_params.extend(
                    signatures[&class_constructor_symbol(&name)]
                        .params
                        .iter()
                        .cloned(),
                );
                signatures.insert(
                    class_initializer_symbol(&name),
                    FnSignature {
                        params: initializer_params,
                        variadic: None,
                        native_rest: None,
                        abstract_class_constructor: false,
                        ret: instance_type.clone(),
                        is_async: false,
                        uses_this: false,
                        is_extern: false,
                        source_range: (class_decl.class.span.lo.0, class_decl.class.span.hi.0),
                        generic_type_params: Vec::new(),
                        generic_type_constraints: Vec::new(),
                        generic_type_defaults: Vec::new(),
                        generic_param_patterns: Vec::new(),
                        generic_param_optional: Vec::new(),
                        generic_return_type: None,
                        generic_return_pattern: None,
                    },
                );
                if let Some(constructor) = constructors.first() {
                    let patterns = constructor
                        .params
                        .iter()
                        .map(class_constructor_param_pattern)
                        .collect::<Vec<_>>();
                    let constructor_symbol = class_constructor_symbol(&name);
                    let initializer_symbol = class_initializer_symbol(&name);
                    let native_rest =
                        native_rest_element(&patterns, &signatures[&constructor_symbol].params)?;
                    signatures
                        .get_mut(&constructor_symbol)
                        .expect("class constructor signature")
                        .native_rest = native_rest.clone();
                    signatures
                        .get_mut(&initializer_symbol)
                        .expect("class initializer signature")
                        .native_rest = native_rest;
                    if let Some(default_start) = trailing_omittable_start(&patterns) {
                        for arity in default_start..patterns.len() {
                            let mut constructor_signature = signatures[&constructor_symbol].clone();
                            constructor_signature.params.truncate(arity);
                            signatures.insert(
                                default_arity_symbol(&constructor_symbol, arity),
                                constructor_signature,
                            );
                            let mut initializer_signature = signatures[&initializer_symbol].clone();
                            initializer_signature.params.truncate(arity + 1);
                            signatures.insert(
                                default_arity_symbol(&initializer_symbol, arity + 1),
                                initializer_signature,
                            );
                        }
                    }
                    insert_omitted_parameter_signatures(
                        &mut signatures,
                        &class_constructor_symbol(&name),
                        &patterns,
                        0,
                    )?;
                    insert_omitted_parameter_signatures(
                        &mut signatures,
                        &class_initializer_symbol(&name),
                        &patterns,
                        1,
                    )?;
                }
                for member in &class_decl.class.body {
                    let ClassMember::Method(method) = member else {
                        continue;
                    };
                    if !matches!(
                        method.kind,
                        MethodKind::Method | MethodKind::Getter | MethodKind::Setter
                    ) {
                        continue;
                    }
                    if method.function.type_params.is_some() || method.function.is_generator {
                        return Err(format!(
                            "class `{name}` method `{}` cannot be generic or a generator yet",
                            class_property_name(&method.key)?
                        ));
                    }
                    let method_name = class_property_name(&method.key)?;
                    if method.kind == MethodKind::Getter && !method.function.params.is_empty() {
                        return Err(format!(
                            "class `{name}` getter `{method_name}` cannot accept parameters"
                        ));
                    }
                    if method.kind == MethodKind::Setter && method.function.params.len() != 1 {
                        return Err(format!(
                            "class `{name}` setter `{method_name}` requires exactly one parameter"
                        ));
                    }
                    let mut params = if method.is_static {
                        Vec::new()
                    } else {
                        vec![instance_type.clone()]
                    };
                    params.extend(
                        method
                            .function
                            .params
                            .iter()
                            .map(|parameter| {
                                lower_param(
                                    &parameter.pat,
                                    &interfaces,
                                    &generic_interfaces,
                                    false,
                                    &HashMap::new(),
                                )
                                .map(|parameter| parameter.ty)
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                    let ret = if method.kind == MethodKind::Setter {
                        params
                            .last()
                            .expect("a setter has one declared value parameter")
                            .clone()
                    } else {
                        lower_fn_return_type(
                            method.function.is_async,
                            &method.function.return_type,
                            &format!("{name}.{method_name}"),
                            &interfaces,
                            &generic_interfaces,
                            &HashMap::new(),
                        )?
                    };
                    let symbol = match method.kind {
                        MethodKind::Getter => {
                            class_getter_symbol(&name, &method_name, method.is_static)
                        }
                        MethodKind::Setter => {
                            class_setter_symbol(&name, &method_name, method.is_static)
                        }
                        MethodKind::Method if method.is_static => {
                            class_static_method_symbol(&name, &method_name)
                        }
                        MethodKind::Method => class_method_symbol(&name, &method_name),
                    };
                    if signatures.contains_key(&symbol) {
                        return Err(format!(
                            "class `{name}` has duplicate method `{method_name}`"
                        ));
                    }
                    signatures.insert(
                        symbol,
                        FnSignature {
                            params,
                            variadic: None,
                            native_rest: None,
                            abstract_class_constructor: false,
                            ret,
                            is_async: method.function.is_async,
                            uses_this: function_uses_this(&method.function),
                            is_extern: false,
                            source_range: (method.span.lo.0, method.span.hi.0),
                            generic_type_params: Vec::new(),
                            generic_type_constraints: Vec::new(),
                            generic_type_defaults: Vec::new(),
                            generic_param_patterns: Vec::new(),
                            generic_param_optional: Vec::new(),
                            generic_return_type: None,
                            generic_return_pattern: None,
                        },
                    );
                    let patterns = method
                        .function
                        .params
                        .iter()
                        .map(|parameter| parameter.pat.clone())
                        .collect::<Vec<_>>();
                    let symbol = class_member_symbol(&name, method)?;
                    let native_rest = native_rest_element(&patterns, &signatures[&symbol].params)?;
                    signatures
                        .get_mut(&symbol)
                        .expect("class method signature")
                        .native_rest = native_rest;
                    let unbound_symbol = (method.kind == MethodKind::Method
                        && signatures[&symbol].uses_this)
                        .then(|| unbound_class_method_symbol(&symbol));
                    if let Some(unbound_symbol) = &unbound_symbol {
                        let mut unbound = signatures[&symbol].clone();
                        if !method.is_static {
                            unbound.params.remove(0);
                        }
                        unbound.uses_this = false;
                        signatures.insert(unbound_symbol.clone(), unbound);
                    }
                    if let Some(default_start) = trailing_omittable_start(&patterns) {
                        let receiver_count = usize::from(!method.is_static);
                        for arity in default_start..patterns.len() {
                            let total_arity = arity + receiver_count;
                            let mut wrapper = signatures[&symbol].clone();
                            wrapper.params.truncate(total_arity);
                            signatures.insert(default_arity_symbol(&symbol, total_arity), wrapper);
                        }
                    }
                    insert_omitted_parameter_signatures(
                        &mut signatures,
                        &class_member_symbol(&name, method)?,
                        &patterns,
                        usize::from(!method.is_static),
                    )?;
                    if let Some(unbound_symbol) = unbound_symbol {
                        if let Some(default_start) = trailing_omittable_start(&patterns) {
                            for arity in default_start..patterns.len() {
                                let mut wrapper = signatures[&unbound_symbol].clone();
                                wrapper.params.truncate(arity);
                                signatures
                                    .insert(default_arity_symbol(&unbound_symbol, arity), wrapper);
                            }
                        }
                        insert_omitted_parameter_signatures(
                            &mut signatures,
                            &unbound_symbol,
                            &patterns,
                            0,
                        )?;
                    }
                }
            }
            // Already consumed by `resolve_interfaces` above.
            ModuleItem::Stmt(Stmt::Decl(Decl::TsInterface(_))) => {}
            ModuleItem::Stmt(Stmt::Decl(Decl::TsEnum(_))) => {}
            ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(_))) => {}
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(var_decl))) => {
                for declaration in &var_decl.decls {
                    let Pat::Ident(binding) = &declaration.name else {
                        return Err(
                            "top-level destructuring declarations are not supported yet".into()
                        );
                    };
                    if declaration.init.is_none() {
                        return Err(format!(
                            "top-level binding `{}` needs an initializer",
                            binding.id.sym
                        ));
                    }
                }
                global_decls.push(var_decl.as_ref());
            }
            ModuleItem::Stmt(_) => {}
            ModuleItem::ModuleDecl(_) => {
                return Err("import/export declarations are not supported yet".into())
            }
        }
    }

    // JavaScript synthesizes `constructor(...args) { super(...args); }` for a
    // derived class without an explicit constructor. Propagate the nearest
    // base signature to a fixed point so forward declarations and multi-level
    // implicit constructor chains receive the same concrete argument tuple.
    for _ in 0..class_decls.len() {
        let mut changed = false;
        for derived in &class_decls {
            if derived
                .class
                .body
                .iter()
                .any(|member| matches!(member, ClassMember::Constructor(_)))
            {
                continue;
            }
            let Some(Expr::Ident(base)) = derived.class.super_class.as_deref() else {
                continue;
            };
            let derived_name = derived.ident.sym.as_ref();
            let base_params = signatures[&class_constructor_symbol(base.sym.as_ref())]
                .params
                .clone();
            let base_native_rest = signatures[&class_constructor_symbol(base.sym.as_ref())]
                .native_rest
                .clone();
            let constructor = signatures
                .get_mut(&class_constructor_symbol(derived_name))
                .expect("derived constructor signature");
            if constructor.params != base_params {
                constructor.params = base_params.clone();
                changed = true;
            }
            constructor.native_rest = base_native_rest.clone();
            let mut initializer_params = vec![interfaces[derived_name].clone()];
            initializer_params.extend(base_params.iter().cloned());
            let initializer = signatures
                .get_mut(&class_initializer_symbol(derived_name))
                .expect("derived initializer signature");
            initializer.params = initializer_params;
            initializer.native_rest = base_native_rest;
            for arity in 0..base_params.len() {
                let base_constructor =
                    default_arity_symbol(&class_constructor_symbol(base.sym.as_ref()), arity);
                if let Some(base_wrapper) = signatures.get(&base_constructor).cloned() {
                    let mut derived_wrapper = base_wrapper;
                    derived_wrapper.ret = interfaces[derived_name].clone();
                    signatures.insert(
                        default_arity_symbol(&class_constructor_symbol(derived_name), arity),
                        derived_wrapper,
                    );
                }
                let base_initializer =
                    default_arity_symbol(&class_initializer_symbol(base.sym.as_ref()), arity + 1);
                if let Some(base_wrapper) = signatures.get(&base_initializer).cloned() {
                    let mut derived_wrapper = base_wrapper;
                    derived_wrapper.params[0] = interfaces[derived_name].clone();
                    derived_wrapper.ret = interfaces[derived_name].clone();
                    signatures.insert(
                        default_arity_symbol(&class_initializer_symbol(derived_name), arity + 1),
                        derived_wrapper,
                    );
                }
            }
            if base_params.len() <= 16 {
                for mask in 1usize..(1usize << base_params.len()) {
                    let base_constructor = omitted_parameter_symbol(
                        &class_constructor_symbol(base.sym.as_ref()),
                        mask,
                    );
                    if let Some(base_wrapper) = signatures.get(&base_constructor).cloned() {
                        let mut derived_wrapper = base_wrapper;
                        derived_wrapper.ret = interfaces[derived_name].clone();
                        signatures.insert(
                            omitted_parameter_symbol(&class_constructor_symbol(derived_name), mask),
                            derived_wrapper,
                        );
                    }
                    let initializer_mask = mask << 1;
                    let base_initializer = omitted_parameter_symbol(
                        &class_initializer_symbol(base.sym.as_ref()),
                        initializer_mask,
                    );
                    if let Some(base_wrapper) = signatures.get(&base_initializer).cloned() {
                        let mut derived_wrapper = base_wrapper;
                        derived_wrapper.params[0] = interfaces[derived_name].clone();
                        derived_wrapper.ret = interfaces[derived_name].clone();
                        signatures.insert(
                            omitted_parameter_symbol(
                                &class_initializer_symbol(derived_name),
                                initializer_mask,
                            ),
                            derived_wrapper,
                        );
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    let class_by_name = class_decls
        .iter()
        .map(|declaration| (declaration.ident.sym.to_string(), *declaration))
        .collect::<HashMap<_, _>>();
    let member_key = |member: &swc_ecma_ast::ClassMethod| -> Result<(u8, bool, Symbol), String> {
        Ok((
            match member.kind {
                MethodKind::Method => 0,
                MethodKind::Getter => 1,
                MethodKind::Setter => 2,
            },
            member.is_static,
            class_property_name(&member.key)?,
        ))
    };
    let mut inherited_class_functions = Vec::new();
    let mut inherited_virtual_class_decls = Vec::new();
    for derived in &class_decls {
        let derived_name = derived.ident.sym.to_string();
        let derived_type = interfaces[&derived_name].clone();
        let mut seen = derived
            .class
            .body
            .iter()
            .filter_map(|member| match member {
                ClassMember::Method(method) => member_key(method).ok(),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let mut base_name =
            derived
                .class
                .super_class
                .as_ref()
                .and_then(|base| match base.as_ref() {
                    Expr::Ident(base) => Some(base.sym.to_string()),
                    _ => None,
                });
        while let Some(current_name) = base_name {
            // `Error`/`TypeError`/etc. are not real declared classes (see
            // `lower/module/classes.rs`'s synthetic base-layout branch) and
            // so have no members of their own to inherit here.
            let Some(&base) = class_by_name.get(&current_name) else {
                break;
            };
            for member in &base.class.body {
                let ClassMember::Method(method) = member else {
                    continue;
                };
                let key = member_key(method)?;
                let newly_seen = seen.insert(key);
                if method.is_abstract {
                    if !newly_seen {
                        let base_symbol = class_member_symbol(&current_name, method)?;
                        let derived_symbol = class_member_symbol(&derived_name, method)?;
                        let base_signature = &signatures[&base_symbol];
                        let derived_signature = &signatures[&derived_symbol];
                        let receiver_count = usize::from(!method.is_static);
                        if base_signature.params[receiver_count..]
                            != derived_signature.params[receiver_count..]
                            || base_signature.ret != derived_signature.ret
                            || base_signature.is_async != derived_signature.is_async
                            || base_signature.native_rest != derived_signature.native_rest
                        {
                            return Err(format!(
                                "class `{derived_name}` implements abstract member `{}` from `{current_name}` with an incompatible signature",
                                class_property_name(&method.key)?
                            ));
                        }
                        continue;
                    }
                    if !derived.class.is_abstract {
                        return Err(format!(
                            "concrete class `{derived_name}` must implement abstract member `{}` from `{current_name}`",
                            class_property_name(&method.key)?
                        ));
                    }
                    continue;
                }
                if !newly_seen {
                    continue;
                }
                let base_symbol = class_member_symbol(&current_name, method)?;
                let derived_symbol = class_member_symbol(&derived_name, method)?;
                let mut signature = signatures[&base_symbol].clone();
                if !method.is_static {
                    signature.params[0] = derived_type.clone();
                }
                signatures.insert(derived_symbol.clone(), signature.clone());
                if method.kind == MethodKind::Method && signature.uses_this {
                    let base_unbound = unbound_class_method_symbol(&base_symbol);
                    let derived_unbound = unbound_class_method_symbol(&derived_symbol);
                    let mut unbound_signature =
                        signatures.get(&base_unbound).cloned().unwrap_or_else(|| {
                            let mut unbound = signature.clone();
                            if !method.is_static {
                                unbound.params.remove(0);
                            }
                            unbound
                        });
                    unbound_signature.uses_this = false;
                    signatures.insert(derived_unbound, unbound_signature);
                }
                if !derived.class.is_abstract {
                    let mut inherited = (*base).clone();
                    inherited.ident = derived.ident.clone();
                    inherited.class.body = vec![ClassMember::Method(method.clone())];
                    inherited.class.is_abstract = false;
                    inherited.class.implements.clear();
                    inherited_virtual_class_decls.push(inherited);
                }

                let patterns = method
                    .function
                    .params
                    .iter()
                    .map(|parameter| parameter.pat.clone())
                    .collect::<Vec<_>>();
                let receiver_count = usize::from(!method.is_static);
                if let Some(default_start) = trailing_omittable_start(&patterns) {
                    for arity in default_start..patterns.len() {
                        let total_arity = arity + receiver_count;
                        let base_wrapper = default_arity_symbol(&base_symbol, total_arity);
                        let derived_wrapper = default_arity_symbol(&derived_symbol, total_arity);
                        let Some(mut wrapper_signature) = signatures.get(&base_wrapper).cloned()
                        else {
                            continue;
                        };
                        if !method.is_static {
                            wrapper_signature.params[0] = derived_type.clone();
                        }
                        signatures.insert(derived_wrapper.clone(), wrapper_signature.clone());
                    }
                }
                for mask in omitted_parameter_masks(&patterns, receiver_count)? {
                    let base_wrapper = omitted_parameter_symbol(&base_symbol, mask);
                    let derived_wrapper = omitted_parameter_symbol(&derived_symbol, mask);
                    let Some(mut wrapper_signature) = signatures.get(&base_wrapper).cloned() else {
                        continue;
                    };
                    if !method.is_static {
                        wrapper_signature.params[0] = derived_type.clone();
                    }
                    signatures.insert(derived_wrapper.clone(), wrapper_signature.clone());
                }
                if method.kind == MethodKind::Method && signature.uses_this {
                    let base_unbound = unbound_class_method_symbol(&base_symbol);
                    let derived_unbound = unbound_class_method_symbol(&derived_symbol);
                    if let Some(default_start) = trailing_omittable_start(&patterns) {
                        for arity in default_start..patterns.len() {
                            let base_wrapper = default_arity_symbol(&base_unbound, arity);
                            let derived_wrapper = default_arity_symbol(&derived_unbound, arity);
                            if let Some(wrapper) = signatures.get(&base_wrapper).cloned() {
                                signatures.insert(derived_wrapper, wrapper);
                            }
                        }
                    }
                    for mask in omitted_parameter_masks(&patterns, 0)? {
                        let base_wrapper = omitted_parameter_symbol(&base_unbound, mask);
                        let derived_wrapper = omitted_parameter_symbol(&derived_unbound, mask);
                        if let Some(wrapper) = signatures.get(&base_wrapper).cloned() {
                            signatures.insert(derived_wrapper, wrapper);
                        }
                    }
                }
            }
            base_name = base
                .class
                .super_class
                .as_ref()
                .and_then(|parent| match parent.as_ref() {
                    Expr::Ident(parent) => Some(parent.sym.to_string()),
                    _ => None,
                });
        }
    }

    let mut global_types = HashMap::new();
    let mut immutable_globals = HashSet::new();
    for declaration in &global_decls {
        for declarator in &declaration.decls {
            let Pat::Ident(binding) = &declarator.name else {
                unreachable!("top-level patterns were validated above")
            };
            let name = binding.id.sym.to_string();
            if global_types.contains_key(&name) || signatures.contains_key(&name) {
                return Err(format!("duplicate top-level binding `{name}`"));
            }
            let ty = binding
                .type_ann
                .as_ref()
                .map(|annotation| {
                    lower_ts_type(&annotation.type_ann, &interfaces, &generic_interfaces)
                })
                .transpose()?
                .unwrap_or(HirType::Dynamic);
            global_types.insert(name, ty);
            if declaration.kind == swc_ecma_ast::VarDeclKind::Const {
                immutable_globals.insert(binding.id.sym.to_string());
            }
        }
    }
    for declaration in &class_decls {
        let class_name = declaration.ident.sym.as_ref();
        let mut fields = HashSet::new();
        for member in &declaration.class.body {
            let ClassMember::ClassProp(property) = member else {
                continue;
            };
            if !property.is_static {
                continue;
            }
            let field = class_property_name(&property.key)?;
            if property.is_abstract || property.declare {
                return Err(format!(
                    "class `{class_name}` static field `{field}` cannot be abstract or ambient"
                ));
            }
            if !fields.insert(field.clone()) {
                return Err(format!(
                    "class `{class_name}` has duplicate static field `{field}`"
                ));
            }
            for member_symbol in [
                class_static_method_symbol(class_name, &field),
                class_getter_symbol(class_name, &field, true),
                class_setter_symbol(class_name, &field, true),
            ] {
                if signatures.contains_key(&member_symbol) {
                    return Err(format!(
                        "class `{class_name}` static field `{field}` collides with a static method or accessor"
                    ));
                }
            }
            let symbol = class_static_field_symbol(class_name, &field);
            let mut ty = match &property.type_ann {
                Some(annotation) => {
                    lower_ts_type(&annotation.type_ann, &interfaces, &generic_interfaces)?
                }
                None => property
                    .value
                    .as_deref()
                    .and_then(infer_class_field_literal_type)
                    .ok_or_else(|| {
                        format!(
                            "class `{class_name}` static field `{field}` needs a type annotation"
                        )
                    })?,
            };
            if property.is_optional {
                ty = optional_parameter_type(ty);
            }
            if property.value.is_none() && !matches!(ty, HirType::Optional(_) | HirType::Nullish(_))
            {
                return Err(format!(
                    "class `{class_name}` static field `{field}` without an initializer needs an optional or undefined-capable type"
                ));
            }
            if global_types.insert(symbol.clone(), ty).is_some() {
                return Err(format!(
                    "class `{class_name}` static field `{field}` conflicts with an existing generated binding"
                ));
            }
            if property.readonly {
                immutable_globals.insert(symbol);
            }
        }
    }

    // Inherited static fields share their declaring class's single storage.
    // Derived getters/setters provide the same dispatch surface as inherited
    // static accessors without copying the field into a second global.
    for derived in &class_decls {
        let derived_name = derived.ident.sym.as_ref();
        let mut seen = derived
            .class
            .body
            .iter()
            .filter_map(|member| match member {
                ClassMember::ClassProp(property) if property.is_static => {
                    class_property_name(&property.key).ok()
                }
                ClassMember::Method(method) if method.is_static => {
                    class_property_name(&method.key).ok()
                }
                _ => None,
            })
            .collect::<HashSet<_>>();
        let mut base_name =
            derived
                .class
                .super_class
                .as_ref()
                .and_then(|base| match base.as_ref() {
                    Expr::Ident(base) => Some(base.sym.to_string()),
                    _ => None,
                });
        while let Some(current_name) = base_name {
            // `Error`/`TypeError`/etc. are not real declared classes (see
            // `lower/module/classes.rs`'s synthetic base-layout branch) and
            // so have no members of their own to inherit here.
            let Some(&base) = class_by_name.get(&current_name) else {
                break;
            };
            for member in &base.class.body {
                let field = match member {
                    ClassMember::ClassProp(property) if property.is_static => {
                        class_property_name(&property.key)?
                    }
                    ClassMember::Method(method) if method.is_static => {
                        class_property_name(&method.key)?
                    }
                    _ => continue,
                };
                if !seen.insert(field.clone()) {
                    continue;
                }
                let ClassMember::ClassProp(property) = member else {
                    continue;
                };
                let storage = class_static_field_symbol(&current_name, &field);
                let ty = global_types[&storage].clone();
                let getter = class_getter_symbol(derived_name, &field, true);
                let source_range = (property.span.lo.0, property.span.hi.0);
                signatures.insert(
                    getter.clone(),
                    FnSignature {
                        params: Vec::new(),
                        variadic: None,
                        native_rest: None,
                        abstract_class_constructor: false,
                        ret: ty.clone(),
                        is_async: false,
                        uses_this: false,
                        is_extern: false,
                        source_range,
                        generic_type_params: Vec::new(),
                        generic_type_constraints: Vec::new(),
                        generic_type_defaults: Vec::new(),
                        generic_param_patterns: Vec::new(),
                        generic_param_optional: Vec::new(),
                        generic_return_type: None,
                        generic_return_pattern: None,
                    },
                );
                inherited_class_functions.push(HirFunction {
                    name: getter,
                    params: Vec::new(),
                    ret: ty.clone(),
                    is_async: false,
                    body: vec![HirStmt::Return(Some(HirExpr::Var(storage.clone())))],
                });
                if !property.readonly {
                    let setter = class_setter_symbol(derived_name, &field, true);
                    signatures.insert(
                        setter.clone(),
                        FnSignature {
                            params: vec![ty.clone()],
                            variadic: None,
                            native_rest: None,
                            abstract_class_constructor: false,
                            ret: ty.clone(),
                            is_async: false,
                            uses_this: false,
                            is_extern: false,
                            source_range,
                            generic_type_params: Vec::new(),
                            generic_type_constraints: Vec::new(),
                            generic_type_defaults: Vec::new(),
                            generic_param_patterns: Vec::new(),
                            generic_param_optional: Vec::new(),
                            generic_return_type: None,
                            generic_return_pattern: None,
                        },
                    );
                    let parameter = "__thaw_inherited_static_value".to_string();
                    inherited_class_functions.push(HirFunction {
                        name: setter,
                        params: vec![HirParam {
                            name: parameter.clone(),
                            ty: ty.clone(),
                        }],
                        ret: ty,
                        is_async: false,
                        body: vec![
                            HirStmt::Expr(HirExpr::Assign(
                                storage,
                                Box::new(HirExpr::Var(parameter.clone())),
                            )),
                            HirStmt::Return(Some(HirExpr::Var(parameter))),
                        ],
                    });
                }
            }
            base_name = base
                .class
                .super_class
                .as_ref()
                .and_then(|parent| match parent.as_ref() {
                    Expr::Ident(parent) => Some(parent.sym.to_string()),
                    _ => None,
                });
        }
    }

    // Missing parameter/return annotations and global initializer types are type
    // variables. Re-lower them together until forward references reach a fixed point.
    // signatures discovered in the previous round until forward calls and
    // mutually recursive functions reach a fixed point.
    for _ in 0..=((fn_decls.len() + global_types.len()) * 2 + 1) {
        let mut changed = false;
        let call_constraints = RefCell::new(Vec::new());
        let globals = lower_global_decls(
            &global_decls,
            &global_types,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            Some(&call_constraints),
        )?;
        for global in globals {
            let ty = global_types.get_mut(&global.name).unwrap();
            if hir_type_contains_dynamic(ty) && *ty != global.ty {
                *ty = global.ty;
                changed = true;
            }
        }
        let _ = lower_top_level_initializers(
            module,
            &global_types,
            &immutable_globals,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            Some(&call_constraints),
        )?;
        for fn_decl in &fn_decls {
            let name = fn_decl.ident.sym.to_string();
            let function = lower_fn_decl(
                fn_decl,
                &signatures,
                &interfaces,
                &generic_interfaces,
                &enum_values,
                &enum_reverse_values,
                &global_types,
                &immutable_globals,
                Some(&call_constraints),
            )?;
            if signatures[&name].ret == HirType::Dynamic && function.ret != HirType::Dynamic {
                signatures.get_mut(&name).unwrap().ret = function.ret;
                changed = true;
            }
        }
        for constraint in call_constraints.into_inner() {
            match constraint {
                CallConstraint::Generic(callee, types) => {
                    if types.contains(&HirType::Dynamic) {
                        continue;
                    }
                    if signatures
                        .get(&callee)
                        .is_some_and(|signature| signature.is_extern)
                    {
                        continue;
                    }
                    let instances = generic_instantiations.entry(callee).or_default();
                    if !instances.contains(&types) {
                        instances.push(types);
                    }
                }
                CallConstraint::Parameter(callee, index, actual, call_range) => {
                    if actual == HirType::Dynamic {
                        continue;
                    }
                    let is_extern = signatures[&callee].is_extern;
                    let param = &mut signatures.get_mut(&callee).unwrap().params[index];
                    if *param == HirType::Dynamic {
                        *param = actual;
                        changed = true;
                    } else if *param == HirType::Json
                        && actual == HirType::JsValue
                        && is_extern
                    {
                        // `Json` is an explicit host-marshalling boundary, not
                        // an inference placeholder. The call lowering wraps a
                        // live value (including native callbacks) for it.
                    } else if *param == HirType::Json && actual == HirType::JsValue {
                        *param = HirType::JsValue;
                        changed = true;
                    } else if *param != actual {
                        return Err(format!(
                            "conflicting inferred types for parameter {} of `{callee}`: {param:?} and {actual:?} at bytes {}..{}",
                            index + 1, call_range.0, call_range.1,
                        ));
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    if let Some((name, signature)) = signatures.iter().find(|(_, signature)| {
        !signature.is_extern
            && signature.generic_type_params.is_empty()
            && signature.params.contains(&HirType::Dynamic)
    }) {
        let index = signature
            .params
            .iter()
            .position(|ty| *ty == HirType::Dynamic)
            .unwrap();
        return Err(format!(
            "cannot infer parameter {} of function `{name}` from its call sites; add an explicit type annotation at bytes {}..{}",
            index + 1,
            signature.source_range.0,
            signature.source_range.1,
        ));
    }
    let unresolved = signatures.iter().find(|(_, signature)| {
        !signature.is_extern
            && signature.generic_type_params.is_empty()
            && signature.ret == HirType::Dynamic
    });
    if let Some((name, signature)) = unresolved {
        return Err(format!(
            "cannot infer the return type of function `{name}`; add an explicit return annotation at bytes {}..{}",
            signature.source_range.0,
            signature.source_range.1,
        ));
    }
    if let Some((name, _)) = global_types
        .iter()
        .find(|(_, ty)| hir_type_contains_dynamic(ty))
    {
        return Err(format!(
            "cannot infer the type of top-level binding `{name}`; add an explicit type annotation"
        ));
    }

    let mut globals = lower_global_decls(
        &global_decls,
        &global_types,
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
        None,
    )?;
    globals.extend(lower_static_class_globals(
        &class_decls,
        &global_types,
        &immutable_globals,
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
    )?);
    let initializers = lower_top_level_initializers(
        module,
        &global_types,
        &immutable_globals,
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
        None,
    )?;

    let extern_functions = signatures
        .iter()
        .filter(|(name, sig)| sig.is_extern && dynamic_symbol(name).is_none())
        .map(|(name, sig)| FfiSignature {
            symbol: name.clone(),
            params: sig.params.clone(),
            variadic: sig.variadic.clone(),
            variadic_abi: crate::FfiVariadicAbi::Native,
            ret: sig.ret.clone(),
            error_abi: FfiErrorAbi::Direct,
            return_ownership: FfiOwnership::Borrowed,
            error_ownership: FfiOwnership::Borrowed,
            param_string_abis: vec![FfiStringAbi::NullTerminated; sig.params.len()],
            return_string_abi: FfiStringAbi::NullTerminated,
            calling_convention: FfiCallingConvention::C,
            aggregate_return_abi: FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
        })
        .collect();

    let mut specialized = Vec::new();
    for fn_decl in fn_decls {
        let function = lower_fn_decl(
            fn_decl,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
            None,
        )?;
        if !signatures[&function.name].generic_type_params.is_empty() {
            continue;
        }
        let patterns = fn_decl
            .function
            .params
            .iter()
            .map(|parameter| parameter.pat.clone())
            .collect::<Vec<_>>();
        specialized.extend(lower_callable_default_wrappers(
            &function.name,
            &function.params,
            &patterns,
            0,
            None,
            &function.ret,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
        )?);
        specialized.extend(lower_callable_omitted_parameter_wrappers(
            &function.name,
            &function.params,
            &patterns,
            0,
            None,
            &function.ret,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
        )?);
        specialized.push(function);
    }
    for declaration in &class_decls {
        specialized.extend(lower_class_constructor(
            declaration,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
        )?);
    }
    for declaration in class_decls {
        specialized.extend(lower_class_methods(
            declaration,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
        )?);
    }
    for declaration in &inherited_virtual_class_decls {
        specialized.extend(lower_class_methods(
            declaration,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
        )?);
    }
    specialized.extend(inherited_class_functions);
    let mut pending = generic_instantiations
        .iter()
        .flat_map(|(name, instances)| instances.iter().cloned().map(|types| (name.clone(), types)))
        .collect::<Vec<_>>();
    let mut completed: Vec<(Symbol, Vec<HirType>)> = Vec::new();
    while let Some((name, types)) = pending.pop() {
        if completed
            .iter()
            .any(|done| done == &(name.clone(), types.clone()))
        {
            continue;
        }
        let decl = module
            .body
            .iter()
            .find_map(|item| match item {
                ModuleItem::Stmt(Stmt::Decl(Decl::Fn(decl))) if decl.ident.sym.as_ref() == name => {
                    Some(decl)
                }
                _ => None,
            })
            .ok_or_else(|| format!("missing declaration for `{name}`"))?;
        let nested_constraints = RefCell::new(Vec::new());
        let instance = lower_generic_instance(
            decl,
            &types,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
            Some(&nested_constraints),
        )?;
        completed.push((name, types));
        specialized.push(instance);
        for constraint in nested_constraints.into_inner() {
            if let CallConstraint::Generic(nested_name, nested_types) = constraint {
                if !nested_types.contains(&HirType::Dynamic) {
                    pending.push((nested_name, nested_types));
                }
            }
        }
    }

    Ok(HirProgram {
        globals,
        initializers,
        functions: specialized,
        extern_functions,
    })
}
