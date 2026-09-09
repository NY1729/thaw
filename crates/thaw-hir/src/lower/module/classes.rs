fn collect_native_classes<'a>(
    module: &'a Module,
    interfaces: &mut HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Vec<&'a ClassDecl>, String> {
    fn method_type(
        class: &str,
        method: &ClassMethod,
        interfaces: &HashMap<Symbol, HirType>,
        generic_interfaces: &GenericInterfaces,
    ) -> Result<HirType, String> {
        if method.function.type_params.is_some() {
            return Ok(HirType::Dynamic);
        }
        let substitution = HashMap::new();
        let mut params = method
            .function
            .params
            .iter()
            .map(|param| {
                lower_param(
                    &param.pat,
                    interfaces,
                    generic_interfaces,
                    false,
                    &substitution,
                )
                .map(|param| param.ty)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let optional = method
            .function
            .params
            .iter()
            .map(|param| {
                matches!(&param.pat, Pat::Ident(binding) if binding.id.optional)
                    || matches!(param.pat, Pat::Assign(_))
            })
            .collect::<Vec<_>>();
        for (param, optional) in params.iter_mut().zip(&optional) {
            if *optional {
                *param = optional_parameter_type(param.clone());
            }
        }
        let rest = if method
            .function
            .params
            .last()
            .is_some_and(|param| matches!(param.pat, Pat::Rest(_)))
        {
            let HirType::Array(element) = params.pop().expect("rest parameter has a type") else {
                return Err("method rest parameter needs an array type annotation".into());
            };
            Some(element)
        } else {
            None
        };
        let result = Box::new(if method.function.is_generator {
            lower_generator_return_type(
                method.function.is_async,
                &method.function.return_type,
                class,
                interfaces,
                generic_interfaces,
                &substitution,
            )?
        } else {
            lower_fn_return_type(
                method.function.is_async,
                &method.function.return_type,
                class,
                interfaces,
                generic_interfaces,
                &substitution,
            )?
        });
        Ok(if rest.is_some() || optional.iter().any(|value| *value) {
            HirType::CallableFunction(
                params,
                optional_parameter_mask(&optional[..optional.len() - usize::from(rest.is_some())]),
                rest,
                result,
            )
        } else {
            HirType::Function(params, result)
        })
    }

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
                // `Error`/`TypeError`/etc. are not real declared classes --
                // `new Error(message)` lowers straight to a tagged string
                // (see `lower/expressions/lowering.rs`), not an object. A
                // user class extending one of them still gets a real
                // object layout here (an inherited `message: Str` field
                // plus its own identity marker), so normal construction,
                // field access, and `instanceof` on the object all work
                // the usual way; `throw`ing such an instance is what
                // recovers the tagged-string form other exception readers
                // (`.message`/`.name`/`instanceof` on the *caught* value,
                // console.log, N-API, Lambda error reporting) already
                // understand -- see `lower/statements/lowering.rs`.
                if !matches!(
                    base_name,
                    "Error"
                        | "TypeError"
                        | "RangeError"
                        | "SyntaxError"
                        | "ReferenceError"
                        | "EvalError"
                        | "URIError"
                ) {
                    return Err(format!(
                        "class `{name}` extends unknown native class `{base_name}`"
                    ));
                }
                identities.push(base_name.to_string());
                inherited_fields.push(("message".to_string(), HirType::Str));
            } else {
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
                    // `Error`/`TypeError`/etc. are not real declared classes
                    // (see the synthetic base-layout branch above) and so
                    // have no abstract members of their own to check.
                    let Some(&base) = declarations.get(&current) else {
                        break;
                    };
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
                let field_type = fields
                    .iter()
                    .find_map(|(candidate, ty)| (candidate == field).then_some(ty.clone()));
                let mut implemented_method = None;
                let mut owner = Some(*declaration);
                while let Some(candidate) = owner {
                    implemented_method = candidate.class.body.iter().find_map(|member| {
                        let ClassMember::Method(method) = member else {
                            return None;
                        };
                        (!method.is_static
                            && method.kind == MethodKind::Method
                            && class_property_name(&method.key).ok().as_deref() == Some(field))
                        .then(|| method_type(name, method, interfaces, generic_interfaces))
                    });
                    if implemented_method.is_some() {
                        break;
                    }
                    owner = candidate
                        .class
                        .super_class
                        .as_deref()
                        .and_then(Expr::as_ident)
                        .and_then(|parent| declarations.get(parent.sym.as_ref()).copied());
                }
                let actual_type = match (field_type, implemented_method) {
                    (Some(ty), _) => ty,
                    (None, Some(ty)) => ty?,
                    (None, None) => {
                        return Err(format!(
                            "class `{name}` is missing field `{field}` required by `{}`",
                            target.sym
                        ))
                    }
                };
                if &actual_type != required_type {
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
            continue;
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
            // `Error`/`TypeError`/etc. are not real declared classes (see
            // the synthetic base-layout branch above) and so have no
            // `ClassMember`s of their own to walk.
            let Some(&class) = declarations.get(&current_name) else {
                break;
            };
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
