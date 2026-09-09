/// Resolves top-level interfaces into fixed native layouts.
fn collect_type_declarations<'a>(items: &'a [ModuleItem], output: &mut Vec<&'a Decl>) {
    for item in items {
        let declaration = match item {
            ModuleItem::Stmt(Stmt::Decl(declaration)) => Some(declaration),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
            _ => None,
        };
        let Some(declaration) = declaration else {
            continue;
        };
        if let Decl::TsModule(namespace) = declaration {
            if let Some(swc_ecma_ast::TsNamespaceBody::TsModuleBlock(block)) = &namespace.body {
                collect_type_declarations(&block.body, output);
            }
        } else {
            output.push(declaration);
        }
    }
}

fn resolve_interfaces(
    module: &Module,
) -> Result<(HashMap<Symbol, HirType>, GenericInterfaces<'_>), String> {
    let mut raw: HashMap<Symbol, &TsInterfaceDecl> = HashMap::new();
    let mut aliases = HashMap::new();
    let mut generic = GenericInterfaces::new();
    let mut declarations = Vec::new();
    collect_type_declarations(&module.body, &mut declarations);
    for declaration in declarations {
        match declaration {
            Decl::TsInterface(iface) => {
                let name = iface.id.sym.to_string();
                if let Some(parameters) = &iface.type_params {
                    validate_trailing_type_parameter_defaults(
                        "generic interface",
                        &name,
                        parameters,
                    )?;
                    generic.interfaces.insert(name, iface.as_ref());
                } else if matches!(
                    iface.body.body.as_slice(),
                    [TsTypeElement::TsCallSignatureDecl(call)] if call.type_params.is_some()
                ) {
                    generic.function_interfaces.insert(name, iface.as_ref());
                } else {
                    raw.insert(name.clone(), iface.as_ref());
                    generic.plain_interfaces.insert(name, iface.as_ref());
                }
            }
            Decl::TsTypeAlias(alias) => {
                let name = alias.id.sym.to_string();
                if matches!(
                    strip_parenthesized_ts_type(&alias.type_ann),
                    TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function))
                        if function.type_params.is_some()
                ) || matches!(
                    strip_parenthesized_ts_type(&alias.type_ann),
                    TsType::TsTypeLit(literal)
                        if matches!(literal.members.as_slice(),
                            [TsTypeElement::TsCallSignatureDecl(call)]
                                if call.type_params.is_some())
                ) {
                    generic.function_aliases.insert(name, alias.as_ref());
                } else if let Some(parameters) = &alias.type_params {
                    validate_trailing_type_parameter_defaults(
                        "generic type alias",
                        &name,
                        parameters,
                    )?;
                    generic.aliases.insert(name, alias.as_ref());
                } else {
                    generic.plain_aliases.insert(name.clone(), alias.as_ref());
                    aliases.insert(name, alias.as_ref());
                }
            }
            _ => {}
        }
    }

    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) = item else {
            continue;
        };
        let Some(return_type) = &function.function.return_type else {
            continue;
        };
        let discriminants = object_union_discriminants(&return_type.type_ann, &generic);
        if !discriminants.is_empty() {
            generic
                .function_discriminants
                .insert(function.ident.sym.to_string(), discriminants);
        }
        let array_discriminants =
            array_element_union_discriminants(&return_type.type_ann, &generic);
        if !array_discriminants.is_empty() {
            generic
                .function_array_discriminants
                .insert(function.ident.sym.to_string(), array_discriminants);
        }
        let nested_discriminants =
            nested_array_union_discriminants(&return_type.type_ann, &generic);
        if !nested_discriminants.is_empty() {
            generic
                .function_nested_array_discriminants
                .insert(function.ident.sym.to_string(), nested_discriminants);
        }
        let property_discriminants =
            object_array_property_discriminants(&return_type.type_ann, &generic);
        if !property_discriminants.is_empty() {
            generic
                .function_object_array_property_discriminants
                .insert(function.ident.sym.to_string(), property_discriminants);
        }
        let function_property_discriminants =
            object_function_property_discriminants(&return_type.type_ann, &generic);
        if !function_property_discriminants.is_empty() {
            generic
                .function_object_function_property_discriminants
                .insert(
                    function.ident.sym.to_string(),
                    function_property_discriminants,
                );
        }
    }

    loop {
        let callable_aliases = aliases
            .iter()
            .filter_map(|(name, alias)| {
                let TsType::TsTypeRef(reference) = strip_parenthesized_ts_type(&alias.type_ann)
                else {
                    return None;
                };
                let swc_ecma_ast::TsEntityName::Ident(target) = &reference.type_name else {
                    return None;
                };
                if reference.type_params.is_none()
                    && (generic.function_aliases.contains_key(target.sym.as_ref())
                        || generic
                            .function_interfaces
                            .contains_key(target.sym.as_ref())
                        || generic
                            .function_alias_chains
                            .contains_key(target.sym.as_ref())
                        || generic
                            .function_interface_chains
                            .contains_key(target.sym.as_ref()))
                {
                    Some((name.clone(), target.sym.to_string()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let callable_interfaces = raw
            .iter()
            .filter_map(|(name, interface)| {
                let [base] = interface.extends.as_slice() else {
                    return None;
                };
                if !interface.body.body.is_empty() || base.type_args.is_some() {
                    return None;
                }
                let Expr::Ident(target) = base.expr.as_ref() else {
                    return None;
                };
                (generic.function_aliases.contains_key(target.sym.as_ref())
                    || generic
                        .function_interfaces
                        .contains_key(target.sym.as_ref())
                    || generic
                        .function_alias_chains
                        .contains_key(target.sym.as_ref())
                    || generic
                        .function_interface_chains
                        .contains_key(target.sym.as_ref()))
                .then(|| (name.clone(), target.sym.to_string()))
            })
            .collect::<Vec<_>>();
        if callable_aliases.is_empty() && callable_interfaces.is_empty() {
            break;
        }
        for (name, target) in callable_aliases {
            aliases.remove(&name);
            generic.function_alias_chains.insert(name, target);
        }
        for (name, target) in callable_interfaces {
            raw.remove(&name);
            generic.function_interface_chains.insert(name, target);
        }
    }

    if let Some(name) = raw.keys().find(|name| aliases.contains_key(*name)) {
        return Err(format!(
            "type alias `{name}` conflicts with an interface declaration"
        ));
    }
    if let Some(name) = generic.aliases.keys().find(|name| {
        raw.contains_key(*name)
            || aliases.contains_key(*name)
            || generic.interfaces.contains_key(*name)
    }) {
        return Err(format!(
            "generic type alias `{name}` conflicts with another type declaration"
        ));
    }
    if let Some(name) = aliases
        .keys()
        .find(|name| generic.interfaces.contains_key(*name))
    {
        return Err(format!(
            "type alias `{name}` conflicts with a generic interface declaration"
        ));
    }
    if let Some(name) = generic.function_aliases.keys().find(|name| {
        raw.contains_key(*name)
            || aliases.contains_key(*name)
            || generic.interfaces.contains_key(*name)
            || generic.aliases.contains_key(*name)
            || generic.function_alias_chains.contains_key(*name)
            || generic.function_interface_chains.contains_key(*name)
    }) {
        return Err(format!(
            "generic function type alias `{name}` conflicts with another type declaration"
        ));
    }
    if let Some(name) = generic.function_interfaces.keys().find(|name| {
        raw.contains_key(*name)
            || aliases.contains_key(*name)
            || generic.interfaces.contains_key(*name)
            || generic.aliases.contains_key(*name)
            || generic.function_aliases.contains_key(*name)
            || generic.function_alias_chains.contains_key(*name)
            || generic.function_interface_chains.contains_key(*name)
    }) {
        return Err(format!(
            "generic callable interface `{name}` conflicts with another type declaration"
        ));
    }
    if let Some(name) = generic.function_alias_chains.keys().find(|name| {
        raw.contains_key(*name)
            || generic.interfaces.contains_key(*name)
            || generic.aliases.contains_key(*name)
            || generic.function_interface_chains.contains_key(*name)
    }) {
        return Err(format!(
            "callable type alias `{name}` conflicts with another type declaration"
        ));
    }
    if let Some(name) = generic.function_interface_chains.keys().find(|name| {
        aliases.contains_key(*name)
            || generic.interfaces.contains_key(*name)
            || generic.aliases.contains_key(*name)
    }) {
        return Err(format!(
            "callable interface `{name}` conflicts with another type declaration"
        ));
    }

    let mut resolved = HashMap::new();
    let names = raw
        .keys()
        .chain(aliases.keys())
        .cloned()
        .collect::<Vec<_>>();
    for name in names {
        resolve_named_type(
            &name,
            &raw,
            &aliases,
            &generic,
            &mut resolved,
            &mut Vec::new(),
        )?;
    }
    Ok((resolved, generic))
}

fn resolve_named_type(
    name: &str,
    interfaces: &HashMap<Symbol, &TsInterfaceDecl>,
    aliases: &HashMap<Symbol, &swc_ecma_ast::TsTypeAliasDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<Symbol, HirType>,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if let Some(ty) = resolved.get(name) {
        return Ok(ty.clone());
    }
    if in_progress.iter().any(|active| active == name) {
        if interfaces.contains_key(name) {
            return resolve_interface(name, interfaces, aliases, generic, resolved, in_progress);
        }
        let mut cycle = in_progress.clone();
        cycle.push(name.to_string());
        return Err(format!(
            "type declaration cycle `{}` cannot use Thaw's fixed-size native layout",
            cycle.join(" -> ")
        ));
    }
    if interfaces.contains_key(name) {
        resolve_interface(name, interfaces, aliases, generic, resolved, in_progress)
    } else if let Some(alias) = aliases.get(name) {
        in_progress.push(name.to_string());
        let ty = resolve_type_with_interfaces(
            &alias.type_ann,
            interfaces,
            aliases,
            generic,
            resolved,
            in_progress,
        )?;
        in_progress.pop();
        resolved.insert(name.to_string(), ty.clone());
        Ok(ty)
    } else {
        Err(format!("unknown type declaration `{name}`"))
    }
}

fn resolve_interface(
    name: &str,
    raw: &HashMap<Symbol, &TsInterfaceDecl>,
    aliases: &HashMap<Symbol, &swc_ecma_ast::TsTypeAliasDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<Symbol, HirType>,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if let Some(ty) = resolved.get(name) {
        return Ok(ty.clone());
    }
    if in_progress.iter().any(|n| n == name) {
        return Err(format!(
            "interface `{name}` is (indirectly) self-referential, which Thaw's fixed-size object layout can't represent"
        ));
    }

    let iface = raw
        .get(name)
        .ok_or_else(|| format!("unknown interface `{name}`"))?;

    if iface.type_params.is_some() {
        return Err(format!(
            "generic interfaces are not supported yet (`{name}`)"
        ));
    }

    in_progress.push(name.to_string());

    // `extends`: each base's fields come first, in `extends`-list order,
    // each base's own fields in its own declared order, followed by this
    // interface's own fields. A colliding field name (between two bases,
    // or between a base and this interface's own body) is rejected rather
    // than guessing an override/merge rule.
    let mut fields: Vec<(Symbol, HirType)> = Vec::new();
    let mut dictionary = None;
    for base in &iface.extends {
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            return Err(format!(
                "interface `{name}` has an unsupported `extends` target (only a plain interface name is supported)"
            ));
        };
        let base_name = base_ident.sym.to_string();
        let base_ty = if let Some(base_decl) = generic.interfaces.get(&base_name) {
            resolve_generic_interface_dependencies(
                &base_name,
                base_decl,
                raw,
                aliases,
                generic,
                resolved,
                in_progress,
                &mut Vec::new(),
            )?;
            let reference = swc_ecma_ast::TsTypeRef {
                span: base.span,
                type_name: swc_ecma_ast::TsEntityName::Ident(base_ident.clone()),
                type_params: base.type_args.clone(),
            };
            resolve_generic_interface(
                &base_name,
                base_decl,
                &reference,
                resolved,
                generic,
                None,
                in_progress,
            )?
        } else if let Some(base_decl) = generic.aliases.get(&base_name) {
            resolve_type_dependencies(
                &base_decl.type_ann,
                raw,
                aliases,
                generic,
                resolved,
                in_progress,
            )?;
            let reference = swc_ecma_ast::TsTypeRef {
                span: base.span,
                type_name: swc_ecma_ast::TsEntityName::Ident(base_ident.clone()),
                type_params: base.type_args.clone(),
            };
            resolve_generic_alias(
                &base_name,
                base_decl,
                &reference,
                resolved,
                generic,
                None,
                in_progress,
            )?
        } else {
            if base.type_args.is_some() {
                return Err(format!(
                    "interface `{name}` supplies type arguments to non-generic base `{base_name}`"
                ));
            }
            resolve_interface(&base_name, raw, aliases, generic, resolved, in_progress)?
        };
        let base_fields = match base_ty {
            HirType::Object(fields) => fields,
            HirType::Dictionary(element) => {
                if dictionary
                    .as_ref()
                    .is_some_and(|existing| existing != element.as_ref())
                {
                    return Err(format!(
                        "interface `{name}` inherits incompatible dictionary value types"
                    ));
                }
                dictionary = Some(*element);
                Vec::new()
            }
            _ => {
                return Err(format!(
                    "interface `{name}` can only extend object-shaped interface `{base_name}`"
                ))
            }
        };
        for (field_name, field_ty) in base_fields {
            if fields.iter().any(|(n, _)| *n == field_name) {
                return Err(format!(
                    "interface `{name}` inherits field `{field_name}` from `{base_name}`, which collides with an earlier field of the same name"
                ));
            }
            fields.push((field_name, field_ty));
        }
    }

    for member in &iface.body.body {
        if let TsTypeElement::TsIndexSignature(signature) = member {
            let value = resolve_type_with_interfaces(
                index_signature_value(signature)?,
                raw,
                aliases,
                generic,
                resolved,
                in_progress,
            )?;
            if dictionary
                .as_ref()
                .is_some_and(|existing| existing != &value)
            {
                return Err(format!(
                    "interface `{name}` declares an incompatible dictionary value type"
                ));
            }
            dictionary = Some(value);
            continue;
        }
        let (key, field_type, optional) = match member {
            TsTypeElement::TsPropertySignature(property) => {
                let annotation = property.type_ann.as_ref().ok_or_else(|| {
                    "interface property needs an explicit type annotation".to_string()
                })?;
                (
                    property.key.as_ref(),
                    annotation.type_ann.as_ref().clone(),
                    property.optional,
                )
            }
            TsTypeElement::TsMethodSignature(method) => (
                method.key.as_ref(),
                method_signature_function_type(method)?,
                false,
            ),
            _ => {
                return Err(format!(
                    "interface `{name}` has an unsupported member (properties and methods are supported; index signatures are not)"
                ))
            }
        };
        let field_name = match key {
            Expr::Ident(ident) => ident.sym.to_string(),
            _ => {
                return Err(format!(
                    "interface `{name}` has an unsupported property key"
                ))
            }
        };
        let mut field_ty = resolve_type_with_interfaces(
            &field_type,
            raw,
            aliases,
            generic,
            resolved,
            in_progress,
        )?;
        if optional {
            field_ty = optional_parameter_type(field_ty);
        }
        if let Some((_, existing)) = fields.iter().find(|(n, _)| *n == field_name) {
            if existing == &field_ty {
                continue;
            }
            return Err(format!(
                "interface `{name}` redeclares field `{field_name}` with an incompatible type"
            ));
        }
        if dictionary
            .as_ref()
            .is_some_and(|element| element != &field_ty)
        {
            return Err(format!(
                "interface `{name}` property `{field_name}` does not match its index value type"
            ));
        }
        fields.push((field_name, field_ty));
    }

    in_progress.pop();

    let hir_ty = if let Some(element) = dictionary {
        HirType::Dictionary(Box::new(element))
    } else {
        HirType::Object(fields)
    };
    resolved.insert(name.to_string(), hir_ty.clone());
    Ok(hir_ty)
}

#[allow(clippy::too_many_arguments)]
fn resolve_generic_interface_dependencies(
    name: &str,
    decl: &TsInterfaceDecl,
    raw: &HashMap<Symbol, &TsInterfaceDecl>,
    aliases: &HashMap<Symbol, &swc_ecma_ast::TsTypeAliasDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<Symbol, HirType>,
    in_progress: &mut Vec<Symbol>,
    dependency_progress: &mut Vec<Symbol>,
) -> Result<(), String> {
    if dependency_progress.iter().any(|active| active == name) {
        return Ok(());
    }
    dependency_progress.push(name.to_string());
    for base in &decl.extends {
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            return Err(format!(
                "generic interface `{name}` has an unsupported `extends` target"
            ));
        };
        let base_name = base_ident.sym.as_str();
        if raw.contains_key(base_name) || aliases.contains_key(base_name) {
            resolve_named_type(base_name, raw, aliases, generic, resolved, in_progress)?;
        } else if let Some(base_decl) = generic.interfaces.get(base_name) {
            resolve_generic_interface_dependencies(
                base_name,
                base_decl,
                raw,
                aliases,
                generic,
                resolved,
                in_progress,
                dependency_progress,
            )?;
        }
    }
    let inserted_progress = !in_progress.iter().any(|active| active == name);
    if inserted_progress {
        in_progress.push(name.to_string());
    }
    for member in &decl.body.body {
        let ty = match member {
            TsTypeElement::TsPropertySignature(property) => property
                .type_ann
                .as_ref()
                .map(|value| value.type_ann.clone()),
            TsTypeElement::TsMethodSignature(method) => {
                Some(Box::new(method_signature_function_type(method)?))
            }
            _ => None,
        };
        if let Some(ty) = ty {
            resolve_type_dependencies(&ty, raw, aliases, generic, resolved, in_progress)?;
        }
    }
    if inserted_progress {
        in_progress.pop();
    }
    dependency_progress.pop();
    Ok(())
}

/// Like `lower_ts_type`, but additionally resolves a `TsTypeRef` naming a
/// not-yet-resolved interface, recursively. Used only while building the
/// interface table (`resolve_interfaces`); everywhere else, `lower_ts_type`
/// consults the finished, read-only table instead.
fn resolve_type_with_interfaces(
    ty: &TsType,
    raw: &HashMap<Symbol, &TsInterfaceDecl>,
    aliases: &HashMap<Symbol, &swc_ecma_ast::TsTypeAliasDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<Symbol, HirType>,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    resolve_type_dependencies(ty, raw, aliases, generic, resolved, in_progress)?;
    lower_ts_type(ty, resolved, generic)
}

fn resolve_type_dependencies(
    ty: &TsType,
    interfaces: &HashMap<Symbol, &TsInterfaceDecl>,
    aliases: &HashMap<Symbol, &swc_ecma_ast::TsTypeAliasDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<Symbol, HirType>,
    in_progress: &mut Vec<Symbol>,
) -> Result<(), String> {
    match ty {
        TsType::TsTypeRef(reference) => {
            if let swc_ecma_ast::TsEntityName::Ident(id) = &reference.type_name {
                let name = id.sym.as_str();
                if interfaces.contains_key(name) || aliases.contains_key(name) {
                    resolve_named_type(name, interfaces, aliases, generic, resolved, in_progress)?;
                } else if let Some(decl) = generic.interfaces.get(name) {
                    if !in_progress.iter().any(|active| active == name) {
                        resolve_generic_interface_dependencies(
                            name,
                            decl,
                            interfaces,
                            aliases,
                            generic,
                            resolved,
                            in_progress,
                            &mut Vec::new(),
                        )?;
                    }
                } else if let Some(decl) = generic.aliases.get(name) {
                    if !in_progress.iter().any(|active| active == name) {
                        in_progress.push(name.to_string());
                        resolve_type_dependencies(
                            &decl.type_ann,
                            interfaces,
                            aliases,
                            generic,
                            resolved,
                            in_progress,
                        )?;
                        in_progress.pop();
                    }
                }
            }
            if let Some(arguments) = &reference.type_params {
                for argument in &arguments.params {
                    resolve_type_dependencies(
                        argument,
                        interfaces,
                        aliases,
                        generic,
                        resolved,
                        in_progress,
                    )?;
                }
            }
        }
        TsType::TsParenthesizedType(parenthesized) => resolve_type_dependencies(
            &parenthesized.type_ann,
            interfaces,
            aliases,
            generic,
            resolved,
            in_progress,
        )?,
        TsType::TsArrayType(array) => resolve_type_dependencies(
            &array.elem_type,
            interfaces,
            aliases,
            generic,
            resolved,
            in_progress,
        )?,
        TsType::TsTypeOperator(operator)
            if operator.op == swc_ecma_ast::TsTypeOperatorOp::ReadOnly =>
        {
            resolve_type_dependencies(
                &operator.type_ann,
                interfaces,
                aliases,
                generic,
                resolved,
                in_progress,
            )?
        }
        TsType::TsTupleType(tuple) => {
            for element in &tuple.elem_types {
                resolve_type_dependencies(
                    &element.ty,
                    interfaces,
                    aliases,
                    generic,
                    resolved,
                    in_progress,
                )?;
            }
        }
        TsType::TsUnionOrIntersectionType(value) => {
            let elements = match value {
                TsUnionOrIntersectionType::TsUnionType(union) => &union.types,
                TsUnionOrIntersectionType::TsIntersectionType(intersection) => &intersection.types,
            };
            for element in elements {
                resolve_type_dependencies(
                    element,
                    interfaces,
                    aliases,
                    generic,
                    resolved,
                    in_progress,
                )?;
            }
        }
        TsType::TsTypeLit(literal) => {
            for member in &literal.members {
                let ty = match member {
                    TsTypeElement::TsPropertySignature(property) => property
                        .type_ann
                        .as_ref()
                        .map(|value| value.type_ann.clone()),
                    TsTypeElement::TsMethodSignature(method) => {
                        Some(Box::new(method_signature_function_type(method)?))
                    }
                    _ => None,
                };
                if let Some(ty) = ty {
                    resolve_type_dependencies(
                        &ty,
                        interfaces,
                        aliases,
                        generic,
                        resolved,
                        in_progress,
                    )?;
                }
            }
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            for parameter in &function.params {
                if let TsFnParam::Ident(parameter) = parameter {
                    if let Some(annotation) = &parameter.type_ann {
                        resolve_type_dependencies(
                            &annotation.type_ann,
                            interfaces,
                            aliases,
                            generic,
                            resolved,
                            in_progress,
                        )?;
                    }
                }
            }
            resolve_type_dependencies(
                &function.type_ann.type_ann,
                interfaces,
                aliases,
                generic,
                resolved,
                in_progress,
            )?;
        }
        _ => {}
    }
    Ok(())
}
