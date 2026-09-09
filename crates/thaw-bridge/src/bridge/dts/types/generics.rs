/// Resolves `Name<ConcreteArg, ...>` for a generic interface `Name`,
/// mirroring `thaw_hir::lower::resolve_generic_interface` but degrading to
/// `DtsType::Unsupported` instead of erroring (wrong argument count,
/// self-reference, `extends`, or an unsupported member all degrade rather
/// than abort). Same scope limits as the thaw-hir version.
fn resolve_generic_interface(
    name: &str,
    decl: &TsInterfaceDecl,
    ty_ref: &swc_ecma_ast::TsTypeRef,
    outer_substitution: Option<&HashMap<String, HirType>>,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if in_progress.iter().any(|n| n == name) {
        return DtsType::Unsupported(format!(
            "generic interface `{name}` is (indirectly) self-referential"
        ));
    }
    if !decl.extends.is_empty() {
        return DtsType::Unsupported(format!(
            "generic interface `{name}` cannot use `extends` yet"
        ));
    }

    let type_param_decl = decl
        .type_params
        .as_ref()
        .expect("caller only reaches here for a generic interface");
    let type_param_names: Vec<String> = type_param_decl
        .params
        .iter()
        .map(|p| p.name.sym.to_string())
        .collect();

    let type_args: &[Box<TsType>] = ty_ref
        .type_params
        .as_ref()
        .map(|params| params.params.as_slice())
        .unwrap_or(&[]);
    if type_args.len() != type_param_names.len() {
        return DtsType::Unsupported(format!(
            "interface `{name}` expects {} type argument(s), got {}",
            type_param_names.len(),
            type_args.len()
        ));
    }

    let mut substitution: HashMap<String, HirType> = HashMap::new();
    for (param_name, arg) in type_param_names.into_iter().zip(type_args) {
        let resolved = outer_substitution.map_or_else(
            || classify_ts_type(arg, interfaces, generic_interfaces),
            |outer| {
                resolve_ts_type_with_substitution(
                    arg,
                    outer,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )
            },
        );
        match resolved {
            DtsType::Native(ty) => {
                substitution.insert(param_name, ty);
            }
            DtsType::Unsupported(reason) => {
                return DtsType::Unsupported(format!("type argument for `{param_name}`: {reason}"))
            }
        }
    }

    in_progress.push(name.to_string());

    let mut fields = Vec::with_capacity(decl.body.body.len());
    let mut dictionary = None;
    let mut failure = None;
    for member in &decl.body.body {
        if let TsTypeElement::TsIndexSignature(signature) = member {
            let value = match index_signature_value(signature) {
                Ok(value) => resolve_ts_type_with_substitution(
                    value,
                    &substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                ),
                Err(reason) => DtsType::Unsupported(reason),
            };
            match value {
                DtsType::Native(value)
                    if dictionary.as_ref().is_none_or(|existing| existing == &value)
                        && fields.iter().all(|(_, field)| field == &value) =>
                {
                    dictionary = Some(value);
                }
                DtsType::Native(_) => {
                    failure = Some("declares an incompatible dictionary value type".into());
                    break;
                }
                DtsType::Unsupported(reason) => {
                    failure = Some(reason);
                    break;
                }
            }
            continue;
        }
        let TsTypeElement::TsPropertySignature(prop) = member else {
            failure = Some("has a non-property member (method/index signature)".to_string());
            break;
        };
        let Some(field_name) = type_property_name(&prop.key) else {
            failure = Some("has an unsupported property key".to_string());
            break;
        };
        let field_ty = match &prop.type_ann {
            Some(ann) => resolve_ts_type_with_substitution(
                &ann.type_ann,
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ),
            None => DtsType::Unsupported(format!("field `{field_name}` has no type annotation")),
        };
        match field_ty {
            DtsType::Native(ty) => {
                let ty = if prop.optional {
                    optional_hir_type(ty)
                } else {
                    ty
                };
                if dictionary.as_ref().is_some_and(|element| element != &ty) {
                    failure = Some(format!(
                        "field `{field_name}` does not match its index value type"
                    ));
                    break;
                }
                fields.push((field_name, ty));
            }
            DtsType::Unsupported(reason) => {
                failure = Some(format!("field `{field_name}`: {reason}"));
                break;
            }
        }
    }

    in_progress.pop();

    match failure {
        Some(reason) => DtsType::Unsupported(format!("interface `{name}` {reason}")),
        None => DtsType::Native(dictionary.map_or(HirType::Object(fields), |element| {
            HirType::Dictionary(Box::new(element))
        })),
    }
}

fn bridge_type_satisfies_constraint(actual: &HirType, constraint: &HirType) -> bool {
    if actual == constraint {
        return true;
    }
    match constraint {
        HirType::Union(elements) => elements
            .iter()
            .any(|element| bridge_type_satisfies_constraint(actual, element)),
        HirType::Object(required) => match actual {
            HirType::Object(fields) => required.iter().all(|(name, ty)| {
                fields
                    .iter()
                    .find(|(field, _)| field == name)
                    .is_some_and(|(_, actual)| bridge_type_satisfies_constraint(actual, ty))
            }),
            _ => false,
        },
        _ => false,
    }
}

fn resolve_generic_alias(
    name: &str,
    decl: &swc_ecma_ast::TsTypeAliasDecl,
    ty_ref: &swc_ecma_ast::TsTypeRef,
    outer_substitution: Option<&HashMap<String, HirType>>,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if in_progress.iter().any(|active| active == name) {
        return DtsType::Unsupported(format!(
            "generic type alias `{name}` is (indirectly) self-referential"
        ));
    }
    let parameters = &decl
        .type_params
        .as_ref()
        .expect("caller only reaches generic aliases")
        .params;
    let arguments = ty_ref
        .type_params
        .as_ref()
        .map(|parameters| parameters.params.as_slice())
        .unwrap_or_default();
    let required = parameters
        .iter()
        .take_while(|parameter| parameter.default.is_none())
        .count();
    if arguments.len() < required || arguments.len() > parameters.len() {
        return DtsType::Unsupported(format!(
            "type alias `{name}` expects {}..={} type argument(s), got {}",
            required,
            parameters.len(),
            arguments.len()
        ));
    }

    let mut substitution = HashMap::new();
    for (index, parameter) in parameters.iter().enumerate() {
        let concrete = if let Some(argument) = arguments.get(index) {
            match outer_substitution {
                Some(outer) => resolve_ts_type_with_substitution(
                    argument,
                    outer,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                ),
                None => classify_ts_type(argument, interfaces, generic_interfaces),
            }
        } else {
            resolve_ts_type_with_substitution(
                parameter
                    .default
                    .as_ref()
                    .expect("arity validation requires a default"),
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )
        };
        let concrete = match concrete {
            DtsType::Native(concrete) => concrete,
            unsupported => return unsupported,
        };
        if let Some(constraint) = &parameter.constraint {
            let constraint = resolve_ts_type_with_substitution(
                constraint,
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            );
            let constraint = match constraint {
                DtsType::Native(constraint) => constraint,
                unsupported => return unsupported,
            };
            if !bridge_type_satisfies_constraint(&concrete, &constraint) {
                return DtsType::Unsupported(format!(
                    "type argument {concrete:?} does not satisfy constraint {constraint:?} for `{}` in alias `{name}`",
                    parameter.name.sym
                ));
            }
        }
        substitution.insert(parameter.name.sym.to_string(), concrete);
    }

    in_progress.push(name.to_string());
    let result = resolve_ts_type_with_substitution(
        &decl.type_ann,
        &substitution,
        interfaces,
        generic_interfaces,
        in_progress,
    );
    in_progress.pop();
    result
}

/// Like `classify_ts_type`, but a bare `TsTypeRef` matching one of `Name`'s
/// type parameters resolves to the corresponding concrete type instead of
/// an unknown-reference `Unsupported`. Mirrors
/// `thaw_hir::lower::resolve_ts_type_with_substitution`.
fn resolve_ts_type_with_substitution(
    ty: &TsType,
    substitution: &HashMap<String, HirType>,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let TsType::TsTypeRef(ty_ref) = ty {
        if let TsEntityName::Ident(id) = &ty_ref.type_name {
            let ref_name = id.sym.as_str();
            if let Some(concrete) = substitution.get(ref_name) {
                return DtsType::Native(concrete.clone());
            }
            if let Some(decl) = generic_interfaces.interfaces.get(ref_name) {
                return resolve_generic_interface(
                    ref_name,
                    decl,
                    ty_ref,
                    Some(substitution),
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
            }
            if let Some(decl) = generic_interfaces.aliases.get(ref_name) {
                return resolve_generic_alias(
                    ref_name,
                    decl,
                    ty_ref,
                    Some(substitution),
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
            }
            if ref_name == "Pick" || ref_name == "Omit" {
                let [object, keys] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return DtsType::Unsupported(format!(
                        "{ref_name}<T, K> requires exactly two type arguments"
                    ));
                };
                let object = resolve_ts_type_with_substitution(
                    object,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
                let keys = substituted_utility_keys(
                    keys,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
                return apply_pick_or_omit(object, keys, ref_name == "Omit");
            }
            if ref_name == "Record" {
                let [keys, value] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return DtsType::Unsupported(
                        "Record<K, V> requires exactly two type arguments".into(),
                    );
                };
                let value = resolve_ts_type_with_substitution(
                    value,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
                if matches!(
                    keys.as_ref(),
                    TsType::TsKeywordType(keyword)
                        if keyword.kind == TsKeywordTypeKind::TsStringKeyword
                ) {
                    return match value {
                        DtsType::Native(value) => {
                            DtsType::Native(HirType::Dictionary(Box::new(value)))
                        }
                        unsupported => unsupported,
                    };
                }
                let keys = substituted_utility_keys(
                    keys,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
                return apply_record(value, keys);
            }
            if let Some(params) = &ty_ref.type_params {
                if let [elem] = params.params.as_slice() {
                    let resolved_elem = resolve_ts_type_with_substitution(
                        elem,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    );
                    match (ref_name, resolved_elem) {
                        ("Readonly", resolved) => return resolved,
                        ("Partial", resolved) => {
                            return apply_partial_or_required(resolved, false)
                        }
                        ("Required", resolved) => {
                            return apply_partial_or_required(resolved, true)
                        }
                        ("NonNullable", resolved) => return strip_non_nullable(resolved),
                        ("Array" | "ReadonlyArray", DtsType::Native(element))
                            if supports_native_array_element(&element) => {
                            return DtsType::Native(HirType::Array(Box::new(element)))
                        }
                        ("Array" | "ReadonlyArray", DtsType::Native(other)) => {
                            return DtsType::Unsupported(format!(
                                "array element type {other:?} does not have a native collection layout"
                            ))
                        }
                        ("Array" | "ReadonlyArray", DtsType::Unsupported(reason)) => {
                            return DtsType::Unsupported(format!("array element type: {reason}"))
                        }
                        _ => {}
                    }
                }
            }
        }
        return classify_ts_type(ty, interfaces, generic_interfaces);
    }

    match ty {
        TsType::TsParenthesizedType(parenthesized) => resolve_ts_type_with_substitution(
            &parenthesized.type_ann,
            substitution,
            interfaces,
            generic_interfaces,
            in_progress,
        ),
        TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::ReadOnly => {
            resolve_ts_type_with_substitution(
                &operator.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )
        }
        TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::KeyOf => {
            match resolve_ts_type_with_substitution(
                &operator.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ) {
                DtsType::Native(HirType::Object(_) | HirType::Dictionary(_)) => {
                    DtsType::Native(HirType::Str)
                }
                DtsType::Native(other) => DtsType::Unsupported(format!(
                    "keyof {other:?} does not have a string-only native ABI"
                )),
                unsupported => unsupported,
            }
        }
        TsType::TsConditionalType(conditional) => {
            let check = resolve_ts_type_with_substitution(
                &conditional.check_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            );
            let extends = resolve_ts_type_with_substitution(
                &conditional.extends_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            );
            match (check, extends) {
                (DtsType::Native(check), DtsType::Native(extends)) => {
                    resolve_ts_type_with_substitution(
                        if bridge_type_satisfies_constraint(&check, &extends) {
                            &conditional.true_type
                        } else {
                            &conditional.false_type
                        },
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )
                }
                (DtsType::Unsupported(reason), _) | (_, DtsType::Unsupported(reason)) => {
                    DtsType::Unsupported(format!("conditional type test: {reason}"))
                }
            }
        }
        TsType::TsIndexedAccessType(indexed) => classify_indexed_access(
            resolve_ts_type_with_substitution(
                &indexed.obj_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ),
            &indexed.index_type,
        ),
        TsType::TsMappedType(mapped) => classify_mapped_type(
            mapped,
            Some(substitution),
            interfaces,
            generic_interfaces,
            in_progress,
        ),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            classify_native_union(union, |element| {
                resolve_ts_type_with_substitution(
                    element,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )
            })
        }
        TsType::TsArrayType(arr) => {
            match resolve_ts_type_with_substitution(
                &arr.elem_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ) {
                DtsType::Native(element) if supports_native_array_element(&element) => {
                    DtsType::Native(HirType::Array(Box::new(element)))
                }
                DtsType::Native(other) => DtsType::Unsupported(format!(
                    "array element type {other:?} does not have a native collection layout"
                )),
                DtsType::Unsupported(reason) => {
                    DtsType::Unsupported(format!("array element type: {reason}"))
                }
            }
        }
        TsType::TsTupleType(tuple) => {
            let elements = tuple
                .elem_types
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    match resolve_ts_type_with_substitution(
                        &element.ty,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    ) {
                        DtsType::Native(element) => Ok(element),
                        DtsType::Unsupported(reason) => {
                            Err(format!("tuple element {index}: {reason}"))
                        }
                    }
                })
                .collect::<Result<Vec<_>, _>>();
            match elements {
                Ok(elements) => DtsType::Native(HirType::Tuple(elements)),
                Err(reason) => DtsType::Unsupported(reason),
            }
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            if function.type_params.is_some() {
                return DtsType::Unsupported("generic callback types are not supported".into());
            }
            let mut params = Vec::with_capacity(function.params.len());
            let mut optional = Vec::with_capacity(function.params.len());
            let mut rest = None;
            for (index, parameter) in function.params.iter().enumerate() {
                if matches!(parameter, TsFnParam::Ident(parameter) if parameter.id.sym == "this") {
                    continue;
                }
                let (annotation, is_optional) = match parameter {
                    TsFnParam::Ident(parameter) => {
                        let Some(annotation) = &parameter.type_ann else {
                            return DtsType::Unsupported(format!(
                                "callback parameter `{}` has no type annotation",
                                parameter.id.sym
                            ));
                        };
                        (annotation.type_ann.as_ref(), parameter.id.optional)
                    }
                    TsFnParam::Rest(parameter) if index + 1 == function.params.len() => {
                        let Some(annotation) = &parameter.type_ann else {
                            return DtsType::Unsupported(
                                "callback rest parameter has no type annotation".into(),
                            );
                        };
                        let TsType::TsArrayType(array) = annotation.type_ann.as_ref() else {
                            return DtsType::Unsupported(
                                "callback rest parameter must use an array type".into(),
                            );
                        };
                        match resolve_ts_type_with_substitution(
                            &array.elem_type,
                            substitution,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        ) {
                            DtsType::Native(ty) => rest = Some(ty),
                            DtsType::Unsupported(_) => rest = Some(HirType::Json),
                        }
                        continue;
                    }
                    TsFnParam::Rest(_) => {
                        return DtsType::Unsupported("callback rest parameter must be last".into());
                    }
                    _ => {
                        return DtsType::Unsupported(
                            "callback parameters must be identifiers or a trailing rest parameter"
                                .into(),
                        );
                    }
                };
                let mut ty = match resolve_ts_type_with_substitution(
                    annotation,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                ) {
                    DtsType::Native(ty) => ty,
                    DtsType::Unsupported(_) => HirType::Json,
                };
                if is_optional {
                    ty = optional_hir_type(ty);
                }
                params.push(ty);
                optional.push(is_optional);
            }
            let ret = match resolve_ts_type_with_substitution(
                &function.type_ann.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ) {
                DtsType::Native(ret) => normalize_callback_return(ret),
                DtsType::Unsupported(_) if ts_type_includes_void(&function.type_ann.type_ann) => {
                    HirType::Void
                }
                DtsType::Unsupported(reason) => {
                    return DtsType::Unsupported(format!("callback return type: {reason}"));
                }
            };
            DtsType::Native(if rest.is_some() || optional.iter().any(|value| *value) {
                HirType::CallableFunction(
                    params,
                    HirOptionalMask::from_bools(&optional),
                    rest.map(Box::new),
                    Box::new(ret),
                )
            } else {
                HirType::Function(params, Box::new(ret))
            })
        }
        TsType::TsTypeLit(type_lit) => {
            let mut fields = Vec::with_capacity(type_lit.members.len());
            let mut dictionary = None;
            for member in &type_lit.members {
                if let TsTypeElement::TsIndexSignature(signature) = member {
                    let value = match index_signature_value(signature) {
                        Ok(value) => resolve_ts_type_with_substitution(
                            value,
                            substitution,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        ),
                        Err(reason) => return DtsType::Unsupported(reason),
                    };
                    match value {
                        DtsType::Native(value)
                            if dictionary.as_ref().is_none_or(|existing| existing == &value)
                                && fields.iter().all(|(_, field)| field == &value) =>
                        {
                            dictionary = Some(value);
                        }
                        DtsType::Native(_) => {
                            return DtsType::Unsupported(
                                "object type literal has incompatible dictionary values".into(),
                            )
                        }
                        unsupported => return unsupported,
                    }
                    continue;
                }
                let TsTypeElement::TsPropertySignature(prop) = member else {
                    return DtsType::Unsupported(
                        "object type literal has a non-property member (method/index signature)"
                            .to_string(),
                    );
                };
                let Some(field_name) = type_property_name(&prop.key) else {
                    return DtsType::Unsupported(
                        "unsupported object type literal key".to_string(),
                    );
                };
                let field_ty = match &prop.type_ann {
                    Some(ann) => resolve_ts_type_with_substitution(
                        &ann.type_ann,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    ),
                    None => {
                        DtsType::Unsupported(format!("field `{field_name}` has no type annotation"))
                    }
                };
                match field_ty {
                    DtsType::Native(ty) => {
                        let ty = if prop.optional {
                            optional_hir_type(ty)
                        } else {
                            ty
                        };
                        if dictionary.as_ref().is_some_and(|element| element != &ty) {
                            return DtsType::Unsupported(format!(
                                "object field `{field_name}` does not match its index value type"
                            ));
                        }
                        fields.push((field_name, ty));
                    }
                    DtsType::Unsupported(reason) => {
                        return DtsType::Unsupported(format!(
                            "object field `{field_name}`: {reason}"
                        ))
                    }
                }
            }
            DtsType::Native(dictionary.map_or(HirType::Object(fields), |element| {
                HirType::Dictionary(Box::new(element))
            }))
        }
        other => classify_ts_type(other, interfaces, generic_interfaces),
    }
}
