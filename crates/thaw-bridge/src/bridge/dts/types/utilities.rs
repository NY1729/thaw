fn supports_native_array_element(ty: &HirType) -> bool {
    match ty {
        HirType::F64 | HirType::Str | HirType::Bool | HirType::Json | HirType::JsValue => true,
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            supports_native_array_element(payload)
        }
        HirType::Array(element) => supports_native_array_element(element),
        HirType::Tuple(elements) => elements.iter().all(supports_native_array_element),
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| supports_native_array_element(field)),
        _ => false,
    }
}

fn classify_indexed_access(object: DtsType, index: &TsType) -> DtsType {
    let index = match index {
        TsType::TsParenthesizedType(parenthesized) => parenthesized.type_ann.as_ref(),
        other => other,
    };
    let key = match index {
        TsType::TsLitType(literal) => match &literal.lit {
            TsLit::Str(value) => value.value.to_string_lossy().into_owned(),
            TsLit::Number(value) => value.value.to_string(),
            _ => {
                return DtsType::Unsupported(
                    "indexed access requires a string or number literal key".into(),
                )
            }
        },
        _ => {
            return DtsType::Unsupported(
                "indexed access requires one statically known property key".into(),
            )
        }
    };

    select_object_key(object, &key)
}

fn select_object_key(object: DtsType, key: &str) -> DtsType {
    match object {
        DtsType::Native(HirType::Object(fields)) => fields
            .into_iter()
            .find_map(|(name, ty)| (name == key).then_some(DtsType::Native(ty)))
            .unwrap_or_else(|| {
                DtsType::Unsupported(format!(
                    "indexed access key `{key}` does not exist on the object type"
                ))
            }),
        DtsType::Native(HirType::Dictionary(element)) => DtsType::Native(*element),
        DtsType::Native(other) => DtsType::Unsupported(format!(
            "indexed access requires an object type, found {other:?}"
        )),
        unsupported => unsupported,
    }
}

fn finite_utility_keys(ty: &TsType, keys: &mut Vec<String>) -> Result<(), String> {
    match ty {
        TsType::TsParenthesizedType(parenthesized) => {
            finite_utility_keys(&parenthesized.type_ann, keys)
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            for ty in &union.types {
                finite_utility_keys(ty, keys)?;
            }
            Ok(())
        }
        TsType::TsLitType(literal) => {
            let key = match &literal.lit {
                TsLit::Str(value) => value.value.to_string_lossy().into_owned(),
                TsLit::Number(value) => value.value.to_string(),
                _ => return Err("utility keys must be string or number literals".into()),
            };
            if !keys.contains(&key) {
                keys.push(key);
            }
            Ok(())
        }
        _ => Err("utility keys must be a finite literal union".into()),
    }
}

fn utility_keys(
    ty: &TsType,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Vec<String>, String> {
    if let TsType::TsTypeOperator(operator) = ty {
        if operator.op == TsTypeOperatorOp::KeyOf {
            return match classify_ts_type(&operator.type_ann, interfaces, generic_interfaces) {
                DtsType::Native(HirType::Object(fields)) => {
                    Ok(fields.into_iter().map(|(name, _)| name).collect())
                }
                DtsType::Native(_) => Err("keyof utility keys require an object type".into()),
                DtsType::Unsupported(reason) => Err(reason),
            };
        }
    }
    let mut keys = Vec::new();
    finite_utility_keys(ty, &mut keys)?;
    Ok(keys)
}

fn substituted_utility_keys(
    ty: &TsType,
    substitution: &HashMap<String, HirType>,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> Result<Vec<String>, String> {
    if let TsType::TsTypeOperator(operator) = ty {
        if operator.op == TsTypeOperatorOp::KeyOf {
            return match resolve_ts_type_with_substitution(
                &operator.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ) {
                DtsType::Native(HirType::Object(fields)) => {
                    Ok(fields.into_iter().map(|(name, _)| name).collect())
                }
                DtsType::Native(_) => Err("keyof utility keys require an object type".into()),
                DtsType::Unsupported(reason) => Err(reason),
            };
        }
    }
    let mut keys = Vec::new();
    finite_utility_keys(ty, &mut keys)?;
    Ok(keys)
}

fn apply_pick_or_omit(object: DtsType, keys: Result<Vec<String>, String>, omit: bool) -> DtsType {
    let keys = match keys {
        Ok(keys) => keys,
        Err(reason) => return DtsType::Unsupported(reason),
    };
    let DtsType::Native(HirType::Object(fields)) = object else {
        return DtsType::Unsupported("Pick/Omit require a fixed object type".into());
    };
    if let Some(key) = keys
        .iter()
        .find(|key| !fields.iter().any(|(name, _)| name == *key))
    {
        return DtsType::Unsupported(format!(
            "Pick/Omit key `{key}` does not exist on the object type"
        ));
    }
    DtsType::Native(HirType::Object(
        fields
            .into_iter()
            .filter(|(name, _)| keys.contains(name) != omit)
            .collect(),
    ))
}

fn apply_partial_or_required(object: DtsType, required: bool) -> DtsType {
    match object {
        DtsType::Native(HirType::Object(fields)) => DtsType::Native(HirType::Object(
            fields
                .into_iter()
                .map(|(name, ty)| {
                    let ty = if required {
                        match ty {
                            HirType::Optional(value) => *value,
                            HirType::Nullish(value) => HirType::Nullable(value),
                            other => other,
                        }
                    } else {
                        optional_hir_type(ty)
                    };
                    (name, ty)
                })
                .collect(),
        )),
        DtsType::Native(_) => DtsType::Unsupported(format!(
            "{}<T> requires a fixed object type",
            if required { "Required" } else { "Partial" }
        )),
        unsupported => unsupported,
    }
}

fn apply_record(value: DtsType, keys: Result<Vec<String>, String>) -> DtsType {
    let value = match value {
        DtsType::Native(value) => value,
        unsupported => return unsupported,
    };
    match keys {
        Ok(keys) => DtsType::Native(HirType::Object(
            keys.into_iter().map(|key| (key, value.clone())).collect(),
        )),
        Err(reason) => DtsType::Unsupported(reason),
    }
}

fn strip_non_nullable(ty: DtsType) -> DtsType {
    match ty {
        DtsType::Native(HirType::Optional(value))
        | DtsType::Native(HirType::Nullable(value))
        | DtsType::Native(HirType::Nullish(value)) => DtsType::Native(*value),
        other => other,
    }
}

fn classify_non_nullable_type(
    ty: &TsType,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsType {
    let TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) = ty
    else {
        return strip_non_nullable(classify_ts_type(
            ty,
            interfaces,
            generic_interfaces,
        ));
    };
    let mut native = None;
    for element in &union.types {
        if matches!(
            element.as_ref(),
            TsType::TsKeywordType(keyword)
                if matches!(
                    keyword.kind,
                    TsKeywordTypeKind::TsNullKeyword
                        | TsKeywordTypeKind::TsUndefinedKeyword
                )
        ) {
            continue;
        }
        match classify_ts_type(element, interfaces, generic_interfaces) {
            DtsType::Native(ty) if native.as_ref().is_none_or(|current| current == &ty) => {
                native = Some(ty)
            }
            DtsType::Native(_) => {
                return DtsType::Unsupported(
                    "NonNullable<T> has multiple incompatible native layouts".into(),
                )
            }
            unsupported => return unsupported,
        }
    }
    native
        .map(DtsType::Native)
        .unwrap_or_else(|| DtsType::Unsupported("NonNullable<T> has no native value".into()))
}

fn ts_type_includes_void(ty: &TsType) -> bool {
    match ty {
        TsType::TsKeywordType(keyword) => keyword.kind == TsKeywordTypeKind::TsVoidKeyword,
        TsType::TsParenthesizedType(parenthesized) => {
            ts_type_includes_void(&parenthesized.type_ann)
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            union.types.iter().any(|ty| ts_type_includes_void(ty))
        }
        _ => false,
    }
}

fn normalize_callback_return(ty: HirType) -> HirType {
    match ty {
        HirType::Union(members)
            if !members.is_empty()
                && members.iter().all(|member| {
                matches!(member, HirType::Void)
                    || matches!(member, HirType::Promise(value) if **value == HirType::Void)
                }) =>
        {
            HirType::Void
        }
        other => other,
    }
}

fn classify_native_union(
    union: &swc_ecma_ast::TsUnionType,
    mut classify: impl FnMut(&TsType) -> DtsType,
) -> DtsType {
    let mut native = Vec::new();
    let mut has_null = false;
    let mut has_undefined = false;
    for element in &union.types {
        match element.as_ref() {
            TsType::TsKeywordType(keyword) if keyword.kind == TsKeywordTypeKind::TsNullKeyword => {
                has_null = true;
            }
            TsType::TsKeywordType(keyword)
                if keyword.kind == TsKeywordTypeKind::TsUndefinedKeyword =>
            {
                has_undefined = true;
            }
            other => match classify(other) {
                DtsType::Native(HirType::Union(elements)) => {
                    for ty in elements {
                        if !native.contains(&ty) {
                            native.push(ty);
                        }
                    }
                }
                DtsType::Native(ty) if !native.contains(&ty) => native.push(ty),
                DtsType::Native(_) => {}
                unsupported => return unsupported,
            },
        }
    }
    if native.is_empty() {
        return DtsType::Unsupported("union has no native value type".into());
    }
    let payload = if native.len() == 1 {
        native.pop().unwrap()
    } else {
        HirType::Union(native)
    };
    let tagged = match (payload, has_null, has_undefined) {
        (HirType::Nullish(inner), _, _) => HirType::Nullish(inner),
        (HirType::Optional(inner), true, _) | (HirType::Nullable(inner), _, true) => {
            HirType::Nullish(inner)
        }
        (payload @ HirType::Optional(_), false, _)
        | (payload @ HirType::Nullable(_), _, false) => payload,
        (payload, true, true) => HirType::Nullish(Box::new(payload)),
        (payload, true, false) => HirType::Nullable(Box::new(payload)),
        (payload, false, true) => HirType::Optional(Box::new(payload)),
        (payload, false, false) => payload,
    };
    DtsType::Native(tagged)
}

fn remap_mapped_key(name_type: &TsType, parameter: &str, key: &str) -> Result<String, String> {
    match name_type {
        TsType::TsParenthesizedType(parenthesized) => {
            remap_mapped_key(&parenthesized.type_ann, parameter, key)
        }
        TsType::TsTypeRef(reference)
            if matches!(
                &reference.type_name,
                TsEntityName::Ident(name) if name.sym == parameter
            ) =>
        {
            Ok(key.to_string())
        }
        TsType::TsTypeRef(reference) => {
            let TsEntityName::Ident(name) = &reference.type_name else {
                return Err("qualified mapped key transforms are not supported".into());
            };
            let [argument] = reference
                .type_params
                .as_ref()
                .map(|params| params.params.as_slice())
                .unwrap_or_default()
            else {
                return Err("mapped key transform requires one type argument".into());
            };
            let value = remap_mapped_key(argument, parameter, key)?;
            match name.sym.as_str() {
                "Uppercase" => Ok(value.to_uppercase()),
                "Lowercase" => Ok(value.to_lowercase()),
                "Capitalize" => {
                    let mut chars = value.chars();
                    Ok(chars
                        .next()
                        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default())
                }
                "Uncapitalize" => {
                    let mut chars = value.chars();
                    Ok(chars
                        .next()
                        .map(|first| first.to_lowercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default())
                }
                _ => Err(format!("mapped key transform `{}` is not supported", name.sym)),
            }
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => {
            let dynamic = intersection
                .types
                .iter()
                .filter(|ty| {
                    !matches!(
                        ty.as_ref(),
                        TsType::TsKeywordType(keyword)
                            if keyword.kind == TsKeywordTypeKind::TsStringKeyword
                    )
                })
                .collect::<Vec<_>>();
            match dynamic.as_slice() {
                [value] => remap_mapped_key(value, parameter, key),
                _ => Err("mapped key intersection is not statically resolvable".into()),
            }
        }
        TsType::TsLitType(literal) => match &literal.lit {
            TsLit::Str(value) => Ok(value.value.to_string_lossy().into_owned()),
            TsLit::Tpl(template) => {
                if template.quasis.len() != template.types.len() + 1 {
                    return Err("invalid mapped template literal key".into());
                }
                let mut rendered = String::new();
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let text = quasi
                        .cooked
                        .as_ref()
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_else(|| quasi.raw.to_string());
                    rendered.push_str(&text);
                    if let Some(interpolation) = template.types.get(index) {
                        rendered.push_str(&remap_mapped_key(interpolation, parameter, key)?);
                    }
                }
                Ok(rendered)
            }
            _ => Err("mapped key remapping must produce a string key".into()),
        },
        _ => Err("mapped key remapping is not statically resolvable".into()),
    }
}

fn classify_mapped_type(
    mapped: &swc_ecma_ast::TsMappedType,
    outer_substitution: Option<&HashMap<String, HirType>>,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> DtsType {
    let Some(constraint) = &mapped.type_param.constraint else {
        return DtsType::Unsupported("mapped type key needs a finite constraint".into());
    };
    let keys = match outer_substitution {
        Some(substitution) => substituted_utility_keys(
            constraint,
            substitution,
            interfaces,
            generic_interfaces,
            in_progress,
        ),
        None => utility_keys(constraint, interfaces, generic_interfaces),
    };
    let keys = match keys {
        Ok(keys) => keys,
        Err(reason) => return DtsType::Unsupported(reason),
    };
    let Some(value_type) = &mapped.type_ann else {
        return DtsType::Unsupported("mapped type needs a value annotation".into());
    };
    let parameter = mapped.type_param.name.sym.as_str();
    let indexed_object = match value_type.as_ref() {
        TsType::TsIndexedAccessType(indexed)
            if matches!(
                indexed.index_type.as_ref(),
                TsType::TsTypeRef(reference)
                    if matches!(
                        &reference.type_name,
                        TsEntityName::Ident(name) if name.sym == parameter
                    )
            ) => Some(indexed.obj_type.as_ref()),
        _ => None,
    };

    let mut fields = Vec::with_capacity(keys.len());
    for key in keys {
        let value = if let Some(object) = indexed_object {
            let object = match outer_substitution {
                Some(substitution) => resolve_ts_type_with_substitution(
                    object,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                ),
                None => classify_ts_type(object, interfaces, generic_interfaces),
            };
            select_object_key(object, &key)
        } else {
            let mut substitution = outer_substitution.cloned().unwrap_or_default();
            substitution.insert(parameter.to_string(), HirType::Str);
            resolve_ts_type_with_substitution(
                value_type,
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )
        };
        let mut value = match value {
            DtsType::Native(value) => value,
            unsupported => return unsupported,
        };
        match mapped.optional {
            Some(TruePlusMinus::True | TruePlusMinus::Plus) => {
                value = optional_hir_type(value)
            }
            Some(TruePlusMinus::Minus) => {
                value = match value {
                    HirType::Optional(inner) => *inner,
                    HirType::Nullish(inner) => HirType::Nullable(inner),
                    other => other,
                }
            }
            None => {}
        }
        let field_name = match &mapped.name_type {
            Some(name_type) => match remap_mapped_key(name_type, parameter, &key) {
                Ok(name) => name,
                Err(reason) => return DtsType::Unsupported(reason),
            },
            None => key,
        };
        if fields.iter().any(|(name, _)| name == &field_name) {
            return DtsType::Unsupported(format!(
                "mapped key remapping produces duplicate field `{field_name}`"
            ));
        }
        fields.push((field_name, value));
    }
    DtsType::Native(HirType::Object(fields))
}
