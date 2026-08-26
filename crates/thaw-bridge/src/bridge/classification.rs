/// Classifies a whole function signature: `FastPath` only if *every*
/// parameter and the return type are `DtsType::Native` (see
/// docs/design/bridge.md section 4.2 for why partial native/dynamic
/// signatures aren't supported).
pub fn classify(func: &DtsFunction) -> Classification {
    let mut params = Vec::with_capacity(func.params.len());
    for (name, ty) in &func.params {
        match ty {
            DtsType::Native(hir_ty) => params.push(hir_ty.clone()),
            DtsType::Unsupported(reason) => {
                return Classification::Fallback {
                    function: func.name.clone(),
                    reason: format!("parameter `{name}`: {reason}"),
                }
            }
        }
    }

    let ret = match &func.ret {
        DtsType::Native(hir_ty) => hir_ty.clone(),
        DtsType::Unsupported(reason) => {
            return Classification::Fallback {
                function: func.name.clone(),
                reason: format!("return type: {reason}"),
            }
        }
    };

    let variadic = match &func.rest_param {
        None => None,
        Some((_, DtsType::Native(ty))) if supports_variadic_element(ty) => Some(ty.clone()),
        Some((name, DtsType::Native(other))) => {
            return Classification::Fallback {
                function: func.name.clone(),
                reason: format!(
                "rest parameter `{name}` has unsupported native variadic element layout {other:?}"
            ),
            }
        }
        Some((name, DtsType::Unsupported(reason))) => {
            return Classification::Fallback {
                function: func.name.clone(),
                reason: format!("rest parameter `{name}`: {reason}"),
            }
        }
    };

    Classification::FastPath(Box::new(FfiSignature {
        symbol: func.name.clone(),
        params,
        variadic,
        variadic_abi: thaw_hir::FfiVariadicAbi::Native,
        ret,
        error_abi: FfiErrorAbi::Direct,
        return_ownership: FfiOwnership::Borrowed,
        error_ownership: FfiOwnership::Borrowed,
        param_string_abis: vec![FfiStringAbi::NullTerminated; func.params.len()],
        return_string_abi: FfiStringAbi::NullTerminated,
        calling_convention: FfiCallingConvention::C,
        aggregate_return_abi: FfiAggregateAbi::Internal,
        aggregate_return_layout: None,
    }))
}

fn supports_variadic_element(ty: &HirType) -> bool {
    match ty {
        HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue => true,
        HirType::Array(element) => matches!(
            element.as_ref(),
            HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue
        ),
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            supports_variadic_element(payload)
        }
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| supports_variadic_element(field)),
        _ => false,
    }
}

fn classify_variadic_ts_type(
    ty: &TsType,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsType {
    if let TsType::TsParenthesizedType(parenthesized) = ty {
        return classify_variadic_ts_type(&parenthesized.type_ann, interfaces, generic_interfaces);
    }
    if let TsType::TsArrayType(array) = ty {
        return match classify_ts_type(&array.elem_type, interfaces, generic_interfaces) {
            DtsType::Native(
                element @ (HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue),
            ) => DtsType::Native(HirType::Array(Box::new(element))),
            DtsType::Native(other) => DtsType::Unsupported(format!(
                "variadic array element type {other:?} requires explicit marshalling"
            )),
            unsupported => unsupported,
        };
    }
    let TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) = ty
    else {
        return classify_ts_type(ty, interfaces, generic_interfaces);
    };
    let mut payload = None;
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
            other => match classify_ts_type(other, interfaces, generic_interfaces) {
                DtsType::Native(ty) if payload.is_none() => payload = Some(ty),
                DtsType::Native(_) => {
                    return DtsType::Unsupported(
                        "variadic tagged union requires exactly one payload type".into(),
                    )
                }
                unsupported => return unsupported,
            },
        }
    }
    let Some(payload) = payload else {
        return DtsType::Unsupported("variadic tagged union has no payload type".into());
    };
    match (has_null, has_undefined) {
        (false, true) => DtsType::Native(HirType::Optional(Box::new(payload))),
        (true, false) => DtsType::Native(HirType::Nullable(Box::new(payload))),
        (true, true) => DtsType::Native(HirType::Nullish(Box::new(payload))),
        (false, false) => DtsType::Unsupported(
            "variadic union must include null or undefined in addition to its payload".into(),
        ),
    }
}

/// Classifies every function in `functions`, but only once per distinct
/// name: a `.d.ts` overload set (multiple `declare function foo(...)`
/// signatures sharing a name -- common in real npm packages, e.g. `ms`'s
/// `ms(value: number, options?): string` / `ms(value: string): number`)
/// can't become a single native `extern "C"` symbol the way Fast path
/// needs, even when one individual overload would classify Fast path on
/// its own. So a name with more than one signature always falls back as
/// a whole; Fallback's `callDynamic` doesn't care about a fixed shape,
/// so the real JS function is free to handle whatever overload logic it
/// wants. A name with exactly one signature classifies exactly as
/// `classify` would.
///
/// Found necessary by running a real overloaded package (`ms`) through
/// `generate_shim`: without this, an overload set whose members classify
/// differently produced two separate, name-colliding top-level
/// declarations, and thaw-hir/codegen silently picked whichever was
/// declared last -- no error, and the outcome depended entirely on
/// declaration order rather than being a real decision.
pub fn classify_all(functions: &[DtsFunction]) -> Vec<(String, Classification)> {
    let mut by_name: Vec<(&str, Vec<&DtsFunction>)> = Vec::new();
    for func in functions {
        match by_name.iter_mut().find(|(name, _)| *name == func.name) {
            Some((_, group)) => group.push(func),
            None => by_name.push((&func.name, vec![func])),
        }
    }

    by_name
        .into_iter()
        .map(|(name, group)| {
            let classification = match group.as_slice() {
                [only] => classify(only),
                overloads => Classification::Fallback {
                    function: name.to_string(),
                    reason: format!(
                        "`{name}` has {} overloaded signatures in the .d.ts; Fast path needs exactly one",
                        overloads.len()
                    ),
                },
            };
            (name.to_string(), classification)
        })
        .collect()
}

/// Renders a `HirType` back into the TS syntax `thaw_hir::lower::lower_ts_type`
/// accepts, for `generate_shim`'s ambient declarations. Only ever called on
/// types that actually came from a successful classification (primitives,
/// `number[]`, and flat/nested objects), so the `Json`/`Union`/`Dynamic`
/// arms are just defensive completeness, not expected to be exercised.
fn render_ts_type(ty: &HirType) -> String {
    match ty {
        HirType::F64 | HirType::I64 => "number".to_string(),
        HirType::Bool => "boolean".to_string(),
        HirType::Undefined => "undefined".to_string(),
        HirType::Null => "null".to_string(),
        HirType::Void => "void".to_string(),
        HirType::Str => "string".to_string(),
        HirType::Json => "Json".to_string(),
        HirType::Dictionary(element) => {
            format!("{{ [key: string]: {} }}", render_ts_type(element))
        }
        HirType::JsValue => "JsValue".to_string(),
        HirType::Array(elem) => format!("{}[]", render_ts_type(elem)),
        HirType::Tuple(elements) => format!(
            "[{}]",
            elements
                .iter()
                .map(render_ts_type)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        HirType::Promise(inner) => format!("Promise<{}>", render_ts_type(inner)),
        HirType::Optional(inner) => format!("{} | undefined", render_ts_type(inner)),
        HirType::Nullable(inner) => format!("{} | null", render_ts_type(inner)),
        HirType::Nullish(inner) => {
            format!("{} | null | undefined", render_ts_type(inner))
        }
        HirType::Object(fields) => {
            let rendered = fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", render_ts_type(ty)))
                .collect::<Vec<_>>()
                .join("; ");
            format!("{{ {rendered} }}")
        }
        HirType::Function(params, ret) => {
            let params = params
                .iter()
                .enumerate()
                .map(|(index, ty)| format!("arg{index}: {}", render_ts_type(ty)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({params}) => {}", render_ts_type(ret))
        }
        HirType::CallableFunction(params, optional, rest, ret) => {
            let mut params = params
                .iter()
                .enumerate()
                .map(|(index, ty)| {
                    format!(
                        "arg{index}{}: {}",
                        if optional.contains(index) { "?" } else { "" },
                        render_ts_type(ty)
                    )
                })
                .collect::<Vec<_>>();
            if let Some(rest) = rest {
                params.push(format!("...rest: {}[]", render_ts_type(rest)));
            }
            format!("({}) => {}", params.join(", "), render_ts_type(ret))
        }
        HirType::Union(_) | HirType::Dynamic => "any".to_string(),
    }
}

