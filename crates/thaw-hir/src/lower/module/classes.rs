fn collect_native_classes<'a>(
    module: &'a Module,
    interfaces: &mut HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Vec<&'a ClassDecl>, String> {
    fn resolve_layout(
        name: &str,
        declarations: &HashMap<Symbol, &ClassDecl>,
        interfaces: &mut HashMap<Symbol, HirType>,
        generic_interfaces: &GenericInterfaces,
        resolved: &mut HashSet<Symbol>,
        active: &mut Vec<Symbol>,
    ) -> Result<(), String> {
        if resolved.contains(name) {
            return Ok(());
        }
        if active.iter().any(|current| current == name) {
            active.push(name.to_string());
            return Err(format!("class inheritance cycle `{}`", active.join(" -> ")));
        }
        let declaration = declarations
            .get(name)
            .ok_or_else(|| format!("unknown native class `{name}`"))?;
        active.push(name.to_string());
        let mut inherited_fields = Vec::new();
        let mut identities = vec![name.to_string()];
        if let Some(base) = &declaration.class.super_class {
            let Expr::Ident(base) = base.as_ref() else {
                return Err(format!(
                    "class `{name}` requires an identifier in its extends clause"
                ));
            };
            let base_name = base.sym.as_ref();
            if !declarations.contains_key(base_name) {
                return Err(format!(
                    "class `{name}` extends unknown native class `{base_name}`"
                ));
            }
            resolve_layout(
                base_name,
                declarations,
                interfaces,
                generic_interfaces,
                resolved,
                active,
            )?;
            let HirType::Object(base_fields) = &interfaces[base_name] else {
                unreachable!("native class layouts are objects")
            };
            if let Some((marker, HirType::Bool)) = base_fields.first() {
                let inherited = marker
                    .strip_prefix("__thaw_class_identity_")
                    .expect("base class identity marker");
                identities.extend(inherited.split('$').map(str::to_owned));
            }
            inherited_fields.extend(base_fields.iter().skip(1).cloned());
        }
        let mut fields = vec![(
            format!("__thaw_class_identity_{}", identities.join("$")),
            HirType::Bool,
        )];
        fields.extend(inherited_fields);
        for member in &declaration.class.body {
            let ClassMember::ClassProp(property) = member else {
                continue;
            };
            if property.is_static {
                continue;
            }
            if property.declare {
                return Err(format!(
                    "class `{name}` field `{}` cannot be ambient",
                    class_property_name(&property.key)?
                ));
            }
            let field_name = class_property_name(&property.key)?;
            let mut field_type = match &property.type_ann {
                Some(annotation) => {
                    lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)?
                }
                None => property
                    .value
                    .as_deref()
                    .and_then(infer_class_field_literal_type)
                    .ok_or_else(|| {
                        format!("class `{name}` field `{field_name}` needs a type annotation")
                    })?,
            };
            if property.is_optional {
                field_type = optional_parameter_type(field_type);
            }
            if let Some((_, inherited_type)) =
                fields.iter().find(|(existing, _)| existing == &field_name)
            {
                let mut base_name =
                    declaration
                        .class
                        .super_class
                        .as_deref()
                        .and_then(|base| match base {
                            Expr::Ident(base) => Some(base.sym.to_string()),
                            _ => None,
                        });
                let mut overrides_abstract = false;
                while let Some(current) = base_name {
                    let base = declarations[&current];
                    if base.class.body.iter().any(|member| {
                        matches!(member, ClassMember::ClassProp(candidate)
                            if !candidate.is_static
                                && candidate.is_abstract
                                && class_property_name(&candidate.key).ok().as_deref()
                                    == Some(field_name.as_str()))
                    }) {
                        overrides_abstract = true;
                        break;
                    }
                    base_name = base
                        .class
                        .super_class
                        .as_deref()
                        .and_then(|parent| match parent {
                            Expr::Ident(parent) => Some(parent.sym.to_string()),
                            _ => None,
                        });
                }
                if !overrides_abstract {
                    return Err(format!(
                        "class `{name}` field `{field_name}` collides with an inherited or local field"
                    ));
                }
                if inherited_type != &field_type {
                    return Err(format!(
                        "class `{name}` implements abstract field `{field_name}` with type {field_type:?}, expected {inherited_type:?}"
                    ));
                }
            } else {
                fields.push((field_name, field_type));
            }
        }
        for property in declaration
            .class
            .body
            .iter()
            .filter_map(|member| match member {
                ClassMember::Constructor(constructor) => Some(&constructor.params),
                _ => None,
            })
            .flatten()
            .filter_map(|parameter| match parameter {
                ParamOrTsParamProp::TsParamProp(property) => Some(property),
                ParamOrTsParamProp::Param(_) => None,
            })
        {
            let binding = parameter_property_binding(property)?;
            let field_name = binding.id.sym.to_string();
            if fields.iter().any(|(existing, _)| existing == &field_name) {
                return Err(format!(
                    "class `{name}` parameter property duplicates an inherited or local field `{field_name}`"
                ));
            }
            let annotation = binding.type_ann.as_ref().ok_or_else(|| {
                format!("class `{name}` parameter property `{field_name}` needs a type annotation")
            })?;
            let mut ty = lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)?;
            if binding.id.optional {
                ty = optional_parameter_type(ty);
            }
            fields.push((field_name, ty));
        }
        for implementation in &declaration.class.implements {
            let Expr::Ident(target) = implementation.expr.as_ref() else {
                return Err(format!(
                    "class `{name}` requires an identifier in its implements clause"
                ));
            };
            if declarations.contains_key(target.sym.as_ref()) {
                return Err(format!(
                    "class `{name}` cannot use native class `{}` as an implements target",
                    target.sym
                ));
            }
            let reference = TsType::TsTypeRef(swc_ecma_ast::TsTypeRef {
                span: implementation.span,
                type_name: swc_ecma_ast::TsEntityName::Ident(target.clone()),
                type_params: implementation.type_args.clone(),
            });
            let required =
                lower_ts_type(&reference, interfaces, generic_interfaces).map_err(|error| {
                    format!(
                        "class `{name}` has invalid implements target `{}`: {error}",
                        target.sym
                    )
                })?;
            let HirType::Object(required_fields) = required else {
                return Err(format!(
                    "class `{name}` implements non-object type `{}`",
                    target.sym
                ));
            };
            for (field, required_type) in &required_fields {
                let actual_type = fields
                    .iter()
                    .find_map(|(candidate, ty)| (candidate == field).then_some(ty))
                    .ok_or_else(|| {
                        format!(
                            "class `{name}` is missing field `{field}` required by `{}`",
                            target.sym
                        )
                    })?;
                if actual_type != required_type {
                    return Err(format!(
                        "class `{name}` field `{field}` has type {actual_type:?}, but `{}` requires {required_type:?}",
                        target.sym
                    ));
                }
            }
        }
        interfaces.insert(name.to_string(), HirType::Object(fields));
        active.pop();
        resolved.insert(name.to_string());
        Ok(())
    }

    let mut classes = Vec::new();
    let mut declarations = HashMap::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) = item else {
            continue;
        };
        let name = declaration.ident.sym.to_string();
        if declaration.declare {
            return Err(format!(
                "ambient class `{name}` cannot use the native class path"
            ));
        }
        if declaration.class.type_params.is_some() {
            return Err(format!(
                "class `{name}` currently requires a non-generic class"
            ));
        }
        if !declaration.class.is_abstract
            && declaration.class.body.iter().any(|member| match member {
                ClassMember::Method(method) => method.is_abstract,
                ClassMember::ClassProp(property) => property.is_abstract,
                _ => false,
            })
        {
            return Err(format!(
                "concrete class `{name}` cannot declare abstract members"
            ));
        }
        if interfaces.contains_key(&name)
            || declarations.insert(name.clone(), declaration).is_some()
        {
            return Err(format!(
                "class `{name}` conflicts with an interface or type declaration"
            ));
        }
        classes.push(declaration);
    }
    let mut resolved = HashSet::new();
    for declaration in &classes {
        resolve_layout(
            declaration.ident.sym.as_ref(),
            &declarations,
            interfaces,
            generic_interfaces,
            &mut resolved,
            &mut Vec::new(),
        )?;
    }
    for declaration in &classes {
        if declaration.class.is_abstract {
            continue;
        }
        let name = declaration.ident.sym.as_ref();
        let mut seen = HashSet::new();
        let mut current = Some(name.to_string());
        while let Some(current_name) = current {
            let class = declarations[&current_name];
            for member in &class.class.body {
                let ClassMember::ClassProp(property) = member else {
                    continue;
                };
                if property.is_static {
                    continue;
                }
                let field = class_property_name(&property.key)?;
                if !seen.insert(field.clone()) {
                    continue;
                }
                if property.is_abstract {
                    return Err(format!(
                        "concrete class `{name}` must implement abstract field `{field}` from `{current_name}`"
                    ));
                }
            }
            current = class
                .class
                .super_class
                .as_deref()
                .and_then(|parent| match parent {
                    Expr::Ident(parent) => Some(parent.sym.to_string()),
                    _ => None,
                });
        }
    }
    Ok(classes)
}
