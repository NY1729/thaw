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
