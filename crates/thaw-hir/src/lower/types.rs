fn hir_type_contains_dynamic(ty: &HirType) -> bool {
    match ty {
        HirType::Dynamic => true,
        HirType::Promise(inner)
        | HirType::Array(inner)
        | HirType::Optional(inner)
        | HirType::Nullable(inner)
        | HirType::Nullish(inner) => hir_type_contains_dynamic(inner),
        HirType::Tuple(elements) | HirType::Union(elements) => {
            elements.iter().any(hir_type_contains_dynamic)
        }
        HirType::Object(fields) => fields
            .iter()
            .any(|(_, field)| hir_type_contains_dynamic(field)),
        HirType::Function(parameters, result) => {
            parameters.iter().any(hir_type_contains_dynamic) || hir_type_contains_dynamic(result)
        }
        HirType::CallableFunction(parameters, _, rest, result) => {
            parameters.iter().any(hir_type_contains_dynamic)
                || rest.as_deref().is_some_and(hir_type_contains_dynamic)
                || hir_type_contains_dynamic(result)
        }
        _ => false,
    }
}

fn supports_ffi_variadic_element(ty: &HirType) -> bool {
    match ty {
        HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue => true,
        HirType::Array(element) => matches!(
            element.as_ref(),
            HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue
        ),
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            supports_ffi_variadic_element(payload)
        }
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| supports_ffi_variadic_element(field)),
        _ => false,
    }
}

fn validate_generic_function(fn_decl: &FnDecl) -> Result<Vec<Symbol>, String> {
    let Some(type_params) = &fn_decl.function.type_params else {
        return Ok(Vec::new());
    };
    let name = fn_decl.ident.sym.as_str();
    validate_trailing_type_parameter_defaults("generic function", name, type_params)?;
    if type_params.params.is_empty() || fn_decl.function.return_type.is_none() {
        return Err(format!(
            "generic function `{name}` needs type parameters and a return annotation"
        ));
    }
    Ok(type_params
        .params
        .iter()
        .map(|param| param.name.sym.to_string())
        .collect())
}

fn validate_trailing_type_parameter_defaults(
    declaration_kind: &str,
    name: &str,
    parameters: &swc_ecma_ast::TsTypeParamDecl,
) -> Result<(), String> {
    let mut saw_default = false;
    for parameter in &parameters.params {
        if parameter.default.is_some() {
            saw_default = true;
        } else if saw_default {
            return Err(format!(
                "{declaration_kind} `{name}` has required type parameter `{}` after an optional type parameter",
                parameter.name.sym
            ));
        }
    }
    Ok(())
}

fn supports_generic_native_layout(ty: &HirType) -> bool {
    match ty {
        // `JsValue` is as simple and fixed-size a scalar as `I64`/`Bool`
        // (a plain `i64` handle -- see thaw-llvm's `basic_type`), so a
        // generic function specializes for it exactly the same way. Real
        // example: zod's `optional<T extends core.SomeType>(innerType:
        // T): ZodOptional<T>`, called with another Fallback function's
        // own `JsValue`-typed return (`z.string()`'s schema instance) --
        // `T` infers as `JsValue` from that argument.
        //
        // `Json` is deliberately NOT included here, even though it has
        // the identical "single opaque pointer" `basic_type`
        // representation as `Str`/`JsValue` -- confirmed by direct
        // experiment that loosening this to allow it doesn't fix
        // anything, it just trades this clean compile-time error for a
        // silent runtime crash (later, real zod code on the far side of
        // a dynamic call throws once it doesn't recognize a `{}`-shaped
        // JSON snapshot as a real schema, and that exception surfaces as
        // a bare, silent `exit(1)` with no message). The actual bug this
        // was masking lived one layer up, in object-literal field
        // lowering (`lower_object_lit_field_value`): an unannotated
        // method call whose receiver is `JsValue`-typed (real example:
        // `string().optional()`, a schema-builder chain) used to default
        // to the JSON-decoding behavior when used as a field value,
        // discarding the real handle and forcing exactly this rejected
        // `Object([(name, Json)])` shape to begin with. Fixed there
        // instead -- a correctly-lowered program should never actually
        // produce this shape for that case anymore, so this check
        // staying strict is a real safety net, not just an unfixed gap.
        HirType::F64 | HirType::I64 | HirType::Bool | HirType::Str | HirType::JsValue => true,
        HirType::Array(inner) => **inner == HirType::F64,
        HirType::Tuple(elements) => elements.iter().all(supports_generic_native_layout),
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, ty)| supports_generic_native_layout(ty)),
        HirType::Function(params, ret) => {
            params.iter().all(supports_generic_native_layout)
                && supports_generic_native_layout(ret)
        }
        _ => false,
    }
}

fn promise_settled_result_type(value: HirType) -> HirType {
    HirType::Object(vec![
        ("status".into(), HirType::Str),
        ("value".into(), value),
        ("reason".into(), HirType::Str),
    ])
}

fn awaited_hir_type(mut ty: HirType) -> HirType {
    while let HirType::Promise(value) = ty {
        ty = *value;
    }
    ty
}

fn non_nullable_hir_type(ty: HirType) -> Result<HirType, String> {
    match ty {
        HirType::Optional(value) | HirType::Nullable(value) | HirType::Nullish(value) => {
            non_nullable_hir_type(*value)
        }
        HirType::Union(values) => {
            let values = values
                .into_iter()
                .filter(|value| !matches!(value, HirType::Null | HirType::Undefined))
                .collect::<Vec<_>>();
            match values.as_slice() {
                [] => Err(
                    "NonNullable<T> has no native value when T is only null or undefined".into(),
                ),
                [value] => non_nullable_hir_type(value.clone()),
                _ => Ok(HirType::Union(values)),
            }
        }
        HirType::Null | HirType::Undefined => {
            Err("NonNullable<T> has no native value when T is only null or undefined".into())
        }
        other => Ok(other),
    }
}

fn partial_hir_type(ty: HirType) -> Result<HirType, String> {
    let HirType::Object(fields) = ty else {
        return Err("Partial<T> requires an object type".into());
    };
    Ok(HirType::Object(
        fields
            .into_iter()
            .map(|(name, ty)| (name, optional_parameter_type(ty)))
            .collect(),
    ))
}

fn required_hir_type(ty: HirType) -> Result<HirType, String> {
    let HirType::Object(fields) = ty else {
        return Err("Required<T> requires an object type".into());
    };
    Ok(HirType::Object(
        fields
            .into_iter()
            .map(|(name, ty)| {
                let ty = match ty {
                    HirType::Optional(value) => *value,
                    HirType::Nullish(value) => HirType::Nullable(value),
                    other => other,
                };
                (name, ty)
            })
            .collect(),
    ))
}

fn pick_hir_type(ty: HirType, keys: &[Symbol]) -> Result<HirType, String> {
    let HirType::Object(fields) = ty else {
        return Err("Pick<T, K> requires an object type".into());
    };
    let mut picked = Vec::with_capacity(keys.len());
    for key in keys {
        let (_, ty) = fields
            .iter()
            .find(|(name, _)| name == key)
            .ok_or_else(|| format!("Pick<T, K> key `{key}` does not exist on the object type"))?;
        picked.push((key.clone(), ty.clone()));
    }
    Ok(HirType::Object(picked))
}

fn omit_hir_type(ty: HirType, keys: &[Symbol]) -> Result<HirType, String> {
    let HirType::Object(fields) = ty else {
        return Err("Omit<T, K> requires an object type".into());
    };
    Ok(HirType::Object(
        fields
            .into_iter()
            .filter(|(name, _)| !keys.contains(name))
            .collect(),
    ))
}

fn indexed_access_hir_type(ty: HirType, keys: &[Symbol]) -> Result<HirType, String> {
    let HirType::Object(fields) = ty else {
        return Err("indexed access requires an object type".into());
    };
    let mut selected = Vec::with_capacity(keys.len());
    for key in keys {
        let (_, ty) = fields.iter().find(|(name, _)| name == key).ok_or_else(|| {
            format!("indexed access key `{key}` does not exist on the object type")
        })?;
        if !selected.contains(ty) {
            selected.push(ty.clone());
        }
    }
    match selected.as_slice() {
        [ty] => Ok(ty.clone()),
        _ => Ok(HirType::Union(selected)),
    }
}

fn index_signature_value(signature: &swc_ecma_ast::TsIndexSignature) -> Result<&TsType, String> {
    let [TsFnParam::Ident(key)] = signature.params.as_slice() else {
        return Err("index signature requires one identifier key".into());
    };
    let key_type = key
        .type_ann
        .as_ref()
        .ok_or("index signature key needs a type annotation")?;
    if !matches!(
        key_type.type_ann.as_ref(),
        TsType::TsKeywordType(keyword)
            if keyword.kind == TsKeywordTypeKind::TsStringKeyword
    ) {
        return Err("native dictionary index signatures require a string key".into());
    }
    signature
        .type_ann
        .as_ref()
        .map(|annotation| annotation.type_ann.as_ref())
        .ok_or_else(|| "index signature needs a value type annotation".into())
}

fn type_literal_index_signature(
    literal: &swc_ecma_ast::TsTypeLit,
) -> Result<Option<&swc_ecma_ast::TsIndexSignature>, String> {
    let mut signatures = literal.members.iter().filter_map(|member| match member {
        TsTypeElement::TsIndexSignature(signature) => Some(signature),
        _ => None,
    });
    let first = signatures.next();
    if signatures.next().is_some() {
        return Err("native dictionary types support one index signature".into());
    }
    Ok(first)
}

fn is_string_keyword(ty: &TsType) -> bool {
    matches!(
        ty,
        TsType::TsKeywordType(keyword)
            if keyword.kind == TsKeywordTypeKind::TsStringKeyword
    )
}

fn finite_property_keys(ty: &TsType) -> Result<Vec<Symbol>, String> {
    fn collect(ty: &TsType, keys: &mut Vec<Symbol>) -> Result<(), String> {
        match ty {
            TsType::TsParenthesizedType(parenthesized) => collect(&parenthesized.type_ann, keys),
            TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
                for ty in &union.types {
                    collect(ty, keys)?;
                }
                Ok(())
            }
            TsType::TsLitType(literal) => {
                let key = match &literal.lit {
                    swc_ecma_ast::TsLit::Str(value) => value.value.to_string_lossy().into_owned(),
                    swc_ecma_ast::TsLit::Number(value) => value.value.to_string(),
                    _ => {
                        return Err(
                            "utility type keys must be finite string or number literals".into()
                        )
                    }
                };
                if !keys.contains(&key) {
                    keys.push(key);
                }
                Ok(())
            }
            _ => Err("utility type keys must be a finite string or number literal union".into()),
        }
    }

    let mut keys = Vec::new();
    collect(ty, &mut keys)?;
    Ok(keys)
}

fn hir_object_keys(ty: HirType) -> Result<Vec<Symbol>, String> {
    let HirType::Object(fields) = ty else {
        return Err("keyof requires an object type".into());
    };
    Ok(fields.into_iter().map(|(name, _)| name).collect())
}

fn generic_pattern_keys(pattern: &GenericTypePattern) -> Result<Vec<Symbol>, String> {
    match pattern {
        GenericTypePattern::Object(fields) => {
            Ok(fields.iter().map(|(name, _)| name.clone()).collect())
        }
        GenericTypePattern::Concrete(ty) => hir_object_keys(ty.clone()),
        GenericTypePattern::Partial(inner)
        | GenericTypePattern::Required(inner)
        | GenericTypePattern::NonNullable(inner) => generic_pattern_keys(inner),
        GenericTypePattern::Record(keys, _) | GenericTypePattern::Pick(_, keys) => Ok(keys.clone()),
        GenericTypePattern::Omit(inner, omitted) => Ok(generic_pattern_keys(inner)?
            .into_iter()
            .filter(|key| !omitted.contains(key))
            .collect()),
        _ => Err("keyof requires an object type".into()),
    }
}

fn generic_utility_keys(
    ty: &TsType,
    substitutions: &HashMap<Symbol, GenericTypePattern>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<Symbol>,
) -> Result<Vec<Symbol>, String> {
    if let TsType::TsTypeOperator(operator) = ty {
        if operator.op == swc_ecma_ast::TsTypeOperatorOp::KeyOf {
            return generic_pattern_keys(&generic_type_pattern(
                &operator.type_ann,
                substitutions,
                interfaces,
                generic_interfaces,
                in_progress,
            )?);
        }
    }
    finite_property_keys(ty)
}

fn utility_keys(
    ty: &TsType,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Vec<Symbol>, String> {
    if let TsType::TsTypeOperator(operator) = ty {
        if operator.op == swc_ecma_ast::TsTypeOperatorOp::KeyOf {
            return hir_object_keys(lower_ts_type(
                &operator.type_ann,
                interfaces,
                generic_interfaces,
            )?);
        }
    }
    finite_property_keys(ty)
}

fn substituted_utility_keys(
    ty: &TsType,
    substitution: &HashMap<Symbol, HirType>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<Symbol>,
) -> Result<Vec<Symbol>, String> {
    if let TsType::TsTypeOperator(operator) = ty {
        if operator.op == swc_ecma_ast::TsTypeOperatorOp::KeyOf {
            return hir_object_keys(resolve_ts_type_with_substitution(
                &operator.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?);
        }
    }
    finite_property_keys(ty)
}

fn specialized_generic_name(name: &str, types: &[HirType]) -> Symbol {
    fn fingerprint(ty: &HirType) -> String {
        match ty {
            HirType::F64 => "f64".into(),
            HirType::I64 => "i64".into(),
            HirType::Bool => "bool".into(),
            HirType::Str => "str".into(),
            HirType::Json => "json".into(),
            HirType::Array(inner) => format!("array_{}", fingerprint(inner)),
            HirType::Tuple(elements) => format!(
                "tuple_{}",
                elements
                    .iter()
                    .map(fingerprint)
                    .collect::<Vec<_>>()
                    .join("_")
            ),
            HirType::Object(fields) => format!(
                "object_{}",
                fields
                    .iter()
                    .map(|(name, ty)| format!("{name}_{}", fingerprint(ty)))
                    .collect::<Vec<_>>()
                    .join("_")
            ),
            HirType::Map(key, value) => {
                format!("map_{}_{}", fingerprint(key), fingerprint(value))
            }
            HirType::Set(element) => format!("set_{}", fingerprint(element)),
            other => panic!("unsupported generic specialization type: {other:?}"),
        }
    }
    format!(
        "{name}__thaw_{}",
        types.iter().map(fingerprint).collect::<Vec<_>>().join("__")
    )
}

fn generic_pattern_contains_variable(pattern: &GenericTypePattern, variable: &str) -> bool {
    match pattern {
        GenericTypePattern::Variable(name) => name == variable,
        GenericTypePattern::Array(inner)
        | GenericTypePattern::Promise(inner)
        | GenericTypePattern::Optional(inner)
        | GenericTypePattern::Awaited(inner)
        | GenericTypePattern::NonNullable(inner)
        | GenericTypePattern::Partial(inner)
        | GenericTypePattern::Required(inner)
        | GenericTypePattern::Dictionary(inner) => {
            generic_pattern_contains_variable(inner, variable)
        }
        GenericTypePattern::Record(_, value) => generic_pattern_contains_variable(value, variable),
        GenericTypePattern::Pick(inner, _) | GenericTypePattern::Omit(inner, _) => {
            generic_pattern_contains_variable(inner, variable)
        }
        GenericTypePattern::IndexedAccess(inner, _) => {
            generic_pattern_contains_variable(inner, variable)
        }
        GenericTypePattern::Object(fields) => fields
            .iter()
            .any(|(_, field)| generic_pattern_contains_variable(field, variable)),
        GenericTypePattern::Concrete(_) => false,
    }
}

fn instantiate_generic_pattern(
    pattern: &GenericTypePattern,
    substitution: &HashMap<Symbol, HirType>,
) -> Result<HirType, String> {
    match pattern {
        GenericTypePattern::Variable(name) => substitution
            .get(name)
            .cloned()
            .ok_or_else(|| format!("missing concrete type for `{name}`")),
        GenericTypePattern::Concrete(ty) => Ok(ty.clone()),
        GenericTypePattern::Array(inner) => Ok(HirType::Array(Box::new(
            instantiate_generic_pattern(inner, substitution)?,
        ))),
        GenericTypePattern::Promise(inner) => Ok(HirType::Promise(Box::new(
            instantiate_generic_pattern(inner, substitution)?,
        ))),
        GenericTypePattern::Optional(inner) => Ok(optional_parameter_type(
            instantiate_generic_pattern(inner, substitution)?,
        )),
        GenericTypePattern::Awaited(inner) => Ok(awaited_hir_type(instantiate_generic_pattern(
            inner,
            substitution,
        )?)),
        GenericTypePattern::NonNullable(inner) => {
            non_nullable_hir_type(instantiate_generic_pattern(inner, substitution)?)
        }
        GenericTypePattern::Partial(inner) => {
            partial_hir_type(instantiate_generic_pattern(inner, substitution)?)
        }
        GenericTypePattern::Required(inner) => {
            required_hir_type(instantiate_generic_pattern(inner, substitution)?)
        }
        GenericTypePattern::Record(keys, value) => {
            let value = instantiate_generic_pattern(value, substitution)?;
            Ok(HirType::Object(
                keys.iter()
                    .map(|key| (key.clone(), value.clone()))
                    .collect(),
            ))
        }
        GenericTypePattern::Pick(inner, keys) => {
            pick_hir_type(instantiate_generic_pattern(inner, substitution)?, keys)
        }
        GenericTypePattern::Omit(inner, keys) => {
            omit_hir_type(instantiate_generic_pattern(inner, substitution)?, keys)
        }
        GenericTypePattern::IndexedAccess(inner, keys) => {
            indexed_access_hir_type(instantiate_generic_pattern(inner, substitution)?, keys)
        }
        GenericTypePattern::Dictionary(inner) => Ok(HirType::Dictionary(Box::new(
            instantiate_generic_pattern(inner, substitution)?,
        ))),
        GenericTypePattern::Object(fields) => Ok(HirType::Object(
            fields
                .iter()
                .map(|(name, ty)| {
                    Ok((name.clone(), instantiate_generic_pattern(ty, substitution)?))
                })
                .collect::<Result<Vec<_>, String>>()?,
        )),
    }
}

fn specialized_generic_function_name(
    name: &str,
    concrete_params: &[HirType],
    signature: &FnSignature,
    generic_types: &[HirType],
) -> Symbol {
    let base = specialized_generic_name(name, concrete_params);
    let hidden = signature
        .generic_type_params
        .iter()
        .zip(generic_types)
        .filter_map(|(parameter, ty)| {
            (!signature
                .generic_param_patterns
                .iter()
                .any(|pattern| generic_pattern_contains_variable(pattern, parameter)))
            .then_some(ty.clone())
        })
        .collect::<Vec<_>>();
    if hidden.is_empty() {
        base
    } else {
        format!("{base}__generic{}", specialized_generic_name("", &hidden))
    }
}

fn generic_type_pattern(
    ty: &TsType,
    substitutions: &HashMap<Symbol, GenericTypePattern>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<Symbol>,
) -> Result<GenericTypePattern, String> {
    if let TsType::TsTypeRef(reference) = ty {
        if let swc_ecma_ast::TsEntityName::Ident(id) = &reference.type_name {
            let name = id.sym.as_str();
            if let Some(pattern) = substitutions.get(name) {
                return Ok(pattern.clone());
            }
            if let Some(interface) = generic_interfaces.interfaces.get(name) {
                if in_progress.iter().any(|active| active == name) {
                    return Err(format!("generic interface `{name}` is self-referential"));
                }
                let parameters = &interface
                    .type_params
                    .as_ref()
                    .expect("generic interface type parameters")
                    .params;
                let arguments = reference
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default();
                let required = parameters
                    .iter()
                    .take_while(|parameter| parameter.default.is_none())
                    .count();
                if arguments.len() < required || arguments.len() > parameters.len() {
                    return Err(format!(
                        "generic interface `{name}` expects {required}..={} type argument(s), got {}",
                        parameters.len(),
                        arguments.len()
                    ));
                }
                let mut nested_substitutions = HashMap::new();
                for (index, parameter) in parameters.iter().enumerate() {
                    let argument = arguments
                        .get(index)
                        .map(|argument| argument.as_ref())
                        .or_else(|| parameter.default.as_ref().map(|default| default.as_ref()))
                        .expect("validated generic interface arity requires a default");
                    let pattern = generic_type_pattern(
                        argument,
                        if index < arguments.len() {
                            substitutions
                        } else {
                            &nested_substitutions
                        },
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?;
                    nested_substitutions.insert(parameter.name.sym.to_string(), pattern);
                }
                in_progress.push(name.to_string());
                let mut fields = Vec::new();
                let mut dictionary = None;
                for base in &interface.extends {
                    let Expr::Ident(base_ident) = base.expr.as_ref() else {
                        return Err(format!(
                            "generic interface `{name}` has an unsupported `extends` target"
                        ));
                    };
                    let base_reference = TsType::TsTypeRef(swc_ecma_ast::TsTypeRef {
                        span: base.span,
                        type_name: swc_ecma_ast::TsEntityName::Ident(base_ident.clone()),
                        type_params: base.type_args.clone(),
                    });
                    let base_pattern = generic_type_pattern(
                        &base_reference,
                        &nested_substitutions,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?;
                    let base_fields = match base_pattern {
                        GenericTypePattern::Object(fields) => fields,
                        GenericTypePattern::Concrete(HirType::Object(fields)) => fields
                            .into_iter()
                            .map(|(field, ty)| (field, GenericTypePattern::Concrete(ty)))
                            .collect(),
                        GenericTypePattern::Dictionary(element) => {
                            if dictionary
                                .as_ref()
                                .is_some_and(|existing| existing != element.as_ref())
                            {
                                return Err(format!(
                                    "generic interface `{name}` inherits incompatible dictionary value types"
                                ));
                            }
                            dictionary = Some(*element);
                            Vec::new()
                        }
                        GenericTypePattern::Concrete(HirType::Dictionary(element)) => {
                            let element = GenericTypePattern::Concrete(*element);
                            if dictionary
                                .as_ref()
                                .is_some_and(|existing| existing != &element)
                            {
                                return Err(format!(
                                    "generic interface `{name}` inherits incompatible dictionary value types"
                                ));
                            }
                            dictionary = Some(element);
                            Vec::new()
                        }
                        _ => {
                            return Err(format!(
                            "generic interface `{name}` can only extend an object-shaped interface"
                        ))
                        }
                    };
                    for (field_name, field_ty) in base_fields {
                        if fields.iter().any(|(existing, _)| existing == &field_name) {
                            return Err(format!(
                                "generic interface `{name}` inherits duplicate field `{field_name}`"
                            ));
                        }
                        fields.push((field_name, field_ty));
                    }
                }
                let mut own_fields = Vec::new();
                for member in &interface.body.body {
                    if let TsTypeElement::TsIndexSignature(signature) = member {
                        let value = generic_type_pattern(
                            index_signature_value(signature)?,
                            &nested_substitutions,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        )?;
                        if dictionary
                            .as_ref()
                            .is_some_and(|existing| existing != &value)
                        {
                            return Err(format!(
                                "generic interface `{name}` declares an incompatible dictionary value type"
                            ));
                        }
                        dictionary = Some(value);
                        continue;
                    }
                    let (key, ty, optional) = match member {
                        TsTypeElement::TsPropertySignature(property) => {
                            let annotation = property.type_ann.as_ref().ok_or_else(|| {
                                "generic interface property needs a type annotation".to_string()
                            })?;
                            (
                                property.key.as_ref(),
                                std::borrow::Cow::Borrowed(annotation.type_ann.as_ref()),
                                property.optional,
                            )
                        }
                        TsTypeElement::TsMethodSignature(method) => (
                            method.key.as_ref(),
                            std::borrow::Cow::Owned(method_signature_function_type(method)?),
                            false,
                        ),
                        _ => {
                            return Err(format!(
                                "generic interface `{name}` only supports properties and methods"
                            ))
                        }
                    };
                    let Expr::Ident(field) = key else {
                        return Err(format!(
                            "generic interface `{name}` has an unsupported property key"
                        ));
                    };
                    let ty = generic_type_pattern(
                        &ty,
                        &nested_substitutions,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?;
                    let field_ty = if optional {
                        GenericTypePattern::Optional(Box::new(ty))
                    } else {
                        ty
                    };
                    if dictionary
                        .as_ref()
                        .is_some_and(|element| element != &field_ty)
                    {
                        return Err(format!(
                                "generic interface `{name}` property `{}` does not match its index value type",
                                field.sym
                            ));
                    }
                    own_fields.push((field.sym.to_string(), field_ty));
                }
                for (field_name, field_ty) in own_fields {
                    if fields.iter().any(|(existing, _)| existing == &field_name) {
                        return Err(format!(
                            "generic interface `{name}` declares inherited field `{field_name}` again"
                        ));
                    }
                    fields.push((field_name, field_ty));
                }
                in_progress.pop();
                return Ok(if let Some(element) = dictionary {
                    GenericTypePattern::Dictionary(Box::new(element))
                } else {
                    GenericTypePattern::Object(fields)
                });
            }
            if let Some(alias) = generic_interfaces.aliases.get(name) {
                if in_progress.iter().any(|active| active == name) {
                    return Err(format!("generic type alias `{name}` is self-referential"));
                }
                let parameters = &alias
                    .type_params
                    .as_ref()
                    .expect("generic alias type parameters")
                    .params;
                let arguments = reference
                    .type_params
                    .as_ref()
                    .map(|parameters| parameters.params.as_slice())
                    .unwrap_or_default();
                let required = parameters
                    .iter()
                    .take_while(|parameter| parameter.default.is_none())
                    .count();
                if arguments.len() < required || arguments.len() > parameters.len() {
                    return Err(format!(
                        "generic type alias `{name}` expects {required}..={} type argument(s), got {}",
                        parameters.len(),
                        arguments.len()
                    ));
                }
                let mut nested = HashMap::new();
                for (index, parameter) in parameters.iter().enumerate() {
                    let argument = arguments
                        .get(index)
                        .map(|argument| argument.as_ref())
                        .or_else(|| parameter.default.as_ref().map(|default| default.as_ref()))
                        .expect("validated generic alias arity requires a default");
                    let pattern = generic_type_pattern(
                        argument,
                        if index < arguments.len() {
                            substitutions
                        } else {
                            &nested
                        },
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?;
                    nested.insert(parameter.name.sym.to_string(), pattern);
                }
                in_progress.push(name.to_string());
                let result = generic_type_pattern(
                    &alias.type_ann,
                    &nested,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
                in_progress.pop();
                return result;
            }
            if name == "Record" {
                let [keys, value] = reference
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return Err("Record<K, V> requires exactly two type arguments".into());
                };
                let value = Box::new(generic_type_pattern(
                    value,
                    substitutions,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?);
                if is_string_keyword(keys) {
                    return Ok(GenericTypePattern::Dictionary(value));
                }
                return Ok(GenericTypePattern::Record(
                    generic_utility_keys(
                        keys,
                        substitutions,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?,
                    value,
                ));
            }
            if name == "Pick" || name == "Omit" {
                let [object, keys] = reference
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return Err(format!("{name}<T, K> requires exactly two type arguments"));
                };
                let object = Box::new(generic_type_pattern(
                    object,
                    substitutions,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?);
                let keys = generic_utility_keys(
                    keys,
                    substitutions,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?;
                return Ok(if name == "Pick" {
                    GenericTypePattern::Pick(object, keys)
                } else {
                    GenericTypePattern::Omit(object, keys)
                });
            }
            if let Some(inner) = reference
                .type_params
                .as_ref()
                .and_then(|params| params.params.as_slice().first())
            {
                let inner = Box::new(generic_type_pattern(
                    inner,
                    substitutions,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?);
                match name {
                    "Array" | "ReadonlyArray" => return Ok(GenericTypePattern::Array(inner)),
                    "Promise" => return Ok(GenericTypePattern::Promise(inner)),
                    "Readonly" => return Ok(*inner),
                    "Awaited" => return Ok(GenericTypePattern::Awaited(inner)),
                    "NonNullable" => return Ok(GenericTypePattern::NonNullable(inner)),
                    "Partial" => return Ok(GenericTypePattern::Partial(inner)),
                    "Required" => return Ok(GenericTypePattern::Required(inner)),
                    _ => {}
                }
            }
        }
    }
    match ty {
        TsType::TsTypeOperator(operator)
            if operator.op == swc_ecma_ast::TsTypeOperatorOp::KeyOf =>
        {
            generic_pattern_keys(&generic_type_pattern(
                &operator.type_ann,
                substitutions,
                interfaces,
                generic_interfaces,
                in_progress,
            )?)?;
            Ok(GenericTypePattern::Concrete(HirType::Str))
        }
        TsType::TsTypeOperator(operator)
            if operator.op == swc_ecma_ast::TsTypeOperatorOp::ReadOnly =>
        {
            generic_type_pattern(
                &operator.type_ann,
                substitutions,
                interfaces,
                generic_interfaces,
                in_progress,
            )
        }
        TsType::TsArrayType(array) => {
            Ok(GenericTypePattern::Array(Box::new(generic_type_pattern(
                &array.elem_type,
                substitutions,
                interfaces,
                generic_interfaces,
                in_progress,
            )?)))
        }
        TsType::TsIndexedAccessType(indexed) => {
            let object = Box::new(generic_type_pattern(
                &indexed.obj_type,
                substitutions,
                interfaces,
                generic_interfaces,
                in_progress,
            )?);
            let keys = generic_utility_keys(
                &indexed.index_type,
                substitutions,
                interfaces,
                generic_interfaces,
                in_progress,
            )?;
            Ok(GenericTypePattern::IndexedAccess(object, keys))
        }
        TsType::TsTypeLit(literal) => {
            if let Some(signature) = type_literal_index_signature(literal)? {
                let value = generic_type_pattern(
                    index_signature_value(signature)?,
                    substitutions,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?;
                for member in &literal.members {
                    match member {
                        TsTypeElement::TsIndexSignature(_) => {}
                        TsTypeElement::TsPropertySignature(property) => {
                            let annotation = property.type_ann.as_ref().ok_or_else(|| {
                                "dictionary property needs a type annotation".to_string()
                            })?;
                            let field = generic_type_pattern(
                                &annotation.type_ann,
                                substitutions,
                                interfaces,
                                generic_interfaces,
                                in_progress,
                            )?;
                            if property.optional || field != value {
                                return Err(
                                    "dictionary properties must match the index value type".into(),
                                );
                            }
                        }
                        _ => return Err("dictionary types only support properties".into()),
                    }
                }
                return Ok(GenericTypePattern::Dictionary(Box::new(value)));
            }
            Ok(GenericTypePattern::Object(
                literal
                    .members
                    .iter()
                    .map(|member| {
                        let (key, ty, optional) = match member {
                            TsTypeElement::TsPropertySignature(property) => {
                                let annotation = property.type_ann.as_ref().ok_or_else(|| {
                                    "generic object property needs a type annotation".to_string()
                                })?;
                                (
                                    property.key.as_ref(),
                                    std::borrow::Cow::Borrowed(annotation.type_ann.as_ref()),
                                    property.optional,
                                )
                            }
                            TsTypeElement::TsMethodSignature(method) => (
                                method.key.as_ref(),
                                std::borrow::Cow::Owned(method_signature_function_type(method)?),
                                false,
                            ),
                            _ => {
                                return Err(
                                    "generic object patterns only support properties and methods"
                                        .into(),
                                )
                            }
                        };
                        let Expr::Ident(field) = key else {
                            return Err("generic object pattern has an unsupported key".into());
                        };
                        let ty = generic_type_pattern(
                            &ty,
                            substitutions,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        )?;
                        Ok((
                            field.sym.to_string(),
                            if optional {
                                GenericTypePattern::Optional(Box::new(ty))
                            } else {
                                ty
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            ))
        }
        other => Ok(GenericTypePattern::Concrete(lower_ts_type(
            other,
            interfaces,
            generic_interfaces,
        )?)),
    }
}

fn match_generic_pattern(
    pattern: &GenericTypePattern,
    actual: &HirType,
    inferred: &mut HashMap<Symbol, HirType>,
) -> Result<(), String> {
    match (pattern, actual) {
        (GenericTypePattern::Variable(name), actual) => {
            if let Some(previous) = inferred.get(name) {
                if previous != actual {
                    return Err(format!(
                    "generic type parameter has conflicting call-site types {previous:?} and {actual:?}"
                ));
                }
            } else {
                inferred.insert(name.clone(), actual.clone());
            }
            Ok(())
        }
        (GenericTypePattern::Array(expected), HirType::Array(value))
        | (GenericTypePattern::Promise(expected), HirType::Promise(value)) => {
            match_generic_pattern(expected, value, inferred)
        }
        (GenericTypePattern::Optional(expected), HirType::Optional(value)) => {
            match_generic_pattern(expected, value, inferred)
        }
        (GenericTypePattern::Awaited(expected), actual) => {
            match_generic_pattern(expected, actual, inferred)
        }
        (GenericTypePattern::NonNullable(expected), actual) => {
            match_generic_pattern(expected, actual, inferred)
        }
        (GenericTypePattern::Partial(expected), actual)
        | (GenericTypePattern::Required(expected), actual) => {
            match_generic_pattern(expected, actual, inferred)
        }
        (GenericTypePattern::Record(keys, expected), HirType::Object(fields))
            if keys.len() == fields.len() =>
        {
            for (key, (name, actual)) in keys.iter().zip(fields) {
                if key != name {
                    return Err(format!(
                        "generic Record key `{name}` does not match `{key}`"
                    ));
                }
                match_generic_pattern(expected, actual, inferred)?;
            }
            Ok(())
        }
        (GenericTypePattern::Pick(expected, _), actual)
        | (GenericTypePattern::Omit(expected, _), actual) => {
            match_generic_pattern(expected, actual, inferred)
        }
        (GenericTypePattern::IndexedAccess(expected, _), actual) => {
            match_generic_pattern(expected, actual, inferred)
        }
        (GenericTypePattern::Dictionary(expected), HirType::Dictionary(actual)) => {
            match_generic_pattern(expected, actual, inferred)
        }
        (GenericTypePattern::Object(expected), HirType::Object(value))
            if expected.len() == value.len() =>
        {
            for ((expected_name, expected_ty), (actual_name, actual_ty)) in
                expected.iter().zip(value)
            {
                if expected_name != actual_name {
                    return Err(format!(
                        "generic argument object field `{actual_name}` does not match `{expected_name}`"
                    ));
                }
                match_generic_pattern(expected_ty, actual_ty, inferred)?;
            }
            Ok(())
        }
        // A parameter whose type parameter got substituted away into a
        // plain `Json` fallback (a generic Fallback declaration doing the
        // same "can't classify it precisely, pass it through as Json"
        // substitution `typed_dynamic_declaration` already does for
        // ordinary parameters -- see thaw-cli's shims.rs) still accepts
        // any real argument type here, the same way `coerce_to_declared`
        // itself accepts any JSON-convertible value into a `Json`
        // parameter later. Real example: date-fns's `addDays<DateType
        // extends Date>(date: DateType | number | string, amount:
        // number): DateType`, whose `date` union becomes `Concrete(Json)`
        // once generated, but `DateType` is still inferred from the
        // return position (a `let`/`const` annotation) -- a real `Date`
        // argument there must not fail this unrelated pattern check.
        (GenericTypePattern::Concrete(HirType::Json), _) => Ok(()),
        (GenericTypePattern::Concrete(expected), actual)
            if *expected == HirType::Dynamic || expected == actual =>
        {
            Ok(())
        }
        _ => Err(format!(
            "generic argument has type {actual:?}, incompatible with parameter pattern {pattern:?}"
        )),
    }
}

fn infer_generic_type_tuple(
    signature: &FnSignature,
    actual_params: &[HirType],
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    expected_return: Option<&HirType>,
) -> Result<Vec<HirType>, String> {
    let mut inferred = HashMap::new();
    for (pattern, actual) in signature.generic_param_patterns.iter().zip(actual_params) {
        match_generic_pattern(pattern, actual, &mut inferred)?;
    }
    // A type parameter that appears in no argument at all (e.g. `nanoid
    // <Type extends string>(size?: number): Type`, where `Type` shows up
    // solely in the return position) has nothing above to infer it from.
    // If the call site has a contextual expected type to offer (a
    // `let`/`const` declaration's own annotation -- see
    // `lower_expr_with_expected_type`), try matching it against the
    // return type's own pattern as a last resort before falling through
    // to the "cannot infer" error below. Never lets this override an
    // argument that already provided a value: `match_generic_pattern`'s
    // `Variable` case only *checks* an existing entry for a conflict
    // rather than overwriting it, and any such conflict here is just
    // discarded (kept silent, unlike a real argument mismatch) since a
    // merely-unhelpful contextual type shouldn't turn into a hard error
    // when the call was otherwise going to succeed without it.
    if let (Some(expected), Some(pattern)) = (expected_return, &signature.generic_return_pattern) {
        let _ = match_generic_pattern(pattern, expected, &mut inferred);
    }
    let mut types = Vec::with_capacity(signature.generic_type_params.len());
    let mut substitution = HashMap::new();
    for ((name, default), index) in signature
        .generic_type_params
        .iter()
        .zip(&signature.generic_type_defaults)
        .zip(0..)
    {
        let concrete = if let Some(inferred) = inferred.get(name) {
            inferred.clone()
        } else if let Some(default) = default {
            resolve_ts_type_with_substitution(
                default,
                &substitution,
                interfaces,
                generic_interfaces,
                &mut Vec::new(),
            )?
        } else {
            return Err(format!(
                "cannot infer generic type parameter `{name}` from this call (parameter {})",
                index + 1
            ));
        };
        substitution.insert(name.clone(), concrete.clone());
        types.push(concrete);
    }
    for ((name, actual), constraint) in signature
        .generic_type_params
        .iter()
        .zip(&types)
        .zip(&signature.generic_type_constraints)
    {
        let Some(constraint) = constraint else {
            continue;
        };
        let constraint = resolve_ts_type_with_substitution(
            constraint,
            &substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )?;
        if !type_satisfies_constraint(actual, &constraint) {
            return Err(format!(
                "inferred type {actual:?} does not satisfy constraint {constraint:?} for `{name}`"
            ));
        }
    }
    Ok(types)
}

fn resolve_explicit_generic_type_tuple(
    signature: &FnSignature,
    arguments: &[Box<TsType>],
    actual_params: &[HirType],
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Vec<HirType>, String> {
    let required = signature
        .generic_type_defaults
        .iter()
        .filter(|default| default.is_none())
        .count();
    if arguments.len() < required || arguments.len() > signature.generic_type_params.len() {
        let expected = if required == signature.generic_type_params.len() {
            required.to_string()
        } else {
            format!("{required}..={}", signature.generic_type_params.len())
        };
        return Err(format!(
            "expects {expected} explicit type argument(s), got {}",
            arguments.len()
        ));
    }

    let mut types = Vec::with_capacity(signature.generic_type_params.len());
    let mut substitution = HashMap::new();
    for (index, name) in signature.generic_type_params.iter().enumerate() {
        let concrete = if let Some(argument) = arguments.get(index) {
            lower_ts_type(argument, interfaces, generic_interfaces)?
        } else {
            resolve_ts_type_with_substitution(
                signature.generic_type_defaults[index]
                    .as_ref()
                    .expect("validated explicit generic arity requires a default"),
                &substitution,
                interfaces,
                generic_interfaces,
                &mut Vec::new(),
            )?
        };
        substitution.insert(name.clone(), concrete.clone());
        types.push(concrete);
    }

    for ((name, actual), constraint) in signature
        .generic_type_params
        .iter()
        .zip(&types)
        .zip(&signature.generic_type_constraints)
    {
        let Some(constraint) = constraint else {
            continue;
        };
        let constraint = resolve_ts_type_with_substitution(
            constraint,
            &substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )?;
        if !type_satisfies_constraint(actual, &constraint) {
            return Err(format!(
                "explicit type {actual:?} does not satisfy constraint {constraint:?} for `{name}`"
            ));
        }
    }

    let mut inferred = HashMap::new();
    for (pattern, actual) in signature.generic_param_patterns.iter().zip(actual_params) {
        match_generic_pattern(pattern, actual, &mut inferred)?;
    }
    for (name, inferred) in inferred {
        let explicit = &substitution[&name];
        if &inferred != explicit {
            return Err(format!(
                "argument infers {name} as {inferred:?}, but explicit type is {explicit:?}"
            ));
        }
    }
    Ok(types)
}

#[allow(clippy::too_many_arguments)]
fn lower_generic_instance(
    fn_decl: &FnDecl,
    types: &[HirType],
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
    call_constraints: Option<&RefCell<Vec<CallConstraint>>>,
) -> Result<HirFunction, String> {
    let base_name = fn_decl.ident.sym.to_string();
    let signature = &signatures[&base_name];
    let substitution = signature
        .generic_type_params
        .iter()
        .cloned()
        .zip(types.iter().cloned())
        .collect::<HashMap<_, _>>();
    let params = fn_decl
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
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ret = lower_fn_return_type(
        fn_decl.function.is_async,
        &fn_decl.function.return_type,
        &base_name,
        interfaces,
        generic_interfaces,
        &substitution,
    )?;
    let mut concrete_signatures = signatures.clone();
    let concrete = concrete_signatures.get_mut(&base_name).unwrap();
    concrete.params = params.iter().map(|param| param.ty.clone()).collect();
    concrete.ret = ret.clone();
    let mut lowerer = FnLowerer::new(
        &concrete_signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        ret.clone(),
        call_constraints,
    );
    seed_global_scope(&mut lowerer, global_types, immutable_globals);
    for (source, param) in fn_decl.function.params.iter().zip(&params) {
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
    let body = lowerer.lower_stmts(
        &fn_decl
            .function
            .body
            .as_ref()
            .ok_or_else(|| format!("function `{base_name}` has no body"))?
            .stmts,
    )?;
    Ok(HirFunction {
        name: specialized_generic_function_name(
            &base_name,
            &params
                .iter()
                .map(|param| param.ty.clone())
                .collect::<Vec<_>>(),
            signature,
            types,
        ),
        params,
        ret,
        is_async: fn_decl.function.is_async,
        body,
    })
}
