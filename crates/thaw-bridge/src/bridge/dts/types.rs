/// Renders an arbitrary `.d.ts` type back to a short, TS-like expression,
/// for use in a `Classification::Fallback` `reason`. Validated against a
/// real npm package's `.d.ts` corpus (date-fns): without this, reasons for
/// anything beyond the simplest unsupported types were unreadable `{:?}`
/// dumps of the full AST node (spans, nested `Box`es, etc.) -- e.g.
/// `unsupported type TsUnionOrIntersectionType(TsIntersectionType(TsIntersectionType
/// { span: 1063..1081, types: [...] }))` for a type that's really just
/// `DateArg<Date> & {}`. This never needs to be exhaustive or fully
/// faithful (it's a diagnostic message, not something re-parsed), so
/// unusual type forms fall back to a short generic label instead of
/// recursing further.
fn describe_ts_type(ty: &TsType) -> String {
    match ty {
        TsType::TsKeywordType(kw) => keyword_name(kw.kind).to_string(),
        TsType::TsThisType(_) => "this".to_string(),
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            let params = function
                .params
                .iter()
                .enumerate()
                .map(|(index, parameter)| match parameter {
                    TsFnParam::Ident(parameter) => format!(
                        "{}{}: {}",
                        parameter.id.sym,
                        if parameter.id.optional { "?" } else { "" },
                        parameter
                            .type_ann
                            .as_ref()
                            .map(|annotation| describe_ts_type(&annotation.type_ann))
                            .unwrap_or_else(|| "Json".into())
                    ),
                    _ => format!("arg{index}: Json"),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "({params}) => {}",
                describe_ts_type(&function.type_ann.type_ann)
            )
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsConstructorType(_)) => {
            "a constructor type".to_string()
        }
        TsType::TsTypeRef(ty_ref) => {
            let name = match &ty_ref.type_name {
                TsEntityName::Ident(id) => id.sym.to_string(),
                TsEntityName::TsQualifiedName(_) => "<qualified name>".to_string(),
            };
            match &ty_ref.type_params {
                Some(params) => {
                    let args = params
                        .params
                        .iter()
                        .map(|p| describe_ts_type(p))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{name}<{args}>")
                }
                None => name,
            }
        }
        TsType::TsTypeQuery(_) => "a `typeof` query type".to_string(),
        TsType::TsTypeLit(lit) if lit.members.is_empty() => "{}".to_string(),
        TsType::TsTypeLit(_) => "{ ... }".to_string(),
        TsType::TsArrayType(arr) => format!("{}[]", describe_ts_type(&arr.elem_type)),
        TsType::TsTupleType(_) => "a tuple type".to_string(),
        TsType::TsOptionalType(opt) => format!("{}?", describe_ts_type(&opt.type_ann)),
        TsType::TsRestType(rest) => format!("...{}", describe_ts_type(&rest.type_ann)),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(u)) => u
            .types
            .iter()
            .map(|t| describe_ts_type(t))
            .collect::<Vec<_>>()
            .join(" | "),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(i)) => i
            .types
            .iter()
            .map(|t| describe_ts_type(t))
            .collect::<Vec<_>>()
            .join(" & "),
        TsType::TsConditionalType(_) => "a conditional type".to_string(),
        TsType::TsInferType(_) => "an `infer` type".to_string(),
        TsType::TsParenthesizedType(p) => format!("({})", describe_ts_type(&p.type_ann)),
        TsType::TsTypeOperator(op) => {
            let op_name = match op.op {
                TsTypeOperatorOp::KeyOf => "keyof",
                TsTypeOperatorOp::Unique => "unique",
                TsTypeOperatorOp::ReadOnly => "readonly",
            };
            format!("{op_name} {}", describe_ts_type(&op.type_ann))
        }
        TsType::TsIndexedAccessType(indexed) => format!(
            "{}[{}]",
            describe_ts_type(&indexed.obj_type),
            describe_ts_type(&indexed.index_type)
        ),
        TsType::TsMappedType(_) => "a mapped type".to_string(),
        TsType::TsLitType(lit) => match &lit.lit {
            // `Wtf8Atom` has no `Display`; its `Debug` already renders as
            // a quoted string, which is exactly what's wanted here.
            TsLit::Str(s) => format!("{:?}", s.value),
            TsLit::Bool(b) => b.value.to_string(),
            TsLit::Number(n) => n.value.to_string(),
            _ => "a literal type".to_string(),
        },
        TsType::TsTypePredicate(_) => "a type predicate".to_string(),
        TsType::TsImportType(_) => "an `import()` type".to_string(),
    }
}

fn describe_generic_parameter_type(ty: &TsType, generic: &GenericInterfaces<'_>) -> String {
    let TsType::TsTypeRef(reference) = ty else {
        return describe_ts_type(ty);
    };
    let TsEntityName::Ident(name) = &reference.type_name else {
        return describe_ts_type(ty);
    };
    let Some(alias) = generic.aliases.get(name.sym.as_str()) else {
        return describe_ts_type(ty);
    };
    let Some(parameters) = &alias.type_params else {
        return describe_ts_type(ty);
    };
    let Some(arguments) = &reference.type_params else {
        return describe_ts_type(ty);
    };
    if parameters.params.len() != arguments.params.len() {
        return describe_ts_type(ty);
    }
    let substitutions = parameters
        .params
        .iter()
        .zip(&arguments.params)
        .map(|(parameter, argument)| {
            (
                parameter.name.sym.to_string(),
                describe_ts_type(argument),
            )
        })
        .collect::<HashMap<_, _>>();

    fn render(
        ty: &TsType,
        substitutions: &HashMap<String, String>,
        generic: &GenericInterfaces<'_>,
        depth: u8,
    ) -> String {
        match ty {
            TsType::TsTypeRef(reference) => {
                if let TsEntityName::Ident(name) = &reference.type_name {
                    if reference.type_params.is_none() {
                        if let Some(substitution) = substitutions.get(name.sym.as_str()) {
                            return substitution.clone();
                        }
                    }
                    if depth < 8 {
                        if let (Some(alias), Some(arguments)) = (
                            generic.aliases.get(name.sym.as_str()),
                            &reference.type_params,
                        ) {
                            if let Some(parameters) = &alias.type_params {
                                if parameters.params.len() == arguments.params.len() {
                                    let nested = parameters
                                        .params
                                        .iter()
                                        .zip(&arguments.params)
                                        .map(|(parameter, argument)| {
                                            (
                                                parameter.name.sym.to_string(),
                                                render(argument, substitutions, generic, depth + 1),
                                            )
                                        })
                                        .collect();
                                    return render(&alias.type_ann, &nested, generic, depth + 1);
                                }
                            }
                        }
                    }
                }
                describe_ts_type(ty)
            }
            TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
                let params = function
                    .params
                    .iter()
                    .enumerate()
                    .map(|(index, parameter)| match parameter {
                        TsFnParam::Ident(parameter) => format!(
                            "{}{}: {}",
                            parameter.id.sym,
                            if parameter.id.optional { "?" } else { "" },
                            parameter
                                .type_ann
                                .as_ref()
                                .map(|annotation| {
                                    render(&annotation.type_ann, substitutions, generic, depth)
                                })
                                .unwrap_or_else(|| "Json".into())
                        ),
                        _ => format!("arg{index}: Json"),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "({params}) => {}",
                    render(&function.type_ann.type_ann, substitutions, generic, depth)
                )
            }
            TsType::TsArrayType(array) => {
                format!("{}[]", render(&array.elem_type, substitutions, generic, depth))
            }
            TsType::TsIndexedAccessType(indexed) => format!(
                "{}[{}]",
                render(&indexed.obj_type, substitutions, generic, depth),
                render(&indexed.index_type, substitutions, generic, depth)
            ),
            TsType::TsTypeOperator(operator) => format!(
                "{} {}",
                match operator.op {
                    TsTypeOperatorOp::KeyOf => "keyof",
                    TsTypeOperatorOp::Unique => "unique",
                    TsTypeOperatorOp::ReadOnly => "readonly",
                },
                render(&operator.type_ann, substitutions, generic, depth)
            ),
            TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) =>
                union.types.iter().map(|ty| render(ty, substitutions, generic, depth)).collect::<Vec<_>>().join(" | "),
            TsType::TsParenthesizedType(parenthesized) => {
                format!("({})", render(&parenthesized.type_ann, substitutions, generic, depth))
            }
            _ => describe_ts_type(ty),
        }
    }

    render(&alias.type_ann, &substitutions, generic, 0)
}

fn rest_element_type(ty: &TsType) -> &TsType {
    match ty {
        TsType::TsArrayType(array) => &array.elem_type,
        TsType::TsTypeRef(reference)
            if matches!(&reference.type_name, TsEntityName::Ident(name) if name.sym == *"Array") =>
        {
            reference
                .type_params
                .as_ref()
                .and_then(|parameters| parameters.params.first())
                .map_or(ty, |element| element)
        }
        _ => ty,
    }
}

fn describe_contextual_rest_type(ty: &TsType, generic: &GenericInterfaces<'_>) -> String {
    let mut current = ty;
    for _ in 0..8 {
        let described = describe_generic_parameter_type(current, generic);
        if described.starts_with('(') {
            return described;
        }
        let TsType::TsTypeRef(reference) = current else {
            return described;
        };
        if let TsEntityName::Ident(name) = &reference.type_name {
            if let Some(alias) = generic.aliases.get(name.sym.as_str()) {
                if let TsType::TsUnionOrIntersectionType(
                    TsUnionOrIntersectionType::TsUnionType(union),
                ) = alias.type_ann.as_ref()
                {
                    if let Some(first) = union.types.first() {
                        let forwards_parameter = alias
                            .type_params
                            .as_ref()
                            .and_then(|parameters| parameters.params.first())
                            .is_some_and(|parameter| {
                                matches!(first.as_ref(), TsType::TsTypeRef(first_ref)
                                    if matches!(&first_ref.type_name, TsEntityName::Ident(first_name)
                                        if first_ref.type_params.is_none()
                                            && first_name.sym == parameter.name.sym))
                            });
                        if !forwards_parameter {
                            current = first;
                            continue;
                        }
                    }
                }
            }
        }
        let Some(next) = reference
            .type_params
            .as_ref()
            .and_then(|parameters| (parameters.params.len() == 1).then(|| &*parameters.params[0]))
        else {
            return described;
        };
        current = next;
    }
    describe_generic_parameter_type(current, generic)
}

/// The TS keyword spelling for a `TsKeywordTypeKind` (`number`/`string`/
/// `boolean`/`void` are handled natively by `classify_ts_type` and never
/// reach here; this covers the rest for `describe_ts_type`/error messages).
fn keyword_name(kind: TsKeywordTypeKind) -> &'static str {
    match kind {
        TsKeywordTypeKind::TsAnyKeyword => "any",
        TsKeywordTypeKind::TsUnknownKeyword => "unknown",
        TsKeywordTypeKind::TsNumberKeyword => "number",
        TsKeywordTypeKind::TsObjectKeyword => "object",
        TsKeywordTypeKind::TsBooleanKeyword => "boolean",
        TsKeywordTypeKind::TsBigIntKeyword => "bigint",
        TsKeywordTypeKind::TsStringKeyword => "string",
        TsKeywordTypeKind::TsSymbolKeyword => "symbol",
        TsKeywordTypeKind::TsVoidKeyword => "void",
        TsKeywordTypeKind::TsUndefinedKeyword => "undefined",
        TsKeywordTypeKind::TsNullKeyword => "null",
        TsKeywordTypeKind::TsNeverKeyword => "never",
        TsKeywordTypeKind::TsIntrinsicKeyword => "intrinsic",
    }
}

fn optional_hir_type(ty: HirType) -> HirType {
    match ty {
        HirType::Optional(_) | HirType::Nullish(_) => ty,
        HirType::Nullable(payload) => HirType::Nullish(payload),
        other => HirType::Optional(Box::new(other)),
    }
}

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

/// Mirrors `thaw_hir::lower::lower_ts_type`'s mapping rules, but never
/// fails: anything it can't map becomes `DtsType::Unsupported` with a
/// reason, for `classify` to report per-parameter/return instead of
/// aborting the whole `.d.ts` file over one unsupported signature.
fn classify_ts_type(
    ty: &TsType,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsType {
    match ty {
        // External instances are opaque handles today. Treat a fluent
        // `this` result as discarded so the method stays on its native
        // backend instead of mixing an N-API receiver into QuickJS.
        // ponytail: preserve the receiver once HIR has an external self type.
        TsType::TsThisType(_) => DtsType::Native(HirType::Void),
        TsType::TsParenthesizedType(parenthesized) => {
            classify_ts_type(&parenthesized.type_ann, interfaces, generic_interfaces)
        }
        TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::ReadOnly => {
            classify_ts_type(&operator.type_ann, interfaces, generic_interfaces)
        }
        TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::KeyOf => {
            match classify_ts_type(&operator.type_ann, interfaces, generic_interfaces) {
                DtsType::Native(HirType::Object(_) | HirType::Dictionary(_)) => {
                    DtsType::Native(HirType::Str)
                }
                DtsType::Native(other) => DtsType::Unsupported(format!(
                    "keyof {other:?} does not have a string-only native ABI"
                )),
                unsupported => unsupported,
            }
        }
        TsType::TsKeywordType(kw) => match kw.kind {
            TsKeywordTypeKind::TsNumberKeyword => DtsType::Native(HirType::F64),
            TsKeywordTypeKind::TsStringKeyword => DtsType::Native(HirType::Str),
            TsKeywordTypeKind::TsBooleanKeyword => DtsType::Native(HirType::Bool),
            TsKeywordTypeKind::TsVoidKeyword => DtsType::Native(HirType::Void),
            other => DtsType::Unsupported(format!("`{}` is not supported", keyword_name(other))),
        },

        TsType::TsLitType(literal) => match &literal.lit {
            TsLit::Number(_) => DtsType::Native(HirType::F64),
            TsLit::Str(_) => DtsType::Native(HirType::Str),
            TsLit::Bool(_) => DtsType::Native(HirType::Bool),
            // A template literal type (`` `${number}${Unit}` ``) is,
            // structurally, always a plain string at runtime -- there's
            // no way to validate the actual interpolated pattern
            // without a real type checker, but erasing it to `Str` (the
            // same approach a plain string-literal type already gets)
            // is the same "best-effort classify, don't validate the
            // literal value" choice this whole match already makes.
            // Real example: `ms`'s own `StringValue = \`${number}\` |
            // \`${number}${UnitAnyCase}\` | ...`.
            TsLit::Tpl(_) => DtsType::Native(HirType::Str),
            _ => DtsType::Unsupported("unsupported literal type".into()),
        },

        TsType::TsConditionalType(conditional) => {
            let check = classify_ts_type(&conditional.check_type, interfaces, generic_interfaces);
            let extends =
                classify_ts_type(&conditional.extends_type, interfaces, generic_interfaces);
            match (check, extends) {
                (DtsType::Native(check), DtsType::Native(extends)) => classify_ts_type(
                    if bridge_type_satisfies_constraint(&check, &extends) {
                        &conditional.true_type
                    } else {
                        &conditional.false_type
                    },
                    interfaces,
                    generic_interfaces,
                ),
                (DtsType::Unsupported(reason), _) | (_, DtsType::Unsupported(reason)) => {
                    DtsType::Unsupported(format!("conditional type test: {reason}"))
                }
            }
        }

        TsType::TsIndexedAccessType(indexed) => classify_indexed_access(
            classify_ts_type(&indexed.obj_type, interfaces, generic_interfaces),
            &indexed.index_type,
        ),

        TsType::TsMappedType(mapped) => classify_mapped_type(
            mapped,
            None,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        ),

        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            classify_native_union(union, |element| {
                classify_ts_type(element, interfaces, generic_interfaces)
            })
        }

        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => {
            let mut native = Vec::new();
            for element in &intersection.types {
                match classify_ts_type(element, interfaces, generic_interfaces) {
                    DtsType::Native(ty) => native.push(ty),
                    DtsType::Unsupported(_) => {
                        return DtsType::Unsupported(format!(
                            "unsupported type `{}`",
                            describe_ts_type(ty)
                        ))
                    }
                }
            }
            let Some(first) = native.first() else {
                return DtsType::Unsupported("empty intersection type is not supported".into());
            };
            if native.iter().all(|element| element == first) {
                return DtsType::Native(first.clone());
            }
            if native
                .iter()
                .all(|element| matches!(element, HirType::Object(_)))
            {
                let mut merged = Vec::new();
                for element in native {
                    let HirType::Object(fields) = element else {
                        unreachable!()
                    };
                    for (name, ty) in fields {
                        if let Some((_, existing)) =
                            merged.iter().find(|(existing, _)| existing == &name)
                        {
                            if existing != &ty {
                                return DtsType::Unsupported(format!(
                                    "intersection field `{name}` has conflicting native layouts"
                                ));
                            }
                        } else {
                            merged.push((name, ty));
                        }
                    }
                }
                return DtsType::Native(HirType::Object(merged));
            }
            DtsType::Unsupported(format!("unsupported type `{}`", describe_ts_type(ty)))
        }

        TsType::TsArrayType(arr) => {
            match classify_ts_type(&arr.elem_type, interfaces, generic_interfaces) {
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
                    match classify_ts_type(&element.ty, interfaces, generic_interfaces) {
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
                        (annotation, parameter.id.optional)
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
                        match classify_ts_type(&array.elem_type, interfaces, generic_interfaces) {
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
                match classify_ts_type(&annotation.type_ann, interfaces, generic_interfaces) {
                    DtsType::Native(mut ty) => {
                        if is_optional {
                            ty = match ty {
                                HirType::Optional(_) | HirType::Nullish(_) => ty,
                                HirType::Nullable(payload) => HirType::Nullish(payload),
                                other => HirType::Optional(Box::new(other)),
                            };
                        }
                        params.push(ty);
                    }
                    // Callback values cross the JavaScript/N-API boundary as
                    // dynamic JSON. In real Node declarations the error slot
                    // is normally `Error | null` and result slots are often
                    // `any`; neither has a native AOT layout, but both have a
                    // faithful dynamic representation at this boundary.
                    DtsType::Unsupported(_) => params.push(HirType::Json),
                }
                optional.push(is_optional);
            }
            let ret = match classify_ts_type(
                &function.type_ann.type_ann,
                interfaces,
                generic_interfaces,
            ) {
                DtsType::Native(ret) => ret,
                DtsType::Unsupported(_) if ts_type_includes_void(&function.type_ann.type_ann) => {
                    HirType::Void
                }
                DtsType::Unsupported(reason) => {
                    return DtsType::Unsupported(format!("callback return type: {reason}"));
                }
            };
            DtsType::Native(if rest.is_some() || optional.iter().any(|value| *value) {
                let optional = HirOptionalMask::from_bools(&optional);
                HirType::CallableFunction(params, optional, rest.map(Box::new), Box::new(ret))
            } else {
                HirType::Function(params, Box::new(ret))
            })
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsConstructorType(_)) => {
            DtsType::Unsupported("constructor callback types are not supported".into())
        }

        TsType::TsTypeLit(type_lit) => {
            let mut fields = Vec::with_capacity(type_lit.members.len());
            let mut dictionary = None;
            for member in &type_lit.members {
                if let TsTypeElement::TsIndexSignature(signature) = member {
                    let value = match index_signature_value(signature) {
                        Ok(value) => {
                            classify_ts_type(value, interfaces, generic_interfaces)
                        }
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
                    Some(ann) => classify_ts_type(&ann.type_ann, interfaces, generic_interfaces),
                    None => {
                        DtsType::Unsupported(format!("field `{field_name}` has no type annotation"))
                    }
                };
                match field_ty {
                    // See the parallel comment in `resolve_interface`:
                    // any representable type works as a field now.
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

        TsType::TsTypeRef(ty_ref) => {
            // A qualified reference (`ms.StringValue`) is tracked by its
            // own bare rightmost identifier only, matching how every
            // interface/alias/class/function here already is regardless
            // of which namespace declares it (see
            // `export_assignment_interface_name`'s doc comment, and
            // `extract_interface_decls`/`extract_type_alias_decls`,
            // which populate `interfaces`/`generic_interfaces` this same
            // namespace-agnostic way) -- real example: `ms`'s own
            // `declare namespace ms { type StringValue = ...; }`,
            // referenced from its sibling overload as `ms.StringValue`.
            // Falls through to the ordinary "not classified" case below
            // exactly as before when no such name is known at all.
            let ref_name = match &ty_ref.type_name {
                TsEntityName::Ident(id) => id.sym.to_string(),
                TsEntityName::TsQualifiedName(qualified) => qualified.right.sym.to_string(),
            };

            // A name matching a resolved (non-generic) `interface` --
            // treated exactly like an inline `{ ... }` type literal.
            if let Some(resolved) = interfaces.get(&ref_name) {
                return resolved.clone();
            }
            // A generic interface, referenced with concrete type
            // arguments -- resolved on demand via substitution.
            if let Some(decl) = generic_interfaces.interfaces.get(&ref_name) {
                return resolve_generic_interface(
                    &ref_name,
                    decl,
                    ty_ref,
                    None,
                    interfaces,
                    generic_interfaces,
                    &mut Vec::new(),
                );
            }
            if let Some(decl) = generic_interfaces.aliases.get(&ref_name) {
                return resolve_generic_alias(
                    &ref_name,
                    decl,
                    ty_ref,
                    None,
                    interfaces,
                    generic_interfaces,
                    &mut Vec::new(),
                );
            }
            if ref_name == "Json" && ty_ref.type_params.is_none() {
                return DtsType::Native(HirType::Json);
            }
            if ref_name == "JsValue" && ty_ref.type_params.is_none() {
                return DtsType::Native(HirType::JsValue);
            }
            // Mirrors `thaw_hir::lower::type_resolution`'s own `Date`
            // handling exactly: a fixed native object with a single
            // millisecond-since-epoch `timestamp` field, not a new value
            // representation. This lets a `.d.ts` `Date` parameter/return
            // (and, via constraint substitution above, a `<T extends
            // Date>` type parameter) classify as `Native` instead of
            // `Unsupported`, so ordinary Object JSON marshaling applies --
            // QuickJS-side `Date.prototype.toJSON`/the `JSON.parse`
            // reviver (thaw-quickjs) convert to/from a real JS `Date` at
            // the call boundary using this same `{"timestamp": ...}` shape.
            if ref_name == "Date" && ty_ref.type_params.is_none() {
                return DtsType::Native(HirType::Object(vec![(
                    "timestamp".to_string(),
                    HirType::F64,
                )]));
            }
            if ref_name == "Readonly" {
                let Some(inner) = ty_ref
                    .type_params
                    .as_ref()
                    .and_then(|params| params.params.first())
                else {
                    return DtsType::Unsupported("Readonly<T> needs one type argument".to_string());
                };
                return classify_ts_type(inner, interfaces, generic_interfaces);
            }
            if matches!(ref_name.as_str(), "Partial" | "Required") {
                let [inner] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return DtsType::Unsupported(format!(
                        "{ref_name}<T> requires exactly one type argument"
                    ));
                };
                return apply_partial_or_required(
                    classify_ts_type(inner, interfaces, generic_interfaces),
                    ref_name == "Required",
                );
            }
            if ref_name == "NonNullable" {
                let [inner] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return DtsType::Unsupported(
                        "NonNullable<T> requires exactly one type argument".into(),
                    );
                };
                return classify_non_nullable_type(inner, interfaces, generic_interfaces);
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
                let value = classify_ts_type(value, interfaces, generic_interfaces);
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
                return apply_record(value, utility_keys(keys, interfaces, generic_interfaces));
            }
            if matches!(ref_name.as_str(), "Pick" | "Omit") {
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
                return apply_pick_or_omit(
                    classify_ts_type(object, interfaces, generic_interfaces),
                    utility_keys(keys, interfaces, generic_interfaces),
                    ref_name == "Omit",
                );
            }
            if matches!(ref_name.as_str(), "Array" | "ReadonlyArray") {
                let Some(element) = ty_ref
                    .type_params
                    .as_ref()
                    .and_then(|params| params.params.first())
                else {
                    return DtsType::Unsupported(format!(
                        "{ref_name}<T> needs one type argument"
                    ));
                };
                return match classify_ts_type(element, interfaces, generic_interfaces) {
                    DtsType::Native(element) if supports_native_array_element(&element) => {
                        DtsType::Native(HirType::Array(Box::new(element)))
                    }
                    _ => DtsType::Unsupported("array element has no native collection layout".into()),
                };
            }

            // Note: no `Promise<T>` recognition here (unlike
            // thaw-hir's `lower_ts_type`) -- a `.d.ts` signature using
            // either still needs a real decision about arena lifetime
            // (arrays) or the async ABI (promises) across a *foreign* FFI
            // boundary that section 5 of the design doc explicitly defers.
            DtsType::Unsupported(format!(
                "type reference `{ref_name}` is not classified yet (Array<T>/Promise<T>)"
            ))
        }

        other => DtsType::Unsupported(format!("unsupported type `{}`", describe_ts_type(other))),
    }
}

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
                DtsType::Native(ret) => ret,
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
