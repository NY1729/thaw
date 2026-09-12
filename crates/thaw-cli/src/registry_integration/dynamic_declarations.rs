/// A valid JS/Thaw identifier fragment from an arbitrary package name --
/// `@hapi/hoek` -> `_hapi_hoek`. Used to build a package-qualified alias
/// identifier (`generate_registry_shims`'s collision resolution); doesn't
/// need to be reversible or collision-free against unrelated packages
/// with a similar sanitized form, since it's always combined with the
/// original function name too.
fn sanitize_identifier(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// A `DtsFunction`'s declared parameter types as plain `HirType`s, for
/// `overload_type_score` (`class_methods.rs`) to compare against a real
/// call site's actual argument types. Anything that isn't `DtsType::
/// Native` -- including a reference to one of the function's own generic
/// type parameters, which doesn't classify as `Native` -- widens to
/// `Json`, matching the "unclassifiable stays Json" convention used
/// everywhere else in this file; `Json` scores as a universal-but-weak
/// match, so a generic overload only loses to a more specifically-typed
/// sibling when one genuinely fits better, never disqualified outright.
fn dts_function_param_hir_types(function: &thaw_bridge::DtsFunction) -> Vec<thaw_hir::HirType> {
    function
        .params
        .iter()
        .map(|(_, ty)| match ty {
            thaw_bridge::DtsType::Native(ty) => ty.clone(),
            // A bare `undefined`/`null` literal type used purely to steer
            // *real* TypeScript's own overload resolution -- real
            // example: uuid's `v4(options?, buf?: undefined, offset?):
            // string`, where `buf`'s declared type exists only to
            // distinguish this overload from its sibling generic
            // buffer-output one (`v4<TBuf>(options, buf: TBuf, offset?):
            // TBuf`). `classify_ts_type` widens a bare `undefined`/`null`
            // keyword type to `Unsupported` the same as any other
            // unclassifiable type (correct for *rendering* -- it's still
            // a callable, Json-typed parameter at runtime, see
            // `typed_dynamic_declaration`'s own matching comment), but
            // scoring it as a weak Json match here would make it
            // indistinguishable from the sibling overload's *own*
            // unresolvable generic-typed `buf` -- letting a real,
            // concrete argument (an actual buffer) tie against this
            // phantom parameter instead of correctly losing to it.
            // Recovering the literal shape here (for scoring only, not
            // rendering) lets `overload_type_score`'s existing exact-type
            // fallback arm correctly disqualify this overload for any
            // argument that isn't itself undefined/null.
            thaw_bridge::DtsType::Unsupported(message)
                if message == "`undefined` is not supported" =>
            {
                thaw_hir::HirType::Undefined
            }
            thaw_bridge::DtsType::Unsupported(message) if message == "`null` is not supported" => {
                thaw_hir::HirType::Null
            }
            _ => thaw_hir::HirType::Json,
        })
        .collect()
}

fn render_dynamic_type(ty: &thaw_hir::HirType) -> Option<String> {
    match ty {
        thaw_hir::HirType::F64 => Some("number".into()),
        thaw_hir::HirType::Str => Some("string".into()),
        thaw_hir::HirType::Bool => Some("boolean".into()),
        thaw_hir::HirType::Void => Some("void".into()),
        thaw_hir::HirType::Json => Some("Json".into()),
        thaw_hir::HirType::JsValue => Some("JsValue".into()),
        thaw_hir::HirType::Bytes => Some("Uint8Array".into()),
        thaw_hir::HirType::Promise(payload) => {
            render_dynamic_type(payload).map(|payload| format!("Promise<{payload}>") )
        }
        thaw_hir::HirType::Optional(payload) => {
            render_dynamic_type(payload).map(|rendered| {
                if matches!(
                    payload.as_ref(),
                    thaw_hir::HirType::Function(..)
                        | thaw_hir::HirType::CallableFunction(..)
                ) {
                    format!("({rendered}) | undefined")
                } else {
                    format!("{rendered} | undefined")
                }
            })
        }
        thaw_hir::HirType::Nullable(payload) => {
            render_dynamic_type(payload).map(|rendered| {
                if matches!(
                    payload.as_ref(),
                    thaw_hir::HirType::Function(..)
                        | thaw_hir::HirType::CallableFunction(..)
                ) {
                    format!("({rendered}) | null")
                } else {
                    format!("{rendered} | null")
                }
            })
        }
        thaw_hir::HirType::Nullish(payload) => render_dynamic_type(payload).map(|rendered| {
            if matches!(
                payload.as_ref(),
                thaw_hir::HirType::Function(..) | thaw_hir::HirType::CallableFunction(..)
            ) {
                format!("({rendered}) | null | undefined")
            } else {
                format!("{rendered} | null | undefined")
            }
        }),
        thaw_hir::HirType::Union(elements) => elements
            .iter()
            .map(|element| {
                render_dynamic_type(element).map(|rendered| {
                    if matches!(element, thaw_hir::HirType::Function(..) | thaw_hir::HirType::CallableFunction(..)) {
                        format!("({rendered})")
                    } else {
                        rendered
                    }
                })
            })
            .collect::<Option<Vec<_>>>()
            .map(|elements| elements.join(" | ")),
        thaw_hir::HirType::Array(element) => {
            render_dynamic_type(element).map(|rendered| {
                if matches!(
                    element.as_ref(),
                    thaw_hir::HirType::Optional(_)
                        | thaw_hir::HirType::Nullable(_)
                        | thaw_hir::HirType::Nullish(_)
                        | thaw_hir::HirType::Function(_, _)
                ) {
                    format!("({rendered})[]")
                } else {
                    format!("{rendered}[]")
                }
            })
        }
        thaw_hir::HirType::Dictionary(element) => {
            render_dynamic_type(element).map(|element| format!("{{ [key: string]: {element} }}"))
        }
        thaw_hir::HirType::Tuple(elements) => elements
            .iter()
            .map(render_dynamic_type)
            .collect::<Option<Vec<_>>>()
            .map(|elements| format!("[{}]", elements.join(", "))),
        thaw_hir::HirType::Object(fields) => fields
            .iter()
            .map(|(name, ty)| render_dynamic_type(ty).map(|ty| format!("{name}: {ty}")))
            .collect::<Option<Vec<_>>>()
            .map(|fields| format!("{{ {} }}", fields.join("; "))),
        thaw_hir::HirType::Function(params, ret) => {
            let params = params
                .iter()
                .enumerate()
                .map(|(index, ty)| render_dynamic_type(ty).map(|ty| format!("arg{index}: {ty}")))
                .collect::<Option<Vec<_>>>()?;
            let ret = render_dynamic_type(ret)?;
            Some(format!("({}) => {ret}", params.join(", ")))
        }
        thaw_hir::HirType::CallableFunction(params, optional, rest, ret) => {
            let mut params = params
                .iter()
                .enumerate()
                .map(|(index, ty)| {
                    render_dynamic_type(ty).map(|ty| {
                        format!(
                            "arg{index}{}: {ty}",
                            if optional.contains(index) { "?" } else { "" }
                        )
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            if let Some(rest) = rest {
                params.push(format!("...args: {}[]", render_dynamic_type(rest)?));
            }
            let ret = render_dynamic_type(ret)?;
            Some(format!("({}) => {ret}", params.join(", ")))
        }
        _ => None,
    }
}

fn typed_dynamic_callable_adapter(
    encoded: &str,
    target: String,
    mut declarations: String,
    params: &[(String, String)],
    required_params: usize,
    napi: bool,
    ret: &thaw_hir::HirType,
) -> Option<(String, String)> {
    let (callback_params, optional, callback_ret) = match ret {
        thaw_hir::HirType::Function(params, ret) => {
            (params, thaw_hir::HirOptionalMask::default(), ret.as_ref())
        }
        thaw_hir::HirType::CallableFunction(params, optional, None, ret) => {
            (params, optional.clone(), ret.as_ref())
        }
        _ => return Some((target, declarations)),
    };
    let (convert, handle_result) = match callback_ret {
        thaw_hir::HirType::Str => ("String", false),
        thaw_hir::HirType::F64 => ("Number", false),
        thaw_hir::HirType::Bool => ("Boolean", false),
        thaw_hir::HirType::Json => ("", false),
        thaw_hir::HirType::JsValue if !napi => ("", true),
        _ => return Some((target, declarations)),
    };
    let callback_types = callback_params
        .iter()
        .map(render_dynamic_type)
        .collect::<Option<Vec<_>>>()?;
    let adapter = format!("__thaw_typed_callable_{encoded}");
    let outer_params = params
        .iter()
        .enumerate()
        .map(|(index, (name, ty))| {
            format!(
                "{name}{}: {ty}",
                if index >= required_params { "?" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let outer_args = params
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let callback_signature = callback_types
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            format!(
                "arg{index}{}: {ty}",
                if optional.contains(index) { "?" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let callback_type = render_dynamic_type(ret)?;
    let call_value = if handle_result {
        "callDynamicValueHandle"
    } else if napi {
        "callNativeAddonValue"
    } else {
        "callDynamicValue"
    };
    let required = (0..callback_params.len())
        .take_while(|index| !optional.contains(*index))
        .count();
    declarations.push_str(&format!(
        "function {adapter}({outer_params}): {callback_type} {{\n    const callable: JsValue = {target}({outer_args});\n    const invoke: {callback_type} = ({callback_signature}): {} => {{\n",
        render_dynamic_type(callback_ret)?
    ));
    for arity in (required + 1..=callback_params.len()).rev() {
        let condition = format!("arg{} !== undefined", arity - 1);
        let args = (0..arity)
            .map(|index| format!("arg{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let call = format!("{call_value}(callable, JSON.parse(JSON.stringify([{args}])))");
        declarations.push_str(&format!(
            "        if ({condition}) return {convert}({call});\n"
        ));
    }
    let args = (0..required)
        .map(|index| format!("arg{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let json_args = if required == 0 {
        "JSON.parse(\"[]\")".to_string()
    } else {
        format!("JSON.parse(JSON.stringify([{args}]))")
    };
    declarations.push_str(&format!(
        "        return {convert}({call_value}(callable, {json_args}));\n    }};\n    return invoke;\n}}\n"
    ));
    Some((adapter, declarations))
}

fn supported_json_collection_element(ty: &thaw_hir::HirType) -> bool {
    match ty {
        thaw_hir::HirType::F64
        | thaw_hir::HirType::Str
        | thaw_hir::HirType::Bool
        | thaw_hir::HirType::Json => true,
        thaw_hir::HirType::Optional(payload)
        | thaw_hir::HirType::Nullable(payload)
        | thaw_hir::HirType::Nullish(payload) => supported_json_collection_element(payload),
        thaw_hir::HirType::Array(element) => supported_json_collection_element(element),
        thaw_hir::HirType::Tuple(elements) => {
            elements.iter().all(supported_json_collection_element)
        }
        thaw_hir::HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| supported_json_collection_element(field)),
        _ => false,
    }
}

fn generic_rest_array_result(
    generic: &thaw_bridge::DtsGenericFunction,
) -> Option<(&str, bool)> {
    generic.type_params.iter().find_map(|(parameter, _)| {
        if generic.return_type == format!("{parameter}[]")
            || generic.return_type == format!("Array<{parameter}>")
        {
            Some((parameter.as_str(), false))
        } else if generic.return_type == format!("Array<{parameter}[keyof {parameter}]>") {
            Some((parameter.as_str(), true))
        } else {
            None
        }
    })
}

/// `typed_dynamic_declaration`'s handling for a Fallback function whose
/// `.d.ts` signature ends in a rest parameter (`...inputs: T[]`, real
/// example: `clsx(...inputs: ClassValue[]): string`). There's no fixed
/// arity to declare an extern signature for, so this leans on the same
/// call-site arity *observation* `generate_napi_class_method_overloads`
/// already uses for a native addon method's own rest parameters: one
/// extern declaration per distinct total argument count actually seen in
/// the user's own call expressions, dispatched by the wrapper (a real,
/// non-extern function, so its own `...inputs: Json[]` rest parameter
/// gets the ordinary `native_rest` treatment and really collects every
/// loose trailing argument into one array) branching on `inputs.length` --
/// deliberately not the `!== undefined` idiom the optional-parameter
/// wrapper uses, since a plain length comparison needs no type-narrowing
/// at all. Every rest slot -- and any unclassifiable fixed parameter, for
/// the same reason as `typed_dynamic_declaration`'s main path -- is
/// rendered as `Json`: the real element type (`ClassValue` above) is
/// often a recursive/aggregate union with no direct FFI ABI anyway, and
/// the dynamic call already marshals every argument through JSON.
fn typed_dynamic_rest_declaration(
    function: &thaw_bridge::DtsFunction,
    napi: bool,
    encoded: &str,
    base_symbol: &str,
    observed_call_arities: &std::collections::BTreeSet<usize>,
) -> Option<(String, String)> {
    let fixed_params = function
        .params
        .iter()
        .map(|(name, ty)| match ty {
            thaw_bridge::DtsType::Native(ty) => {
                render_dynamic_type(ty).map(|ty| (name.clone(), ty))
            }
            thaw_bridge::DtsType::Unsupported(_) => Some((name.clone(), "Json".to_string())),
        })
        .collect::<Option<Vec<_>>>()?;
    let ret_ty = match &function.ret {
        thaw_bridge::DtsType::Native(ty) => ty.clone(),
        thaw_bridge::DtsType::Unsupported(reason)
            if reason == "`any` is not supported" || reason == "`unknown` is not supported" =>
        {
            thaw_hir::HirType::Json
        }
        thaw_bridge::DtsType::Unsupported(_) if function.generic.is_some() => {
            thaw_hir::HirType::JsValue
        }
        thaw_bridge::DtsType::Unsupported(_) => return None,
    };
    let ret = if matches!(
        &ret_ty,
        thaw_hir::HirType::Function(_, _) | thaw_hir::HirType::CallableFunction(..)
    ) {
        "JsValue".to_string()
    } else {
        render_dynamic_type(&ret_ty)?
    };

    let fixed_count = fixed_params.len();
    let mut arities: Vec<usize> = observed_call_arities
        .iter()
        .copied()
        .filter(|count| *count >= function.required_params)
        .collect();
    if arities.is_empty() {
        // No statically-visible call to key arities off of (e.g. every
        // call site spreads a runtime-length array) -- fall back to just
        // the all-fixed, zero-rest-element shape so at least that much
        // stays callable.
        arities.push(fixed_count);
    }
    arities.sort_unstable();
    arities.dedup();

    let render_fixed_params = |total: usize| {
        fixed_params[..total.min(fixed_count)]
            .iter()
            .map(|(name, ty)| format!("{name}: {ty}"))
            .collect::<Vec<_>>()
    };
    let render_arguments = |total: usize| {
        let mut args = fixed_params[..total.min(fixed_count)]
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        let rest_name = function
            .rest_param
            .as_ref()
            .map(|(name, _)| name.as_str())
            .unwrap_or("__thaw_rest");
        for index in 0..total.saturating_sub(fixed_count) {
            args.push(format!("{rest_name}[{index}]"));
        }
        args
    };

    let mut declarations = String::new();
    let generic_array = function
        .generic
        .as_ref()
        .and_then(generic_rest_array_result)
        .filter(|(parameter, object_values)| {
            function
                .generic
                .as_ref()
                .expect("generic_array requires generic metadata")
                .param_types[..fixed_count]
                .iter()
                .any(|ty| {
                    if *object_values {
                        ty == *parameter || ty.starts_with(&format!("{parameter} |"))
                    } else {
                        ty == &format!("{parameter}[]")
                            || ty.contains(&format!("<{parameter}>"))
                    }
                })
        });
    for &total in &arities {
        let mut params_rendered = render_fixed_params(total);
        for index in 0..total.saturating_sub(fixed_count) {
            params_rendered.push(format!("__thaw_rest_{index}: Json"));
        }
        declarations.push_str(&format!(
            "declare function {base_symbol}__arity_{total}({}): {ret};\n",
            params_rendered.join(", ")
        ));
        if let Some((parameter, object_values)) = generic_array {
            let mut specialized = function
                .generic
                .as_ref()
                .expect("generic_array requires generic metadata")
                .param_types[..fixed_count]
                .iter()
                .enumerate()
                .map(|(index, ty)| {
                    let ty = if object_values
                        && (ty == parameter || ty.starts_with(&format!("{parameter} |")))
                    {
                        "Json".into()
                    } else if !object_values
                        && (ty == &format!("{parameter}[]")
                            || ty.contains(&format!("<{parameter}>")))
                    {
                        format!("{parameter}[]")
                    } else {
                        "Json".into()
                    };
                    format!("{}: {ty}", fixed_params[index].0)
                })
                .collect::<Vec<_>>();
            for index in 0..total.saturating_sub(fixed_count) {
                specialized.push(format!("__thaw_rest_{index}: Json"));
            }
            declarations.push_str(&format!(
                "declare function {base_symbol}__generic_rest__arity_{total}<{}>({}): {parameter}[];\n",
                parameter,
                specialized.join(", ")
            ));
        }
    }

    let wrapper = format!(
        "__thaw_typed_wrapper_{}_{}",
        if napi { "napi" } else { "js" },
        encoded
    );
    let rest_name = function
        .rest_param
        .as_ref()
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| "__thaw_rest".to_string());
    let fixed_wrapper_params = fixed_params
        .iter()
        .enumerate()
        .map(|(index, (name, ty))| {
            format!(
                "{name}{}: {ty}",
                if index >= function.required_params {
                    "?"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let wrapper_params = if fixed_wrapper_params.is_empty() {
        format!("...{rest_name}: Json[]")
    } else {
        format!("{fixed_wrapper_params}, ...{rest_name}: Json[]")
    };
    // A `void`-returning extern call can't itself be the operand of
    // `return` (thaw-hir rejects `return f();` for a `void`-declared
    // `f`, only a bare `return;`), so those need their call and return
    // as separate statements -- real example: lodash's `noop(): void`.
    let call_and_return = |call: String| -> String {
        if ret == "void" {
            format!("{call}; return;")
        } else {
            format!("return {call};")
        }
    };
    declarations.push_str(&format!("function {wrapper}({wrapper_params}): {ret} {{\n"));
    for &total in arities.iter().rev() {
        let rest_count = total.saturating_sub(fixed_count);
        let arguments = render_arguments(total).join(", ");
        let call = call_and_return(format!("{base_symbol}__arity_{total}({arguments})"));
        declarations.push_str(&format!(
            "    if ({rest_name}.length === {rest_count}) {{ {call} }}\n"
        ));
    }
    let smallest = *arities.first().expect("arities always has at least one entry");
    let arguments = render_arguments(smallest).join(", ");
    let call = call_and_return(format!("{base_symbol}__arity_{smallest}({arguments})"));
    declarations.push_str(&format!("    {call}\n}}\n"));

    typed_dynamic_callable_adapter(
        encoded,
        wrapper,
        declarations,
        &fixed_params,
        function.required_params,
        napi,
        &ret_ty,
    )
}

/// Whether `text` (a `describe_ts_type` rendering of a generic type
/// parameter's `extends` constraint) is safe to splice verbatim into this
/// declaration's own generated source. `describe_ts_type` is built for a
/// human-readable Fallback *reason*, not re-parseable syntax -- besides
/// its outright placeholder strings (`"a conditional type"`, `"{ ... }"`
/// for a non-empty object literal, `"<qualified name>"`, ...), even a
/// *syntactically* valid rendering like `List<T>` or a bare interface
/// name refers to a type that exists in the original `.d.ts`'s own scope,
/// not in this shim's -- nothing here carries the rest of that package's
/// type aliases/interfaces along with it. Restricting to plain TS
/// keywords sidesteps both problems at once: every one of them means
/// exactly what it says with no external name to resolve, and
/// `describe_ts_type` only ever renders one verbatim (`keyword_name`).
/// Whether a callback-shaped param type string (e.g. `(value: T) =>
/// TResult`, from a generic interface method's `func: (value: T) =>
/// TResult` parameter) mentions any of the function's own generic type
/// parameters anywhere inside it -- a bare top-level `value: T` parameter
/// is proven to work (`specializes_generic_dynamic_ambient_arguments_per_call`),
/// but a type parameter used *inside* a nested callback signature (its
/// own parameter or return type) isn't substituted the same way and
/// produces an unresolvable bare name once compiled, so this rejects the
/// whole declaration rather than emit one that can't build. Checked as a
/// substring with word boundaries (not a real parser -- these strings
/// are already-rendered `describe_ts_type` output, not AST), since a
/// real occurrence is always a bare identifier, never part of a longer
/// one.
fn mentions_any_type_param(text: &str, generic: &thaw_bridge::DtsGenericFunction) -> bool {
    generic
        .type_params
        .iter()
        .any(|(name, _)| contains_word(text, name))
}

/// Whether `word` occurs in `text` as a standalone identifier (not part
/// of a longer one) -- these strings are already-rendered
/// `describe_ts_type` output, not AST, so this is a substring scan with
/// word-boundary checks rather than a real parse.
fn contains_word(text: &str, word: &str) -> bool {
    let is_word_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$';
    let bytes = text.as_bytes();
    let needle = word.as_bytes();
    text.match_indices(word).any(|(index, _)| {
        let before_ok = index == 0 || !is_word_byte(bytes[index - 1]);
        let after = index + needle.len();
        let after_ok = after == bytes.len() || !is_word_byte(bytes[after]);
        before_ok && after_ok
    })
}

/// Whether a callback-shaped param type string (e.g. `(oldValue: any) =>
/// any`, from a generic interface method's own callback parameter) is
/// safe to splice verbatim: it must not mention any of the function's
/// own generic type parameters (see `mentions_any_type_param`'s doc
/// comment), and every bare keyword inside it must be one thaw-hir's own
/// callback-type lowering actually accepts (`number`/`string`/`boolean`/
/// `void` only -- the crude renderer readily produces `any`/`unknown`/
/// `object`/etc, which are fine as a *top-level* `extends` constraint
/// (see `is_reparseable_ts_type`) but rejected outright inside a nested
/// function-type signature).
fn is_safe_callback_param_type(text: &str, generic: &thaw_bridge::DtsGenericFunction) -> bool {
    // A leaf type good to splice into this declaration's own source: one
    // of thaw-hir's callback-type keywords, or `Json`/`JsValue` (already
    // proven safe as a *plain* parameter, and no more of an external
    // reference here than there), or itself a nested callback
    // recursively checked the same way. Anything else -- most commonly a
    // real type reference like `Uint8Array`, but also one of this
    // function's own type parameters (proven separately to produce an
    // unresolvable bare name once compiled when used *inside* a nested
    // callback signature, unlike as a bare top-level parameter) -- isn't
    // in thaw-hir's short callback-type keyword list and/or names
    // something this shim doesn't carry along with it, so it's rejected
    // the same as a top-level `extends` constraint would be (see
    // `is_reparseable_ts_type`'s doc comment).
    fn is_safe_leaf_type(ty: &str, generic: &thaw_bridge::DtsGenericFunction) -> bool {
        let ty = ty.trim().trim_end_matches('?');
        if is_reparseable_ts_type(ty) || matches!(ty, "Json" | "JsValue") {
            return true;
        }
        ty.starts_with('(') && is_safe_callback_param_type(ty, generic)
    }

    if mentions_any_type_param(text, generic) {
        return false;
    }
    let Some(arrow) = text.rfind("=> ") else {
        return false;
    };
    let (params, ret) = (&text[..arrow], &text[arrow + "=> ".len()..]);
    if !is_safe_leaf_type(ret, generic) {
        return false;
    }
    let Some(params) = params
        .trim()
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(") "))
    else {
        return false;
    };
    params.split(',').filter(|param| !param.trim().is_empty()).all(|param| {
        param
            .split_once(':')
            .is_some_and(|(_, ty)| is_safe_leaf_type(ty, generic))
    })
}

/// Whether `text` is safe to splice into a generated declaration as a
/// type parameter's own `extends` *constraint* -- thaw-hir keeps a
/// generic function's declared constraints as raw, unlowered syntax
/// (`FnSignature.generic_type_constraints`, checked structurally via
/// `bridge_type_satisfies_constraint`-style comparisons rather than
/// `lower_ts_type`), so this whitelist is deliberately wider than what's
/// valid in an ordinary *value* type position (a parameter or return
/// type) -- see `is_reparseable_value_type` for that narrower one.
/// Splicing a constraint this crude a renderer can't fully describe
/// (`describe_ts_type`'s `{ ... }` placeholder for a non-empty object
/// literal, found via a real interface method's `T extends { __trapAny:
/// any }`) would otherwise be spliced into the generated source
/// verbatim as broken syntax.
fn is_reparseable_ts_type(text: &str) -> bool {
    matches!(
        text,
        "any" | "unknown"
            | "object"
            | "string"
            | "number"
            | "boolean"
            | "bigint"
            | "symbol"
            | "void"
            | "undefined"
            | "null"
            | "never"
            // `Date` is special-cased by both thaw-bridge's classification
            // and thaw-hir's own `lower_ts_type` (see `date_object_type`),
            // so it's just as safe to splice into a generated declaration
            // as the keywords above -- real example: date-fns's
            // `format<DateType extends Date>(...)`.
            | "Date"
    )
}

/// Whether `text` is safe to splice into a generated declaration as an
/// ordinary *value* type -- a parameter or return type, which (unlike a
/// type parameter's own constraint, see `is_reparseable_ts_type`) does
/// get lowered through thaw-hir's `lower_ts_type`, whose keyword support
/// is deliberately narrower (`any`/`unknown`/`object`/`bigint`/`symbol`/
/// `undefined`/`null`/`never` are all rejected there, "supports
/// number/string/boolean/void" only) -- real example: lodash's
/// `cloneDeepWith<T>(value: T, customizer?: ...): any`, whose literal
/// `any` return used to get spliced straight into the generated ambient
/// declaration's own return position, which `lower_ts_type` then
/// rejected outright when the whole shim got lowered.
fn is_reparseable_value_type(text: &str) -> bool {
    matches!(text, "string" | "number" | "boolean" | "void" | "Date")
}

/// The mutually-exclusive JS `typeof` category `ty` maps to, when it's
/// simple enough for [`union_overload_dispatch_declaration`] to branch
/// on unambiguously -- deliberately narrow (no object/array/function
/// categories, where more than one shape can share a `typeof`) so every
/// qualifying overload's first parameter type maps to a distinct
/// category or the whole mechanism bails out.
fn typeof_discriminator(ty: &thaw_hir::HirType) -> Option<&'static str> {
    match ty {
        thaw_hir::HirType::F64 => Some("number"),
        thaw_hir::HirType::Str => Some("string"),
        thaw_hir::HirType::Bool => Some("boolean"),
        _ => None,
    }
}

/// Dispatches a top-level Fallback function's `.d.ts` overloads purely
/// by the *type* of their shared first parameter -- real example: `ms`'s
/// two overloads, `(value: number, options?): string` and `(value:
/// string): number`, both really calling the same real JS function
/// (whose own implementation already does this exact `typeof` dispatch
/// internally). Without this, `typed_dynamic_declaration`'s per-function
/// "first successful overload wins" rule (needed to avoid emitting two
/// conflicting ambient declarations under the same name -- see its own
/// doc comment) picks exactly one of the two, silently breaking every
/// call shaped like the other.
///
/// Deliberately narrow: every qualifying overload's first parameter type
/// must map to a distinct `typeof` category (`typeof_discriminator`),
/// its return type must be `Native` and non-`void`, and at most one
/// ("primary") overload may carry its own extra parameters -- every
/// other ("secondary") overload must take exactly that one required
/// parameter and nothing else. Returns `None` (falls back to ordinary
/// first-wins) for anything wider than that -- ambiguous discriminators,
/// two overloads both wanting extra parameters, or fewer than two
/// overloads actually qualifying at all. The generated dispatcher's own
/// parameter and return types are the union of every qualifying
/// overload's own (`coerce_to_declared`'s existing `Union` handling,
/// already exercised by an ordinary Union-typed Fallback parameter,
/// covers wrapping each branch's concrete result into it for free).
fn union_overload_dispatch_declaration(
    package: &str,
    name: &str,
    overloads: &[&thaw_bridge::DtsFunction],
) -> Option<(String, String, Vec<FallbackFunctionOverloadRewrite>)> {
    struct Candidate<'a> {
        symbol: String,
        category: &'static str,
        param_name: &'a str,
        param_type: thaw_hir::HirType,
        ret_type: thaw_hir::HirType,
        extra_params: Vec<(&'a str, thaw_hir::HirType, bool)>,
        source: &'a thaw_bridge::DtsFunction,
    }

    let empty_arities = std::collections::BTreeSet::new();
    let mut candidates = Vec::new();
    for (index, function) in overloads.iter().enumerate() {
        if function.generic.is_some() || function.rest_param.is_some() {
            continue;
        }
        let Some((param_name, thaw_bridge::DtsType::Native(param_type))) = function.params.first()
        else {
            continue;
        };
        let Some(category) = typeof_discriminator(param_type) else {
            continue;
        };
        let thaw_bridge::DtsType::Native(ret_type) = &function.ret else {
            continue;
        };
        if *ret_type == thaw_hir::HirType::Void {
            continue;
        }
        if function.params.len() > 1 && function.required_params != 1 {
            continue;
        }
        let Some(extra_params) = function.params[1..]
            .iter()
            .enumerate()
            .map(|(offset, (name, ty))| {
                let thaw_bridge::DtsType::Native(ty) = ty else {
                    return None;
                };
                Some((
                    name.as_str(),
                    ty.clone(),
                    offset + 1 >= function.required_params,
                ))
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let Some((symbol, declaration)) =
            typed_dynamic_declaration(package, function, false, &empty_arities, Some(index))
        else {
            continue;
        };
        candidates.push((
            declaration,
            Candidate {
                symbol,
                category,
                param_name,
                param_type: param_type.clone(),
                ret_type: ret_type.clone(),
                extra_params,
                source: function,
            },
        ));
    }
    if candidates.len() < 2 {
        return None;
    }
    let mut categories = candidates.iter().map(|(_, c)| c.category).collect::<Vec<_>>();
    categories.sort_unstable();
    categories.dedup();
    if categories.len() != candidates.len() {
        return None;
    }
    if candidates
        .iter()
        .filter(|(_, c)| !c.extra_params.is_empty())
        .count()
        > 1
    {
        return None;
    }

    let mut declarations = candidates
        .iter()
        .map(|(declaration, _)| declaration.clone())
        .collect::<String>();
    let primary = candidates
        .iter()
        .map(|(_, c)| c)
        .find(|c| !c.extra_params.is_empty());
    let param_types = candidates
        .iter()
        .map(|(_, c)| c.param_type.clone())
        .collect::<Vec<_>>();
    let param_union = if let [only] = param_types.as_slice() {
        only.clone()
    } else {
        thaw_hir::HirType::Union(param_types)
    };
    let mut ret_types = candidates.iter().map(|(_, c)| c.ret_type.clone()).collect::<Vec<_>>();
    ret_types.dedup();
    let ret_union = if let [only] = ret_types.as_slice() {
        only.clone()
    } else {
        thaw_hir::HirType::Union(ret_types)
    };
    let param_name = primary.map_or(candidates[0].1.param_name, |primary| primary.param_name);
    let mut params_text = format!("{param_name}: {}", render_dynamic_type(&param_union)?);
    if let Some(primary) = primary {
        for (name, ty, optional) in &primary.extra_params {
            params_text.push_str(&format!(
                ", {name}{}: {}",
                if *optional { "?" } else { "" },
                render_dynamic_type(ty)?
            ));
        }
    }
    let ret_text = render_dynamic_type(&ret_union)?;
    let encoded = format!("{package}::{name}")
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let dispatcher = format!("__thaw_typed_dispatch_js_{encoded}");
    declarations.push_str(&format!(
        "function {dispatcher}({params_text}): {ret_text} {{\n"
    ));
    for (index, (_, candidate)) in candidates.iter().enumerate() {
        // The dispatcher has exactly one shared variable for this
        // shared first-argument slot -- `param_name` (the dispatcher's
        // own declared parameter, possibly named after a *different*
        // candidate's own parameter than this one, real example:
        // lodash's `random`'s `(floating?: boolean): number` overload
        // names its only parameter `floating`, not `max`) -- so every
        // branch's call must reference that one shared name, never
        // `candidate.param_name` (that candidate's own, possibly
        // differently-spelled, name for the very same slot).
        let arguments = std::iter::once(param_name.to_string())
            .chain(candidate.extra_params.iter().map(|(name, ..)| name.to_string()))
            .collect::<Vec<_>>()
            .join(", ");
        let call = format!("{}({arguments})", candidate.symbol);
        if index + 1 == candidates.len() {
            declarations.push_str(&format!("    return {call};\n"));
        } else {
            declarations.push_str(&format!(
                "    if (typeof {param_name} === \"{}\") {{ return {call}; }}\n",
                candidate.category
            ));
        }
    }
    declarations.push_str("}\n");
    // Each candidate's own typed symbol (already declared above, as
    // part of `declarations`) is also a legitimate argument-shape
    // rewrite target: the dispatcher's own declared type is a Union
    // (both its parameter and its return, e.g. `ms`'s `string | number`
    // in and out) since a runtime `typeof` check can't be reflected in
    // a static signature, but a real call site whose argument shape
    // statically matches one specific overload can be rewritten
    // straight to that overload's own precisely-typed symbol instead --
    // narrowing `const n: number = ms("2 days")` to a real `number`
    // rather than a `string | number` real callers can't assign
    // anywhere without a cast. `class_methods.rs`'s rewrite pass
    // already picks the best-matching candidate per call site by arity
    // and `overload_type_score`; the dispatcher call above stays as the
    // unconditional fallback for anything that pass doesn't recognize.
    let rewrites = candidates
        .iter()
        .map(|(_, candidate)| {
            (
                name.to_string(),
                candidate.symbol.clone(),
                candidate.source.required_params,
                candidate.source.params.len(),
                dts_function_param_hir_types(candidate.source),
                None,
            )
        })
        .collect();
    Some((dispatcher, declarations, rewrites))
}

/// Builds a plain forwarding wrapper under `bare_name` (the Fallback
/// function's own, un-mangled name, e.g. `"optional"`) that just calls
/// `symbol` -- whatever typed, correctly-marshaling declaration this
/// function already resolved to (a bare ambient declaration, an
/// arity-dispatch wrapper, a callback adapter, or a JIT-compiled native
/// function; this doesn't need to know or care which, since it only
/// forwards positionally).
///
/// Without this, the "no import at all, `--use`'d package's bare name"
/// and package-qualified (`pkg.name(...)`, rewritten to `pkg_name(...)`)
/// call syntaxes both reach only `generate_shim`'s own, always-untyped
/// `(argsArray: Json): Json` fallback under that exact name (a plain
/// import, by contrast, gets rewritten straight to the typed `symbol` --
/// see `module_graph.rs`'s `external_exports` lookup -- so it was never
/// affected). Real example: `--use semver` with no import, then calling
/// bare `semver.major("1.2.3", true)`, crashed ("args_json is not a
/// valid JSON array") the moment `major` took more than the untyped
/// fallback's own single pre-packed-array parameter.
///
/// Not attempted for a generic function (generic-forwarding -- a
/// wrapper's own still-unresolved type parameter flowing into another
/// generic ambient call -- isn't supported: confirmed via a direct
/// experiment that thaw-hir's generic inference sees `Dynamic` for such
/// a parameter and rejects it against the callee's constraint) or one
/// with a `...rest` parameter; the caller filters both out before
/// calling this.
fn typed_dynamic_bare_alias(
    bare_name: &str,
    symbol: &str,
    function: &thaw_bridge::DtsFunction,
    is_jit_backed: bool,
    napi: bool,
) -> Option<String> {
    if function.rest_param.is_some() || function.generic.is_some() {
        return None;
    }
    // Same restriction `typed_dynamic_declaration` itself applies to a
    // `Function`/`CallableFunction`-classified parameter: no QuickJS-NG
    // side marshaling exists for a typed function argument outside the
    // `napi` backend. This wrapper forwards to `symbol` (`typed_dynamic_
    // declaration`'s own, already-correctly-widened declaration) -- if
    // *this* alias's own signature didn't widen the identical parameter
    // the same way, thaw-hir's omitted-trailing-optional-parameter
    // machinery (reached whenever the alias is called with the parameter
    // left out, real example: lodash's `filter(collection)` with its
    // optional `predicate` omitted) needs to synthesize a value for it
    // using *this* declared type -- which, left as a real callback type,
    // crashed with "unsupported dynamic object field Function(...)"
    // (`compile_json_object_set_native`, thaw-llvm, has no `Function`
    // arm either). Kept as its own small copy rather than shared with
    // `typed_dynamic_declaration`'s identical match arm, matching this
    // function's own established convention just below of small
    // independent copies over threading a shared helper through.
    let params = function
        .params
        .iter()
        .map(|(name, ty)| {
            let rendered = match ty {
                thaw_bridge::DtsType::Native(ty) if !napi && contains_callable_type(ty) => {
                    "Json".to_string()
                }
                thaw_bridge::DtsType::Native(ty) => render_dynamic_type(ty)?,
                thaw_bridge::DtsType::Unsupported(_) => "Json".to_string(),
            };
            Some((name.clone(), rendered))
        })
        .collect::<Option<Vec<_>>>()?;
    // Same "Unsupported -> Json/JsValue" mapping `typed_dynamic_declaration`
    // itself uses for a return type -- kept as a separate small copy
    // rather than shared, since the two are independent, narrow leaves
    // (not worth the churn of threading a shared helper through an
    // already-large, working function for this).
    let ret_hir_type = match &function.ret {
        thaw_bridge::DtsType::Unsupported(reason)
            if reason == "`any` is not supported" || reason == "`unknown` is not supported" =>
        {
            thaw_hir::HirType::Json
        }
        thaw_bridge::DtsType::Unsupported(_) => thaw_hir::HirType::JsValue,
        thaw_bridge::DtsType::Native(ret) => ret.clone(),
    };
    // Unlike a plain value return, a *callback*-returning function's
    // `symbol` here isn't necessarily the plain, `JsValue`-returning base
    // declaration `typed_dynamic_declaration` starts from -- when the
    // callback's own return type is one `typed_dynamic_callable_adapter`
    // can convert (`string`/`number`/`boolean`/`Json`), `symbol` is
    // *that* adapter instead, which really does hand back a callable
    // matching the original `.d.ts` signature (see its own doc comment).
    // Matches its exact success condition so this wrapper's own declared
    // return type agrees with what `symbol` actually returns either way
    // -- getting this wrong doesn't just misdeclare the wrapper, it makes
    // `return symbol(...);` itself a type error inside its own body
    // (a real function value where the declaration said `JsValue`).
    let callback_adapter_applies = match &ret_hir_type {
        thaw_hir::HirType::Function(_, ret) => {
            matches!(
                ret.as_ref(),
                thaw_hir::HirType::Str
                    | thaw_hir::HirType::F64
                    | thaw_hir::HirType::Bool
                    | thaw_hir::HirType::Json
                    | thaw_hir::HirType::JsValue
            )
        }
        thaw_hir::HirType::CallableFunction(_, _, None, ret) => {
            matches!(
                ret.as_ref(),
                thaw_hir::HirType::Str
                    | thaw_hir::HirType::F64
                    | thaw_hir::HirType::Bool
                    | thaw_hir::HirType::Json
                    | thaw_hir::HirType::JsValue
            )
        }
        _ => false,
    };
    // The untyped `(argsArray: Json)` native-addon / `callDynamic` shim
    // already serves a zero- or single-argument Fallback function
    // correctly: a call `f(x)` builds `[x]`, which *is* the packed
    // argument array that shim expects. `6448a499` added this typed bare
    // alias to fix *multi*-argument bare/qualified calls -- where `f(a,
    // b)` builds `[a, b]` and the one-parameter untyped shim rejects the
    // arity outright -- but for a single argument it only adds a second
    // layer of wrapping. Real breakage: `utf-8-validate`'s
    // `isValidUTF8(buffer: Buffer)` (`Buffer` renders as `Json`), called
    // with an already-array-shaped `Buffer` payload, reached the addon as
    // `[[...]]` and tripped an N-API assertion. Keep the typed alias only
    // where it genuinely earns its place: more than one real parameter,
    // or a parameter/return the untyped shim can't marshal (a callback).
    let param_needs_marshaling = function.params.iter().any(|(_, ty)| match ty {
        thaw_bridge::DtsType::Native(ty) => contains_callable_type(ty),
        thaw_bridge::DtsType::Unsupported(_) => false,
    });
    if function.params.len() <= 1 && !param_needs_marshaling && !callback_adapter_applies {
        return None;
    }
    // A JIT-backed `symbol` (`jit_numeric_declaration`) renders its own
    // return type with a special, parenthesized convention for an
    // `Optional`-wrapped tagged union -- `(A | B) | undefined`, not the
    // generic `render_dynamic_type`'s flat `A | B | undefined` -- so
    // that thaw-hir's type parser reconstructs the same `Optional(Union
    // (...))` shape from the text instead of collapsing it into a flat,
    // explicit-`Undefined`-member union (see `jit_numeric_declaration`'s
    // own `optional_union_ret`). This wrapper's own declared return type
    // must match whichever convention `symbol` actually used, or `return
    // symbol(...);` is a type error inside the wrapper's own body even
    // though both sides describe the same value -- getting it backwards
    // the other way (parenthesizing for a *non*-JIT symbol) is just as
    // wrong: the ordinary dynamic-call path (`typed_dynamic_declaration`)
    // always uses the flat form for this same shape, and re-parses the
    // parenthesized text into a genuinely different `HirType` its own
    // marshaling doesn't expect.
    let jit_optional_tagged_union_ret = is_jit_backed
        .then(|| match &ret_hir_type {
            thaw_hir::HirType::Optional(payload) => match payload.as_ref() {
                thaw_hir::HirType::Union(elements) if jit_tagged_union(elements) => {
                    render_dynamic_type(payload).map(|inner| format!("({inner}) | undefined"))
                }
                _ => None,
            },
            _ => None,
        })
        .flatten();
    let ret = if let Some(rendered) = jit_optional_tagged_union_ret {
        rendered
    } else if matches!(
        ret_hir_type,
        thaw_hir::HirType::Function(_, _) | thaw_hir::HirType::CallableFunction(..)
    ) && !callback_adapter_applies
    {
        "JsValue".to_string()
    } else {
        render_dynamic_type(&ret_hir_type)?
    };
    let rendered_params = params
        .iter()
        .enumerate()
        .map(|(index, (name, ty))| {
            format!(
                "{name}{}: {ty}",
                if index >= function.required_params { "?" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let args = params
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let call = if ret == "void" {
        format!("{symbol}({args}); return;")
    } else {
        format!("return {symbol}({args});")
    };
    Some(format!(
        "function {bare_name}({rendered_params}): {ret} {{\n    {call}\n}}\n"
    ))
}

fn contains_callable_type(ty: &thaw_hir::HirType) -> bool {
    match ty {
        thaw_hir::HirType::Function(_, _) | thaw_hir::HirType::CallableFunction(..) => true,
        thaw_hir::HirType::Optional(inner)
        | thaw_hir::HirType::Nullable(inner)
        | thaw_hir::HirType::Nullish(inner) => contains_callable_type(inner),
        thaw_hir::HirType::Union(members) => members.iter().any(contains_callable_type),
        _ => false,
    }
}

fn typed_dynamic_declaration(
    package: &str,
    function: &thaw_bridge::DtsFunction,
    napi: bool,
    observed_call_arities: &std::collections::BTreeSet<usize>,
    overload_suffix: Option<usize>,
) -> Option<(String, String)> {
    let runtime_key = if napi {
        function.name.clone()
    } else {
        format!("{package}::{}", function.name)
    };
    let encoded = runtime_key
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    // Distinguishes two ambient declarations that must decode back to
    // the *same* runtime symbol (both dispatch through the one real JS
    // function at the far end -- see `union_overload_dispatch_declaration`)
    // but need different Rust/LLVM-level names because their own
    // parameter/return types genuinely differ. Stripped back off by
    // thaw-hir's `dynamic_symbol` before hex-decoding, so it's applied
    // to `encoded` itself (every symbol this function derives from it,
    // including the rest/arity-dispatch helpers below, inherits it) --
    // never to the `runtime_key` text above, which is the literal
    // lookup string `thaw_js_call`/`getDynamicValue` uses at runtime and
    // must stay exactly what the real JS function is bound under.
    let encoded = match overload_suffix {
        Some(index) => format!("{encoded}__overload_{index}"),
        None => encoded,
    };
    let base_symbol = format!(
        "__thaw_typed_{}_{}",
        if napi { "napi" } else { "js" },
        encoded
    );
    // `argsArray` is this codebase's reserved spelling for an untyped
    // variadic API. Turn it into a real rest parameter so observed call
    // arities get normal forwarding wrappers instead of a one-argument
    // function that rejects valid multi-argument calls.
    if let [(name, _)] = function.params.as_slice() {
        if name == "argsArray" && function.rest_param.is_none() {
            let mut variadic = function.clone();
            let (_, ty) = variadic.params.pop().unwrap();
            variadic.required_params = 0;
            variadic.rest_param = Some((name.clone(), ty));
            return typed_dynamic_rest_declaration(
                &variadic,
                napi,
                &encoded,
                &base_symbol,
                observed_call_arities,
            );
        }
    }
    if function.rest_param.is_some() {
        return typed_dynamic_rest_declaration(
            function,
            napi,
            &encoded,
            &base_symbol,
            observed_call_arities,
        );
    }
    if let Some(generic) = &function.generic {
        // A parameter type this crude a renderer can't classify as one
        // of the forms below still passes through as `Json` (matching
        // the non-generic path's own Unsupported -> Json substitution)
        // rather than aborting the whole declaration -- real example:
        // lodash's `uniq<T>(array: List<T> | null | undefined): T[]`,
        // where `List<T>` is one of lodash's own unresolved type
        // aliases. Without this, a function like `uniq` fell all the
        // way to the bare untyped `(argsArray: Json): Json` fallback,
        // which expects its caller to already have packed every real
        // argument into one array -- silently wrong for a genuine
        // single-array-argument call like `uniq(someArray)`, which
        // instead spread `someArray`'s own elements as the args.
        let param_types = generic
            .param_types
            .iter()
            .map(|ty| {
                if generic.type_params.iter().any(|(name, _)| name == ty)
                    // Same restriction as the non-generic branch below and
                    // `supported_class_method_param` for methods: a
                    // callback-shaped parameter has no QuickJS-side
                    // marshaling at all (`compile_typed_dynamic_call`
                    // silently drops it for any non-`napi` backend), so
                    // it's only safe to keep as a real callback type when
                    // this declaration is `napi`-backed.
                    || (napi && ty.starts_with('(') && is_safe_callback_param_type(ty, generic))
                    || matches!(ty.as_str(), "number" | "string" | "boolean" | "Json" | "JsValue")
                {
                    ty.clone()
                } else {
                    "Json".to_string()
                }
            })
            .collect::<Vec<_>>();
        // thaw-hir can infer a type parameter from the call site's own
        // expected type (a `let`/`const` declaration's annotation) as a
        // fallback when no argument mentions it at all, but only when
        // *this* declaration's return type actually still says so --
        // real example: `nanoid<Type extends string>(size?: number):
        // Type`, where `Type` appears solely in the return position, so
        // `const id: string = nanoid()` is the only place its value is
        // ever written down. Preserved (kept declared, return rendered
        // as the parameter name itself) only for that exact shape --
        // the return being precisely one bare type parameter -- since
        // anything more complex (`T[]`, `Promise<T>`, ...) would need
        // the same re-parseability/external-name scrutiny the parameter
        // substitution above already goes through, not yet done for a
        // return position.
        //
        // A type parameter that isn't returned bare like that, and was
        // substituted out of every parameter that used to carry it (the
        // case just above), is left with no remaining occurrence to
        // infer it from at all -- thaw-hir requires inferring every
        // declared type parameter from some argument or the return
        // fallback, so keeping it declared here would make every real
        // call an inference error even though nothing downstream
        // actually needed it. Dropped from the declaration entirely
        // rather than kept as dead syntax.
        let returns_bare_type_param = generic
            .type_params
            .iter()
            .any(|(name, _)| *name == generic.return_type);
        let type_params = generic
            .type_params
            .iter()
            .filter(|(name, _)| {
                param_types.iter().any(|ty| ty == name) || *name == generic.return_type
            })
            .map(|(name, constraint)| match constraint.as_deref() {
                // `describe_ts_type` is built for a human-readable
                // Fallback *reason* ("a conditional type", "{ ... }" for
                // a non-empty object literal, ...), not for re-parseable
                // syntax -- a constraint it can't fully render would
                // otherwise get spliced into this declaration's actual
                // source text verbatim (found via a real interface
                // method's `T extends { __trapAny: any }` constraint
                // producing the literal placeholder text `{ ... }`).
                // Dropped rather than aborting the whole declaration the
                // way it used to -- a type parameter constrained to some
                // other named type (`T extends core.SomeType`, zod's own
                // `optional<T extends core.SomeType>(innerType: T):
                // ZodOptional<T>`) is an extremely common TS idiom, not a
                // rare edge case, and the parameter using `T` still ends
                // up passed through as `JsValue` below regardless of
                // whether the constraint survives -- thaw-hir infers `T`
                // from the call site's actual argument either way, same
                // as it would for a bare, unconstrained type parameter.
                Some(constraint) if is_reparseable_ts_type(constraint) => {
                    format!("{name} extends {constraint}")
                }
                _ => name.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        // A return type that doesn't mention any of the function's own
        // type parameters at all is just a concrete type, generic
        // function or not -- real example: date-fns's `format<DateType
        // extends Date>(date: DateType | number | string, formatStr:
        // string, options?: FormatOptions): string`, whose `string`
        // return has nothing to do with `DateType`. Rendering it as
        // `JsValue` unconditionally (as if every generic function's
        // return depended on its type parameters) would needlessly
        // discard a plain, safely-reparseable type. Anything that does
        // mention a type param, beyond the bare-type-param case above,
        // still falls back to `JsValue` (same unresolved-scope limitation
        // noted above).
        let return_type = if returns_bare_type_param
            || (!mentions_any_type_param(&generic.return_type, generic)
                && is_reparseable_value_type(&generic.return_type))
        {
            generic.return_type.as_str()
        } else {
            "JsValue"
        };
        let generics = if type_params.is_empty() {
            String::new()
        } else {
            format!("<{type_params}>")
        };
        let params: Vec<(String, String)> = function
            .params
            .iter()
            .zip(&param_types)
            .map(|((name, _), ty)| (name.clone(), ty.clone()))
            .collect();
        // A single ambient declaration with `?`-marked optional trailing
        // parameters relies on thaw-hir tolerating an omitted trailing
        // argument only for a *still-generic* declaration's own
        // `generic_param_optional` arity check (`lower/invocations.rs`)
        // -- exactly nanoid's shape (`nanoid<Type extends string>(size?:
        // number): Type`, `type_params` non-empty below). Once every type
        // parameter this function declared has been substituted or
        // dropped above (`type_params` empty here -- real example:
        // date-fns's `format<DateType extends Date>(date: DateType |
        // number | string, formatStr: string, options?: FormatOptions):
        // string`, whose only type parameter doesn't survive into any
        // retained parameter or the return), the emitted declaration is
        // just a plain ambient function, and thaw-hir enforces *exact*
        // arity for those -- falling back to the same per-arity-
        // declarations-plus-dispatcher trick the plain branch below uses
        // for the same reason.
        if type_params.is_empty() && function.required_params < params.len() {
            return typed_dynamic_arity_dispatch_declaration(
                &encoded,
                base_symbol,
                &params,
                function.required_params,
                return_type,
                napi,
                &thaw_hir::HirType::JsValue,
            );
        }
        let rendered_params = params
            .iter()
            .enumerate()
            .map(|(index, (name, ty))| {
                format!(
                    "{name}{}: {ty}",
                    if index >= function.required_params {
                        "?"
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Some((
            base_symbol.clone(),
            format!(
                "declare function {base_symbol}{generics}({rendered_params}): {return_type};\n"
            ),
        ));
    }
    let params = function
        .params
        .iter()
        .map(|(name, ty)| match ty {
            // A `Function`/`CallableFunction`-classified parameter has no
            // QuickJS-NG-side marshaling at all (`compile_typed_dynamic_
            // call`, thaw-llvm's `dynamic_host.rs`, only wires a typed
            // function argument through for the `napi` backend with a
            // `JsValue` return -- for every other combination, including
            // every Fallback/QuickJS call, it silently drops the argument
            // from the JSON args array instead of erroring). Widened to
            // `Json` here the same way `Unsupported` already is, so the
            // closure instead reaches `coerce_to_declared`'s `Json`-gated
            // `registerNativeCallback` wrapping -- the mechanism that
            // already correctly bridges a native closure into a real,
            // callable QuickJS value elsewhere (a dynamic method-call
            // argument, or a Promise-returning callback's own return).
            // Real example: drizzle-orm's `sqlite-proxy` driver,
            // `drizzle(callback: (sql, params, method) => Promise<{rows}>)`
            // -- a plain top-level Fallback function, not a method call.
            thaw_bridge::DtsType::Native(ty) if !napi && contains_callable_type(ty) => {
                Some((name.clone(), "Json".to_string()))
            }
            // The identical restriction, for a callback parameter that's
            // also *optional* (`predicate?: (value: string, index:
            // number, s: string) => boolean` -- real example: lodash's
            // `filter`'s string-collection overload). Classifies as
            // `Native(Optional(Function(...)))`, a different `Native`
            // variant from the bare case just above, so it fell through
            // to the generic `render_dynamic_type` branch below unwidened
            // -- `compile_json_object_set_native_with_undefined`
            // (thaw-llvm) has no `Function`/`CallableFunction` arm either
            // (only its sibling, the array-element path, was fixed for
            // this earlier), so building any call to such a function
            // crashed with "unsupported dynamic object field
            // Function(...)". Optionality itself needs no special
            // handling once widened -- a JSON value already represents
            // "omitted" naturally, the same as the bare case.
            thaw_bridge::DtsType::Native(ty) => {
                render_dynamic_type(ty).map(|ty| (name.clone(), ty))
            }
            // An individual parameter's real .d.ts type isn't classifiable
            // (`unknown`, an unresolved type reference, a literal
            // `undefined`/`null` type used only to disambiguate a sibling
            // overload, etc.) -- it's still a perfectly callable
            // positional argument at runtime, so pass it through as
            // `Json` (every dynamic call already marshals its arguments
            // through JSON, and `coerce_to_declared` knows how to box any
            // JSON-convertible native value into one) instead of giving
            // up on a typed wrapper for the *whole* function. Real
            // examples: uuid's `v4(options?, buf?: undefined, offset?:
            // number)`, where `buf`'s `undefined` literal type exists
            // only to steer TS overload resolution, and its
            // `validate(uuid: unknown): boolean`.
            thaw_bridge::DtsType::Unsupported(_) => Some((name.clone(), "Json".to_string())),
        })
        .collect::<Option<Vec<_>>>()?;
    // An unclassifiable return type (most commonly a class instance --
    // real example: dayjs's own factory function returning its `Dayjs`
    // class -- or, same as above, a function value) still crosses the
    // host boundary as a retained handle rather than aborting the whole
    // typed declaration; callers can invoke it dynamically
    // (callDynamicValue/callDynamicValueHandle) or just hold/print it,
    // even without dedicated support for its own methods/properties.
    //
    // Kept as the *real* `Function`/`CallableFunction` type here (not
    // collapsed to `JsValue` the way `ret`'s own rendered text below is)
    // specifically so `typed_dynamic_callable_adapter` can still
    // recognize and wrap it -- collapsing this one too used to make
    // every callback-returning Fallback function (real example: nanoid's
    // `customAlphabet(alphabet, size?): (size?: number) => string`)
    // silently skip the adapter and hand back a bare, uninvokable
    // `JsValue` instead of a real callable.
    let ret_hir_type = match &function.ret {
        // A return type declared literally `any`/`unknown` (as opposed
        // to one that merely fails to classify -- a class instance, a
        // conditional type, ...) means "arbitrary JSON-shaped data", the
        // same as it already does for a *parameter* of that type
        // (`json_convertible_native_type`'s own reasoning) -- not "an
        // opaque handle with no defined shape at all". Real example:
        // thaw-registry's own node built-in shims, `declare function
        // join(argsArray: any): any;`, whose actual runtime result
        // (`path.join(...)`, a plain string) callers already round-trip
        // through ordinary JSON conversions like `String(...)`; those
        // broke (`String(...)` only accepts `Json`) the moment this
        // classified as `Unsupported` too and got the same blanket
        // `JsValue` substitution any other Unsupported return does.
        // Matched by text since `DtsType::Unsupported` only carries a
        // diagnostic reason, not the original type -- coupled to
        // `keyword_name`'s exact wording for `any`/`unknown` in
        // thaw-bridge's `classify_ts_type`.
        thaw_bridge::DtsType::Unsupported(reason)
            if reason == "`any` is not supported" || reason == "`unknown` is not supported" =>
        {
            thaw_hir::HirType::Json
        }
        thaw_bridge::DtsType::Unsupported(_) => thaw_hir::HirType::JsValue,
        thaw_bridge::DtsType::Native(ret) => ret.clone(),
    };
    let ret = if matches!(
        ret_hir_type,
        thaw_hir::HirType::Function(_, _) | thaw_hir::HirType::CallableFunction(..)
    ) {
        "JsValue".to_string()
    } else {
        render_dynamic_type(&ret_hir_type)?
    };
    let render_params = |arity: usize| {
        params[..arity]
            .iter()
            .map(|(name, ty)| format!("{name}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if function.required_params == params.len() {
        let declarations = format!(
            "declare function {base_symbol}({}): {ret};\n",
            render_params(params.len())
        );
        return typed_dynamic_callable_adapter(
            &encoded,
            base_symbol,
            declarations,
            &params,
            function.required_params,
            napi,
            &ret_hir_type,
        );
    }
    typed_dynamic_arity_dispatch_declaration(
        &encoded,
        base_symbol,
        &params,
        function.required_params,
        &ret,
        napi,
        &ret_hir_type,
    )
}

/// Shared tail of [`typed_dynamic_declaration`]'s plain and generic
/// branches for a function with optional trailing parameters: thaw-hir
/// enforces *exact* arity for a plain ambient (`declare function`)
/// signature (see `lower/invocations.rs`'s arity check -- only a
/// still-generic declaration's own `generic_param_optional` tolerance
/// lets a real trailing argument be omitted), so a single declaration
/// with `?`-marked params would make a legitimately optional trailing
/// argument a hard arity error. Declares one ambient function per arity
/// from `required_params` to `params.len()` instead, plus a real
/// (non-ambient) TS wrapper function that narrows each optional argument
/// with its own nested `!== undefined` guard and dispatches to the
/// matching fixed-arity declaration.
fn typed_dynamic_arity_dispatch_declaration(
    encoded: &str,
    base_symbol: String,
    params: &[(String, String)],
    required_params: usize,
    ret: &str,
    napi: bool,
    ret_hir_type: &thaw_hir::HirType,
) -> Option<(String, String)> {
    let render_params = |arity: usize| {
        params[..arity]
            .iter()
            .map(|(name, ty)| format!("{name}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let wrapper = format!(
        "__thaw_typed_wrapper_{}_{}",
        if napi { "napi" } else { "js" },
        encoded
    );
    let mut declarations = String::new();
    for arity in required_params..=params.len() {
        declarations.push_str(&format!(
            "declare function {base_symbol}__arity_{arity}({}): {ret};\n",
            render_params(arity)
        ));
    }
    let wrapper_params = params
        .iter()
        .enumerate()
        .map(|(index, (name, ty))| {
            format!(
                "{name}{}: {ty}",
                if index >= required_params { "?" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    // A `void`-returning extern call can't itself be the operand of
    // `return` (thaw-hir rejects `return f();` for a `void`-declared
    // `f`, only a bare `return;`), so those need their call and return
    // as separate statements.
    let call_and_return = |call: String| -> String {
        if ret == "void" {
            format!("{call}; return;")
        } else {
            format!("return {call};")
        }
    };
    declarations.push_str(&format!("function {wrapper}({wrapper_params}): {ret} {{\n"));
    for arity in (required_params + 1..=params.len()).rev() {
        // Every optional slot up to `arity`, not just the last one, needs
        // its own `!= undefined` guard *nested* around this call, not
        // ANDed into one condition: the type-narrowing pass only narrows
        // the single variable a bare `a != undefined` guard names, and
        // recurses into just the left side of an `a != undefined && b !=
        // undefined` chain, so joining them with `&&` would leave every
        // conjunct but the first still statically `Optional(...)`.
        // Nesting instead gets each one narrowed by its own `if`, and all
        // of them stay narrowed going deeper. Real case: uuid's
        // `v4(options?, buf?, offset?)`, three trailing optional params.
        //
        // Loose `!=`, not strict `!==`: identical to strict for a plain
        // optional parameter (its only two states are exactly `undefined`
        // or a value, so there's no coercion difference to worry about),
        // but required when the parameter is *also* nullable
        // (`position?: number | null`, thaw-registry's own `fs.readvSync`/
        // `writevSync`) -- the narrowing pass only narrows a `Nullish`
        // (optional-and-nullable) variable's own strict-equality-checked
        // sibling comparisons (`==`/`!=`) against `undefined`/`null`, not
        // `===`/`!==` ones, since strict `!== undefined` alone doesn't
        // rule out `null` the way loose `!= undefined` does. Left
        // narrowed to `Optional`/`Nullable`/`Nullish`'s appropriate
        // wrapper, `!==` here forwarded the *whole* `Nullish` value
        // unnarrowed into this arity's own fixed (non-optional) parameter
        // slot, a type mismatch this codebase's own arity-checked calls
        // don't tolerate.
        let optional_slots = &params[required_params..arity];
        for (name, _) in optional_slots.iter().rev() {
            declarations.push_str(&format!("    if ({name} != undefined) {{\n"));
        }
        let arguments = params[..arity]
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let call = call_and_return(format!("{base_symbol}__arity_{arity}({arguments})"));
        declarations.push_str(&format!("    {call}\n"));
        for _ in optional_slots {
            declarations.push_str("    }\n");
        }
    }
    let arguments = params[..required_params]
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let call = call_and_return(format!("{base_symbol}__arity_{required_params}({arguments})"));
    declarations.push_str(&format!("    {call}\n}}\n"));
    typed_dynamic_callable_adapter(
        encoded,
        wrapper,
        declarations,
        params,
        required_params,
        napi,
        ret_hir_type,
    )
}
