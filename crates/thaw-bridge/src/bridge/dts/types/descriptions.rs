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

