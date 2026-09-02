/// Classifies a whole function signature: `FastPath` only if *every*
/// parameter and the return type are `DtsType::Native` (see
/// docs/design/bridge.md section 4.2 for why partial native/dynamic
/// signatures aren't supported).
pub fn classify(func: &DtsFunction) -> Classification {
    let mut params = Vec::with_capacity(func.params.len());
    for (name, ty) in &func.params {
        match ty {
            DtsType::Native(HirType::Union(_)) => {
                return Classification::Fallback {
                    function: func.name.clone(),
                    reason: format!(
                        "parameter `{name}` uses a tagged union without an explicit C ABI"
                    ),
                }
            }
            DtsType::Native(hir_ty) if supports_direct_ffi_collections(hir_ty) => {
                params.push(hir_ty.clone())
            }
            DtsType::Native(hir_ty) => {
                return Classification::Fallback {
                    function: func.name.clone(),
                    reason: format!(
                        "parameter `{name}` has unsupported aggregate collection layout {hir_ty:?}"
                    ),
                }
            }
            DtsType::Unsupported(reason) => {
                return Classification::Fallback {
                    function: func.name.clone(),
                    reason: format!("parameter `{name}`: {reason}"),
                }
            }
        }
    }

    let ret = match &func.ret {
        DtsType::Native(HirType::Union(_)) => {
            return Classification::Fallback {
                function: func.name.clone(),
                reason: "return type uses a tagged union without an explicit C ABI".into(),
            }
        }
        DtsType::Native(hir_ty) if supports_direct_ffi_collections(hir_ty) => hir_ty.clone(),
        DtsType::Native(hir_ty) => {
            return Classification::Fallback {
                function: func.name.clone(),
                reason: format!("return type has unsupported aggregate collection layout {hir_ty:?}"),
            }
        }
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

fn supports_direct_ffi_collections(ty: &HirType) -> bool {
    match ty {
        // A source union has an internal tagged layout, but an arbitrary C
        // symbol has no matching discriminator ABI unless one is declared.
        HirType::Union(_) => false,
        HirType::Array(element) => matches!(
            element.as_ref(),
            HirType::F64 | HirType::Str | HirType::Bool | HirType::JsValue
        ),
        HirType::Tuple(elements) => elements.iter().all(supports_direct_ffi_collections),
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| supports_direct_ffi_collections(field)),
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            supports_direct_ffi_collections(payload)
        }
        _ => true,
    }
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
        HirType::Map(key, value) => {
            format!("Map<{}, {}>", render_ts_type(key), render_ts_type(value))
        }
        HirType::Set(element) => format!("Set<{}>", render_ts_type(element)),
        HirType::Union(_) | HirType::Dynamic => "any".to_string(),
    }
}
