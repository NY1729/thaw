fn function_type_substitution(function: &swc_ecma_ast::Function) -> HashMap<Symbol, HirType> {
    function
        .type_params
        .as_ref()
        .map(|params| {
            params
                .params
                .iter()
                .map(|param| (param.name.sym.to_string(), HirType::Dynamic))
                .collect()
        })
        .unwrap_or_default()
}

fn lower_ts_type(
    ty: &TsType,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<HirType, String> {
    match ty {
        TsType::TsTypePredicate(_) => Ok(HirType::Bool),
        TsType::TsParenthesizedType(parenthesized) => lower_ts_type(
            &parenthesized.type_ann,
            interfaces,
            generic_interfaces,
        ),
        TsType::TsTypeOperator(operator)
            if operator.op == swc_ecma_ast::TsTypeOperatorOp::KeyOf =>
        {
            hir_object_keys(lower_ts_type(
                &operator.type_ann,
                interfaces,
                generic_interfaces,
            )?)?;
            Ok(HirType::Str)
        }
        TsType::TsTypeOperator(operator)
            if operator.op == swc_ecma_ast::TsTypeOperatorOp::ReadOnly =>
        {
            lower_ts_type(&operator.type_ann, interfaces, generic_interfaces)
        }
        TsType::TsKeywordType(kw) => match kw.kind {
            TsKeywordTypeKind::TsNumberKeyword => Ok(HirType::F64),
            TsKeywordTypeKind::TsBigIntKeyword => Ok(HirType::I64),
            TsKeywordTypeKind::TsStringKeyword => Ok(HirType::Str),
            TsKeywordTypeKind::TsSymbolKeyword => Ok(HirType::Symbol),
            TsKeywordTypeKind::TsBooleanKeyword => Ok(HirType::Bool),
            TsKeywordTypeKind::TsUndefinedKeyword => Ok(HirType::Undefined),
            TsKeywordTypeKind::TsNullKeyword => Ok(HirType::Null),
            TsKeywordTypeKind::TsVoidKeyword => Ok(HirType::Void),
            TsKeywordTypeKind::TsAnyKeyword | TsKeywordTypeKind::TsUnknownKeyword => {
                Ok(HirType::Json)
            }
            TsKeywordTypeKind::TsObjectKeyword => Ok(HirType::Dynamic),
            other => Err(format!(
                "unsupported type keyword {other:?}"
            )),
        },
        TsType::TsLitType(literal) => match &literal.lit {
            swc_ecma_ast::TsLit::Number(_) => Ok(HirType::F64),
            swc_ecma_ast::TsLit::BigInt(_) => Ok(HirType::I64),
            swc_ecma_ast::TsLit::Str(_) => Ok(HirType::Str),
            swc_ecma_ast::TsLit::Bool(_) => Ok(HirType::Bool),
            swc_ecma_ast::TsLit::Tpl(_) => Ok(HirType::Str),
        },
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            let mut elements = Vec::new();
            for element in &union.types {
                let element = lower_ts_type(element, interfaces, generic_interfaces)?;
                if !elements.contains(&element) {
                    elements.push(element);
                }
            }
            if let [element] = elements.as_slice() {
                return Ok(element.clone());
            }
            if elements.len() == 3
                && elements.contains(&HirType::Null)
                && elements.contains(&HirType::Undefined)
            {
                let payloads = elements
                    .iter()
                    .filter(|element| {
                        **element != HirType::Null && **element != HirType::Undefined
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if let [payload] = payloads.as_slice() {
                    return Ok(HirType::Nullish(Box::new(payload.clone())));
                }
            }
            if elements.len() == 2 {
                if elements[0] == HirType::Undefined {
                    return Ok(HirType::Optional(Box::new(elements[1].clone())));
                }
                if elements[1] == HirType::Undefined {
                    return Ok(HirType::Optional(Box::new(elements[0].clone())));
                }
                if elements[0] == HirType::Null {
                    return Ok(HirType::Nullable(Box::new(elements[1].clone())));
                }
                if elements[1] == HirType::Null {
                    return Ok(HirType::Nullable(Box::new(elements[0].clone())));
                }
            }
            if elements.len() >= 2
                && elements.iter().all(|element| {
                    matches!(
                        element,
                        HirType::F64
                            | HirType::I64
                            | HirType::Bool
                            | HirType::Str
                            | HirType::Json
                            | HirType::JsValue
                            | HirType::Array(_)
                            | HirType::Tuple(_)
                            | HirType::Object(_)
                            | HirType::Function(_, _)
                            | HirType::Undefined
                            | HirType::Null
                    )
                })
            {
                Ok(HirType::Union(elements))
            } else {
                Err(format!("unsupported union type {elements:?}"))
            }
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => {
            let elements = intersection
                .types
                .iter()
                .map(|element| lower_ts_type(element, interfaces, generic_interfaces))
                .collect::<Result<Vec<_>, _>>()?;
            let Some(first) = elements.first() else {
                return Err("empty intersection type is not supported".into());
            };
            if elements.iter().all(|element| element == first) {
                Ok(first.clone())
            } else if elements.iter().all(|element| matches!(element, HirType::Object(_))) {
                let mut merged = Vec::<(Symbol, HirType)>::new();
                for element in elements {
                    let HirType::Object(fields) = element else { unreachable!() };
                    for (name, ty) in fields {
                        if let Some((_, existing)) =
                            merged.iter().find(|(existing, _)| existing == &name)
                        {
                            if existing != &ty {
                                return Err(format!(
                                    "intersection field `{name}` has conflicting types {existing:?} and {ty:?}"
                                ));
                            }
                        } else {
                            merged.push((name, ty));
                        }
                    }
                }
                Ok(HirType::Object(merged))
            } else {
                Err(format!("unsupported intersection type {elements:?}"))
            }
        }
        TsType::TsArrayType(arr) => Ok(HirType::Array(Box::new(lower_ts_type(
            &arr.elem_type,
            interfaces,
            generic_interfaces,
        )?))),
        TsType::TsOptionalType(optional) => Ok(HirType::Optional(Box::new(lower_ts_type(
            &optional.type_ann,
            interfaces,
            generic_interfaces,
        )?))),
        TsType::TsTupleType(tuple) => {
            if let Some(TsType::TsRestType(rest)) =
                tuple.elem_types.last().map(|element| element.ty.as_ref())
            {
                let HirType::Array(rest) =
                    lower_ts_type(&rest.type_ann, interfaces, generic_interfaces)?
                else {
                    return Err("tuple rest element needs an array type".into());
                };
                let prefix = tuple.elem_types[..tuple.elem_types.len() - 1]
                    .iter()
                    .map(|element| lower_ts_type(&element.ty, interfaces, generic_interfaces))
                    .collect::<Result<Vec<_>, _>>()?;
                if prefix.iter().all(|element| element == rest.as_ref()) {
                    return Ok(HirType::Array(rest));
                }
                return Ok(HirType::Array(Box::new(HirType::Json)));
            }
            Ok(HirType::Tuple(
                tuple
                    .elem_types
                    .iter()
                    .map(|element| lower_ts_type(&element.ty, interfaces, generic_interfaces))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        TsType::TsMappedType(mapped) => {
            if mapped.name_type.is_some() {
                return Err("mapped type key remapping is not supported".into());
            }
            let constraint = mapped
                .type_param
                .constraint
                .as_ref()
                .ok_or("mapped type parameter needs a key constraint")?;
            let keys = utility_keys(constraint, interfaces, generic_interfaces)?;
            let value = mapped
                .type_ann
                .as_ref()
                .ok_or("mapped type needs a value annotation")?;
            let mut value = lower_ts_type(value, interfaces, generic_interfaces)?;
            if matches!(
                mapped.optional,
                Some(swc_ecma_ast::TruePlusMinus::True | swc_ecma_ast::TruePlusMinus::Plus)
            ) {
                value = optional_parameter_type(value);
            }
            Ok(HirType::Object(
                keys.into_iter().map(|key| (key, value.clone())).collect(),
            ))
        }
        TsType::TsIndexedAccessType(indexed) => indexed_access_hir_type(
            lower_ts_type(&indexed.obj_type, interfaces, generic_interfaces)?,
            &utility_keys(&indexed.index_type, interfaces, generic_interfaces)?,
        ),
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            if function.type_params.is_some() {
                // Generic callable values have no single native ABI. Class/interface
                // method calls with a known receiver still specialize through the
                // existing generic-method pipeline.
                return Ok(HirType::Dynamic);
            }
            let mut params = Vec::new();
            let mut optional = Vec::new();
            let mut rest = None;
            for (index, param) in function.params.iter().enumerate() {
                match param {
                    TsFnParam::Ident(param) => {
                        let annotation = param.type_ann.as_ref().ok_or_else(|| {
                            format!(
                                "function parameter `{}` needs a type annotation",
                                param.id.sym
                            )
                        })?;
                        let mut ty = lower_ts_type(
                            &annotation.type_ann,
                            interfaces,
                            generic_interfaces,
                        )?;
                        if param.id.optional {
                            ty = optional_parameter_type(ty);
                        }
                        params.push(ty);
                        optional.push(param.id.optional);
                    }
                    TsFnParam::Rest(param) if index + 1 == function.params.len() => {
                        let annotation = param
                            .type_ann
                            .as_ref()
                            .ok_or("function rest parameter needs an array type annotation")?;
                        let lowered = lower_ts_type(
                            &annotation.type_ann,
                            interfaces,
                            generic_interfaces,
                        )?;
                        let HirType::Array(element) = lowered else {
                            return Err(
                                "function rest parameter needs an array type annotation".into(),
                            );
                        };
                        rest = Some(element);
                    }
                    TsFnParam::Rest(_) => {
                        return Err("function rest parameter must be last".into())
                    }
                    _ => {
                        return Err(
                            "function types only support identifier and trailing rest parameters"
                                .into(),
                        )
                    }
                }
            }
            let ret = lower_ts_type(
                &function.type_ann.type_ann,
                interfaces,
                generic_interfaces,
            )?;
            Ok(if rest.is_some() || optional.iter().any(|value| *value) {
                HirType::CallableFunction(
                    params,
                    optional_parameter_mask(&optional),
                    rest,
                    Box::new(ret),
                )
            } else {
                HirType::Function(params, Box::new(ret))
            })
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsConstructorType(constructor)) => {
            lower_ts_type(
                &TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(
                    swc_ecma_ast::TsFnType {
                        span: constructor.span,
                        params: constructor.params.clone(),
                        type_params: constructor.type_params.clone(),
                        type_ann: constructor.type_ann.clone(),
                    },
                )),
                interfaces,
                generic_interfaces,
            )
        }
        TsType::TsTypeRef(ty_ref) => {
            let ref_name = match &ty_ref.type_name {
                swc_ecma_ast::TsEntityName::Ident(id) => Some(id.sym.as_str()),
                swc_ecma_ast::TsEntityName::TsQualifiedName(_) => None,
            };

            // A name matching a resolved (non-generic) `interface` --
            // treated exactly like an inline `{ ... }` type literal.
            if let Some(name) = ref_name {
                if name == "RegExp" {
                    return Ok(regex_object_type());
                }
                if name == "Date" {
                    return Ok(date_object_type());
                }
                if let Some(resolved) = interfaces.get(name) {
                    return Ok(resolved.clone());
                }
                // A generic interface, referenced with concrete type
                // arguments -- resolved on demand via substitution. See
                // `resolve_generic_interface`'s doc comment for scope
                // limits (no nested-inside-another-interface use, no
                // `extends` on the generic interface itself).
                if let Some(decl) = generic_interfaces.interfaces.get(name) {
                    return resolve_generic_interface(
                        name,
                        decl,
                        ty_ref,
                        interfaces,
                        generic_interfaces,
                        None,
                        &mut Vec::new(),
                    );
                }
                if let Some(decl) = generic_interfaces.aliases.get(name) {
                    return resolve_generic_alias(
                        name,
                        decl,
                        ty_ref,
                        interfaces,
                        generic_interfaces,
                        None,
                        &mut Vec::new(),
                    );
                }
            }

            // `Json`, with no type arguments -- the annotation spelling for
            // `HirType::Json` (a dynamic value, e.g. from `JSON.parse`).
            // Without this there was no way to *write* a `Json`-typed
            // parameter/`let` annotation; it could only ever be inferred
            // as an expression's type. Needed for e.g. thaw-bridge's
            // generated Fallback wrappers (`function f(args: Json): Json`).
            if ref_name == Some("Json") && ty_ref.type_params.is_none() {
                return Ok(HirType::Json);
            }
            if ref_name == Some("JsValue") && ty_ref.type_params.is_none() {
                return Ok(HirType::JsValue);
            }
            if ref_name == Some("Record") {
                let [keys, value] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return Err("Record<K, V> requires exactly two type arguments".into());
                };
                let value = lower_ts_type(value, interfaces, generic_interfaces)?;
                if is_string_keyword(keys) {
                    return Ok(HirType::Dictionary(Box::new(value)));
                }
                return Ok(HirType::Object(
                    utility_keys(keys, interfaces, generic_interfaces)?
                        .into_iter()
                        .map(|key| (key, value.clone()))
                        .collect(),
                ));
            }
            if matches!(ref_name, Some("Map" | "WeakMap")) {
                let name = ref_name.unwrap();
                let [key, value] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return Err(format!("{name}<K, V> requires exactly two type arguments"));
                };
                let key = lower_ts_type(key, interfaces, generic_interfaces)?;
                if name == "WeakMap" {
                    weak_key_intrinsic_suffix(&key)?;
                } else {
                    map_key_intrinsic_suffix(&key)?;
                }
                let value = lower_ts_type(value, interfaces, generic_interfaces)?;
                return Ok(HirType::Map(Box::new(key), Box::new(value)));
            }
            if matches!(ref_name, Some("Set" | "WeakSet")) {
                let name = ref_name.unwrap();
                let [element] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return Err(format!("{name}<T> requires exactly one type argument"));
                };
                let element = lower_ts_type(element, interfaces, generic_interfaces)?;
                if name == "WeakSet" {
                    weak_key_intrinsic_suffix(&element)?;
                } else {
                    map_key_intrinsic_suffix(&element)?;
                }
                return Ok(HirType::Set(Box::new(element)));
            }
            if matches!(ref_name, Some("Pick" | "Omit")) {
                let [object, keys] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return Err(format!(
                        "{}<T, K> requires exactly two type arguments",
                        ref_name.unwrap()
                    ));
                };
                let object = lower_ts_type(object, interfaces, generic_interfaces)?;
                let keys = utility_keys(keys, interfaces, generic_interfaces)?;
                return if ref_name == Some("Pick") {
                    pick_hir_type(object, &keys)
                } else {
                    omit_hir_type(object, &keys)
                };
            }

            // Otherwise, accept `Array<T>` / `Promise<T>` as the two
            // other built-in generic spellings we recognize.
            let single_type_param = ty_ref
                .type_params
                .as_ref()
                .and_then(|params| match params.params.as_slice() {
                    [elem] => Some(elem.as_ref()),
                    _ => None,
                });

            if ref_name == Some("TemplateStringsArray") && ty_ref.type_params.is_none() {
                return Ok(HirType::Array(Box::new(HirType::Str)));
            }

            match (ref_name, single_type_param) {
                (Some("Array" | "ReadonlyArray"), Some(elem)) => Ok(HirType::Array(Box::new(
                    lower_ts_type(elem, interfaces, generic_interfaces)?,
                ))),
                (Some("Promise"), Some(inner)) => Ok(HirType::Promise(Box::new(lower_ts_type(
                    inner,
                    interfaces,
                    generic_interfaces,
                )?))),
                (Some("Readonly"), Some(inner)) => {
                    lower_ts_type(inner, interfaces, generic_interfaces)
                }
                (Some("Awaited"), Some(inner)) => {
                    Ok(awaited_hir_type(lower_ts_type(
                        inner,
                        interfaces,
                        generic_interfaces,
                    )?))
                }
                (Some("NonNullable"), Some(inner)) => non_nullable_hir_type(lower_ts_type(
                    inner,
                    interfaces,
                    generic_interfaces,
                )?),
                (Some("Partial"), Some(inner)) => partial_hir_type(lower_ts_type(
                    inner,
                    interfaces,
                    generic_interfaces,
                )?),
                (Some("Required"), Some(inner)) => required_hir_type(lower_ts_type(
                    inner,
                    interfaces,
                    generic_interfaces,
                )?),
                _ => Err("unsupported type reference (generics are not supported yet)".into()),
            }
        }
        TsType::TsTypeLit(type_lit) => {
            if let Some(signature) = type_literal_index_signature(type_lit)? {
                let value = lower_ts_type(
                    index_signature_value(signature)?,
                    interfaces,
                    generic_interfaces,
                )?;
                for member in &type_lit.members {
                    match member {
                        TsTypeElement::TsIndexSignature(_) => {}
                        TsTypeElement::TsPropertySignature(property) => {
                            let annotation = property.type_ann.as_ref().ok_or_else(|| {
                                "dictionary property needs an explicit type annotation".to_string()
                            })?;
                            let field = lower_ts_type(
                                &annotation.type_ann,
                                interfaces,
                                generic_interfaces,
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
                return Ok(HirType::Dictionary(Box::new(value)));
            }
            let fields = type_lit
                .members
                .iter()
                .map(|member| {
                    let (key, ty, optional) = match member {
                        TsTypeElement::TsPropertySignature(property) => {
                            let annotation = property.type_ann.as_ref().ok_or_else(|| {
                                "object property needs an explicit type annotation".to_string()
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
                        _ => return Err("object types only support properties and methods".into()),
                    };
                    let name = match key {
                        Expr::Ident(ident) => ident.sym.to_string(),
                        _ => return Err("unsupported object type literal key".into()),
                    };
                    let mut ty = lower_ts_type(&ty, interfaces, generic_interfaces)?;
                    if optional {
                        ty = optional_parameter_type(ty);
                    }
                    Ok((name, ty))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(HirType::Object(fields))
        }
        other => Err(format!(
            "unsupported type annotation {other:?} (supports primitive keywords, T[]/Array<T>, interfaces, and object type literals)"
        )),
    }
}

/// Resolves `Name<ConcreteArg, ...>` for a generic interface `Name`, by
/// substituting each type parameter with its corresponding concrete
/// argument's `HirType` throughout the interface's field types (see
/// `resolve_ts_type_with_substitution`). `in_progress` guards against a
/// generic interface that references itself (directly, or through another
/// generic interface) -- freshly created at each top-level `lower_ts_type`
/// call, so it only needs to catch a cycle within one such call tree.
///
/// Base interfaces are expanded before the interface's own fields, using the
/// same substitution for generic base arguments. Type arguments are resolved
/// through an optional outer substitution, so function type variables
/// in `Box<T>` and nested forms such as `Wrapper<Box<T>>` are concrete before
/// the interface's own fields are expanded.
fn resolve_generic_interface(
    name: &str,
    decl: &TsInterfaceDecl,
    ty_ref: &swc_ecma_ast::TsTypeRef,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    outer_substitution: Option<&HashMap<Symbol, HirType>>,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if in_progress.iter().any(|n| n == name) {
        return Err(format!(
            "generic interface `{name}` is (indirectly) self-referential, which Thaw's fixed-size object layout can't represent"
        ));
    }
    let parameters = &decl
        .type_params
        .as_ref()
        .expect("caller only reaches here for a generic interface")
        .params;

    let type_args: &[Box<TsType>] = ty_ref
        .type_params
        .as_ref()
        .map(|params| params.params.as_slice())
        .unwrap_or(&[]);
    let required = parameters
        .iter()
        .take_while(|parameter| parameter.default.is_none())
        .count();
    if type_args.len() < required || type_args.len() > parameters.len() {
        let expected = if required == parameters.len() {
            required.to_string()
        } else {
            format!("{required}..={}", parameters.len())
        };
        return Err(format!(
            "interface `{name}` expects {expected} type argument(s), got {}",
            type_args.len()
        ));
    }
    let mut substitution = HashMap::new();
    for (index, parameter) in parameters.iter().enumerate() {
        let concrete = if let Some(argument) = type_args.get(index) {
            match outer_substitution {
                Some(outer) => resolve_ts_type_with_substitution(
                    argument,
                    outer,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?,
                None => lower_ts_type(argument, interfaces, generic_interfaces)?,
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
            )?
        };
        if let Some(constraint) = &parameter.constraint {
            let constraint = resolve_ts_type_with_substitution(
                constraint,
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?;
            if !type_satisfies_constraint(&concrete, &constraint) {
                return Err(format!(
                    "type argument {concrete:?} does not satisfy constraint {constraint:?} for `{}` in interface `{name}`",
                    parameter.name.sym
                ));
            }
        }
        substitution.insert(parameter.name.sym.to_string(), concrete);
    }

    in_progress.push(name.to_string());

    let mut fields = Vec::with_capacity(decl.body.body.len());
    let mut dictionary = None;
    for base in &decl.extends {
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            return Err(format!(
                "generic interface `{name}` has an unsupported `extends` target (only a plain interface name is supported)"
            ));
        };
        let base_name = base_ident.sym.as_str();
        let base_ty = if let Some(base_decl) = generic_interfaces.interfaces.get(base_name) {
            let reference = swc_ecma_ast::TsTypeRef {
                span: base.span,
                type_name: swc_ecma_ast::TsEntityName::Ident(base_ident.clone()),
                type_params: base.type_args.clone(),
            };
            resolve_generic_interface(
                base_name,
                base_decl,
                &reference,
                interfaces,
                generic_interfaces,
                Some(&substitution),
                in_progress,
            )?
        } else {
            if base.type_args.is_some() {
                return Err(format!(
                    "interface `{name}` supplies type arguments to non-generic base `{base_name}`"
                ));
            }
            interfaces
                .get(base_name)
                .cloned()
                .ok_or_else(|| format!("unknown base interface `{base_name}` for `{name}`"))?
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
            if fields.iter().any(|(existing, _)| existing == &field_name) {
                return Err(format!(
                    "interface `{name}` inherits field `{field_name}` from `{base_name}`, which collides with an earlier field of the same name"
                ));
            }
            fields.push((field_name, field_ty));
        }
    }
    for member in &decl.body.body {
        if let TsTypeElement::TsIndexSignature(signature) = member {
            let value = resolve_ts_type_with_substitution(
                index_signature_value(signature)?,
                &substitution,
                interfaces,
                generic_interfaces,
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
        let (key, ty, optional) = match member {
            TsTypeElement::TsPropertySignature(property) => {
                let annotation = property.type_ann.as_ref().ok_or_else(|| {
                    "generic interface property needs an explicit type annotation".to_string()
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
        let mut field_ty = resolve_ts_type_with_substitution(
            &ty,
            &substitution,
            interfaces,
            generic_interfaces,
            in_progress,
        )?;
        if optional {
            field_ty = optional_parameter_type(field_ty);
        }
        if dictionary
            .as_ref()
            .is_some_and(|element| element != &field_ty)
        {
            return Err(format!(
                "interface `{name}` property `{field_name}` does not match its index value type"
            ));
        }
        if let Some((_, existing)) = fields.iter().find(|(name, _)| name == &field_name) {
            if existing == &field_ty {
                continue;
            }
            return Err(format!(
                "interface `{name}` redeclares field `{field_name}` with an incompatible type"
            ));
        }
        fields.push((field_name, field_ty));
    }

    in_progress.pop();

    if let Some(element) = dictionary {
        Ok(HirType::Dictionary(Box::new(element)))
    } else {
        Ok(HirType::Object(fields))
    }
}

fn resolve_generic_alias(
    name: &str,
    decl: &swc_ecma_ast::TsTypeAliasDecl,
    ty_ref: &swc_ecma_ast::TsTypeRef,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    outer_substitution: Option<&HashMap<Symbol, HirType>>,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if in_progress.iter().any(|active| active == name) {
        return Err(format!(
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
        let expected = if required == parameters.len() {
            required.to_string()
        } else {
            format!("{required}..={}", parameters.len())
        };
        return Err(format!(
            "type alias `{name}` expects {expected} type argument(s), got {}",
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
                )?,
                None => lower_ts_type(argument, interfaces, generic_interfaces)?,
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
            )?
        };
        if let Some(constraint) = &parameter.constraint {
            let constraint = resolve_ts_type_with_substitution(
                constraint,
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?;
            if !type_satisfies_constraint(&concrete, &constraint) {
                return Err(format!(
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

fn type_satisfies_constraint(actual: &HirType, constraint: &HirType) -> bool {
    if matches!(actual, HirType::StrLiteral(_)) && constraint == &HirType::Str {
        return true;
    }
    if actual == constraint || constraint == &HirType::Dynamic {
        return true;
    }
    match constraint {
        HirType::Union(elements) => elements
            .iter()
            .any(|element| type_satisfies_constraint(actual, element)),
        HirType::Object(required) => required.iter().all(|(name, ty)| {
            let actual_field = match actual {
                HirType::Object(fields) => fields
                    .iter()
                    .find(|(field, _)| field == name)
                    .map(|(_, ty)| ty),
                HirType::Str | HirType::Array(_) | HirType::Tuple(_) if name == "length" => {
                    Some(&HirType::F64)
                }
                _ => None,
            };
            actual_field.is_some_and(|actual| type_satisfies_constraint(actual, ty))
        }),
        _ => false,
    }
}

/// Like `lower_ts_type`, but a bare `TsTypeRef` matching one of `Name`'s
/// type parameters resolves to the corresponding concrete `HirType`
/// instead of erroring as an unknown reference. Recurses into itself (not
/// plain `lower_ts_type`) for `T[]`/`Array<T>`/`Promise<T>`/object type
/// literal sub-parts, so a type parameter used deeper inside those still
/// gets substituted.
fn resolve_ts_type_with_substitution(
    ty: &TsType,
    substitution: &HashMap<Symbol, HirType>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if let TsType::TsTypeRef(ty_ref) = ty {
        if let swc_ecma_ast::TsEntityName::Ident(id) = &ty_ref.type_name {
            let ref_name = id.sym.as_str();
            if let Some(concrete) = substitution.get(ref_name) {
                return Ok(concrete.clone());
            }
            if let Some(decl) = generic_interfaces.interfaces.get(ref_name) {
                return resolve_generic_interface(
                    ref_name,
                    decl,
                    ty_ref,
                    interfaces,
                    generic_interfaces,
                    Some(substitution),
                    in_progress,
                );
            }
            if let Some(decl) = generic_interfaces.aliases.get(ref_name) {
                return resolve_generic_alias(
                    ref_name,
                    decl,
                    ty_ref,
                    interfaces,
                    generic_interfaces,
                    Some(substitution),
                    in_progress,
                );
            }
            if ref_name == "Record" {
                let [keys, value] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return Err("Record<K, V> requires exactly two type arguments".into());
                };
                let value = resolve_ts_type_with_substitution(
                    value,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?;
                if is_string_keyword(keys) {
                    return Ok(HirType::Dictionary(Box::new(value)));
                }
                return Ok(HirType::Object(
                    substituted_utility_keys(
                        keys,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?
                    .into_iter()
                    .map(|key| (key, value.clone()))
                    .collect(),
                ));
            }
            if ref_name == "Pick" || ref_name == "Omit" {
                let [object, keys] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return Err(format!(
                        "{ref_name}<T, K> requires exactly two type arguments"
                    ));
                };
                let object = resolve_ts_type_with_substitution(
                    object,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?;
                let keys = substituted_utility_keys(
                    keys,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?;
                return if ref_name == "Pick" {
                    pick_hir_type(object, &keys)
                } else {
                    omit_hir_type(object, &keys)
                };
            }
            if let Some(params) = &ty_ref.type_params {
                if let [elem] = params.params.as_slice() {
                    let resolved_elem = resolve_ts_type_with_substitution(
                        elem,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?;
                    match ref_name {
                        "Array" | "ReadonlyArray" => {
                            return Ok(HirType::Array(Box::new(resolved_elem)))
                        }
                        "Promise" => return Ok(HirType::Promise(Box::new(resolved_elem))),
                        "Readonly" => return Ok(resolved_elem),
                        "Awaited" => return Ok(awaited_hir_type(resolved_elem)),
                        "NonNullable" => return non_nullable_hir_type(resolved_elem),
                        "Partial" => return partial_hir_type(resolved_elem),
                        "Required" => return required_hir_type(resolved_elem),
                        _ => {}
                    }
                }
            }
        }
        return lower_ts_type(ty, interfaces, generic_interfaces);
    }

    match ty {
        TsType::TsTypePredicate(_) => Ok(HirType::Bool),
        TsType::TsParenthesizedType(parenthesized) => resolve_ts_type_with_substitution(
            &parenthesized.type_ann,
            substitution,
            interfaces,
            generic_interfaces,
            in_progress,
        ),
        TsType::TsTypeOperator(operator)
            if operator.op == swc_ecma_ast::TsTypeOperatorOp::KeyOf =>
        {
            hir_object_keys(resolve_ts_type_with_substitution(
                &operator.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?)?;
            Ok(HirType::Str)
        }
        TsType::TsTypeOperator(operator)
            if operator.op == swc_ecma_ast::TsTypeOperatorOp::ReadOnly =>
        {
            resolve_ts_type_with_substitution(
                &operator.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )
        }
        TsType::TsArrayType(arr) => {
            Ok(HirType::Array(Box::new(resolve_ts_type_with_substitution(
                &arr.elem_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?)))
        }
        TsType::TsOptionalType(optional) => Ok(HirType::Optional(Box::new(
            resolve_ts_type_with_substitution(
                &optional.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?,
        ))),
        TsType::TsTupleType(tuple) => {
            if let Some(TsType::TsRestType(rest)) =
                tuple.elem_types.last().map(|element| element.ty.as_ref())
            {
                let HirType::Array(rest) = resolve_ts_type_with_substitution(
                    &rest.type_ann,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?
                else {
                    return Err("tuple rest element needs an array type".into());
                };
                let prefix = tuple.elem_types[..tuple.elem_types.len() - 1]
                    .iter()
                    .map(|element| {
                        resolve_ts_type_with_substitution(
                            &element.ty,
                            substitution,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if prefix.iter().all(|element| element == rest.as_ref()) {
                    return Ok(HirType::Array(rest));
                }
                return Ok(HirType::Array(Box::new(HirType::Json)));
            }
            Ok(HirType::Tuple(
                tuple
                    .elem_types
                    .iter()
                    .map(|element| {
                        resolve_ts_type_with_substitution(
                            &element.ty,
                            substitution,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        TsType::TsMappedType(mapped) => {
            if mapped.name_type.is_some() {
                return Err("mapped type key remapping is not supported".into());
            }
            let constraint = mapped
                .type_param
                .constraint
                .as_ref()
                .ok_or("mapped type parameter needs a key constraint")?;
            let keys = substituted_utility_keys(
                constraint,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?;
            let value = mapped
                .type_ann
                .as_ref()
                .ok_or("mapped type needs a value annotation")?;
            let optional = matches!(
                mapped.optional,
                Some(swc_ecma_ast::TruePlusMinus::True | swc_ecma_ast::TruePlusMinus::Plus)
            );
            let mut fields = Vec::with_capacity(keys.len());
            for key in keys {
                let mut nested = substitution.clone();
                nested.insert(
                    mapped.type_param.name.sym.to_string(),
                    HirType::StrLiteral(key.clone()),
                );
                let mut value = resolve_ts_type_with_substitution(
                    value,
                    &nested,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?;
                if optional {
                    value = optional_parameter_type(value);
                }
                fields.push((key, value));
            }
            Ok(HirType::Object(fields))
        }
        TsType::TsIndexedAccessType(indexed) => {
            let object = resolve_ts_type_with_substitution(
                &indexed.obj_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?;
            if object == HirType::Dynamic {
                return Ok(HirType::Dynamic);
            }
            indexed_access_hir_type(
                object,
                &substituted_utility_keys(
                &indexed.index_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
                )?,
            )
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            let mut elements = Vec::new();
            for element in &union.types {
                let element = resolve_ts_type_with_substitution(
                    element,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?;
                if !elements.contains(&element) {
                    elements.push(element);
                }
            }
            if let [element] = elements.as_slice() {
                return Ok(element.clone());
            }
            if elements.len() == 3
                && elements.contains(&HirType::Null)
                && elements.contains(&HirType::Undefined)
            {
                if let Some(payload) = elements
                    .iter()
                    .find(|element| !matches!(element, HirType::Null | HirType::Undefined))
                {
                    return Ok(HirType::Nullish(Box::new(payload.clone())));
                }
            }
            if elements.len() == 2 {
                if let Some(payload) = elements
                    .iter()
                    .find(|element| **element != HirType::Undefined)
                    .filter(|_| elements.contains(&HirType::Undefined))
                {
                    return Ok(HirType::Optional(Box::new(payload.clone())));
                }
                if let Some(payload) = elements
                    .iter()
                    .find(|element| **element != HirType::Null)
                    .filter(|_| elements.contains(&HirType::Null))
                {
                    return Ok(HirType::Nullable(Box::new(payload.clone())));
                }
            }
            Ok(HirType::Union(elements))
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => {
            let elements = intersection
                .types
                .iter()
                .map(|element| {
                    resolve_ts_type_with_substitution(
                        element,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let Some(first) = elements.first() else {
                return Err("empty intersection type is not supported".into());
            };
            if elements.iter().all(|element| element == first) {
                return Ok(first.clone());
            }
            if elements
                .iter()
                .all(|element| matches!(element, HirType::Object(_)))
            {
                let mut fields = Vec::<(Symbol, HirType)>::new();
                for element in elements {
                    let HirType::Object(element_fields) = element else {
                        unreachable!()
                    };
                    for (name, ty) in element_fields {
                        if let Some((_, existing)) =
                            fields.iter().find(|(existing, _)| existing == &name)
                        {
                            if existing != &ty {
                                return Err(format!(
                                    "intersection field `{name}` has conflicting types"
                                ));
                            }
                        } else {
                            fields.push((name, ty));
                        }
                    }
                }
                return Ok(HirType::Object(fields));
            }
            Err(format!("unsupported intersection type {elements:?}"))
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            if function.type_params.is_some() {
                return Ok(HirType::Dynamic);
            }
            let mut params = Vec::new();
            let mut optional = Vec::new();
            let mut rest = None;
            for (index, parameter) in function.params.iter().enumerate() {
                match parameter {
                    TsFnParam::Ident(parameter) => {
                        let annotation = parameter.type_ann.as_ref().ok_or_else(|| {
                            format!(
                                "function parameter `{}` needs a type annotation",
                                parameter.id.sym
                            )
                        })?;
                        let mut ty = resolve_ts_type_with_substitution(
                            &annotation.type_ann,
                            substitution,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        )?;
                        if parameter.id.optional {
                            ty = optional_parameter_type(ty);
                        }
                        params.push(ty);
                        optional.push(parameter.id.optional);
                    }
                    TsFnParam::Rest(parameter) if index + 1 == function.params.len() => {
                        let annotation = parameter
                            .type_ann
                            .as_ref()
                            .ok_or("function rest parameter needs an array type annotation")?;
                        let lowered = resolve_ts_type_with_substitution(
                            &annotation.type_ann,
                            substitution,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        )?;
                        let HirType::Array(element) = lowered else {
                            return Err(
                                "function rest parameter needs an array type annotation".into()
                            );
                        };
                        rest = Some(element);
                    }
                    TsFnParam::Rest(_) => return Err("function rest parameter must be last".into()),
                    _ => {
                        return Err(
                            "function types only support identifier and trailing rest parameters"
                                .into(),
                        )
                    }
                }
            }
            let ret = resolve_ts_type_with_substitution(
                &function.type_ann.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?;
            Ok(if rest.is_some() || optional.iter().any(|value| *value) {
                HirType::CallableFunction(
                    params,
                    optional_parameter_mask(&optional),
                    rest,
                    Box::new(ret),
                )
            } else {
                HirType::Function(params, Box::new(ret))
            })
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsConstructorType(constructor)) => {
            resolve_ts_type_with_substitution(
                &TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(
                    swc_ecma_ast::TsFnType {
                        span: constructor.span,
                        params: constructor.params.clone(),
                        type_params: constructor.type_params.clone(),
                        type_ann: constructor.type_ann.clone(),
                    },
                )),
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )
        }
        TsType::TsTypeLit(type_lit) => {
            if let Some(signature) = type_literal_index_signature(type_lit)? {
                let value = resolve_ts_type_with_substitution(
                    index_signature_value(signature)?,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?;
                for member in &type_lit.members {
                    match member {
                        TsTypeElement::TsIndexSignature(_) => {}
                        TsTypeElement::TsPropertySignature(property) => {
                            let annotation = property.type_ann.as_ref().ok_or_else(|| {
                                "dictionary property needs an explicit type annotation".to_string()
                            })?;
                            let field = resolve_ts_type_with_substitution(
                                &annotation.type_ann,
                                substitution,
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
                return Ok(HirType::Dictionary(Box::new(value)));
            }
            let fields = type_lit
                .members
                .iter()
                .map(|member| {
                    let (key, ty, optional) = match member {
                        TsTypeElement::TsPropertySignature(property) => {
                            let annotation = property.type_ann.as_ref().ok_or_else(|| {
                                "object property needs an explicit type annotation".to_string()
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
                        _ => return Err("object types only support properties and methods".into()),
                    };
                    let name = match key {
                        Expr::Ident(ident) => ident.sym.to_string(),
                        _ => return Err("unsupported object type literal key".to_string()),
                    };
                    let mut field_ty = resolve_ts_type_with_substitution(
                        &ty,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?;
                    if optional {
                        field_ty = optional_parameter_type(field_ty);
                    }
                    Ok((name, field_ty))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(HirType::Object(fields))
        }
        other => lower_ts_type(other, interfaces, generic_interfaces),
    }
}
