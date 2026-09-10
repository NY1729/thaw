impl<'a> FnLowerer<'a> {
    /// Coerces a value into its declared native layout.
    fn coerce_to_declared(&mut self, declared: &HirType, value: HirExpr) -> Result<HirExpr, String> {
        if *declared == HirType::Dynamic {
            return Ok(value);
        }
        if let HirExpr::Var(name) = &value {
            if self.native_class_aliases.get(name) == Some(declared) {
                return Ok(value);
            }
        }
        // The symmetric case to the `HirType::Json` branch just below --
        // a dynamic method call with no `JsValue` hint (`lower_dynamic_
        // value_method_call`'s own default) always comes back `Json`-
        // typed, even when the caller's own declared slot is a concrete
        // scalar. Real trigger: `const body: string = await res.text()`
        // (real hono) used to fail outright ("value has type Json,
        // expected Str") with no way to consume the result except by
        // keeping it `JsValue`-typed and manually decoding it later.
        // Reuses the exact `JsonAsNumber`/`JsonAsString`/`JsonAsBool`
        // nodes `dictionary_value_from_json` (objects.rs) already builds
        // for the identical "decode a Json field into its declared
        // scalar type" problem -- already fully supported by type
        // inference and both codegen backends, so no new HIR node or
        // codegen path is needed here.
        if matches!(declared, HirType::F64 | HirType::Str | HirType::Bool) {
            let actual = self.infer_expr_type(&value)?;
            if matches!(actual, HirType::Json | HirType::JsValue) {
                let value = if actual == HirType::JsValue {
                    HirExpr::Call(
                        Box::new(HirExpr::Var("readDynamicValue".to_string())),
                        vec![value],
                    )
                } else {
                    value
                };
                return Ok(match declared {
                    HirType::F64 => HirExpr::JsonAsNumber(Box::new(value)),
                    HirType::Str => HirExpr::JsonAsString(Box::new(value)),
                    HirType::Bool => HirExpr::JsonAsBool(Box::new(value)),
                    _ => unreachable!(),
                });
            }
        }
        if matches!(
            declared,
            HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) | HirType::Bytes
        ) {
            let actual = self.infer_expr_type(&value)?;
            // `Bytes` and `Array(F64)` share a layout -- pass either
            // straight through as the other.
            if matches!(
                (declared, &actual),
                (HirType::Bytes, HirType::Array(elem)) | (HirType::Array(elem), HirType::Bytes)
                    if **elem == HirType::F64
            ) {
                return Ok(value);
            }
            if matches!(actual, HirType::Json | HirType::JsValue) {
                let value = if actual == HirType::JsValue {
                    HirExpr::Call(
                        Box::new(HirExpr::Var("readDynamicValue".to_string())),
                        vec![value],
                    )
                } else {
                    value
                };
                return Ok(HirExpr::JsonAsNative(Box::new(value), declared.clone()));
            }
        }
        if *declared == HirType::JsValue
            && matches!(self.infer_expr_type(&value)?, HirType::Function(_, _))
        {
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("registerNativeCallback".to_string())),
                vec![value],
            ));
        }
        if let (HirType::Object(declared_fields), HirType::Object(actual_fields)) =
            (declared, self.infer_expr_type(&value)?)
        {
            if actual_fields.starts_with(declared_fields) {
                return Ok(value);
            }
            if let Some(class) = class_name_from_type(&HirType::Object(actual_fields.clone())) {
                let compatible = declared_fields.iter().all(|(name, expected)| {
                    if actual_fields
                        .iter()
                        .any(|(actual_name, actual)| actual_name == name && actual == expected)
                    {
                        return true;
                    }
                    let symbol = class_method_symbol(class, name);
                    if *expected == HirType::Dynamic
                        && self.signatures.keys().any(|candidate| {
                            candidate == &symbol
                                || candidate.starts_with(&format!("{symbol}__thaw_"))
                        })
                    {
                        return true;
                    }
                    let Some(signature) = self.signatures.get(&symbol) else {
                        return false;
                    };
                    let params = signature.params.get(1..).unwrap_or_default();
                    let fixed = if signature.native_rest.is_some() {
                        &params[..params.len().saturating_sub(1)]
                    } else {
                        params
                    };
                    match expected {
                        HirType::Dynamic if !signature.generic_type_params.is_empty() => true,
                        HirType::Function(expected_params, result) => {
                            fixed == expected_params && &signature.ret == result.as_ref()
                        }
                        HirType::CallableFunction(expected_params, _, expected_rest, result) => {
                            fixed.len() == expected_params.len()
                                && fixed.iter().zip(expected_params).all(|(actual, expected)| {
                                    matches!(expected, HirType::Optional(inner) if inner.as_ref() == actual)
                                        || expected == actual
                                })
                                && signature.native_rest.as_ref() == expected_rest.as_deref()
                                && &signature.ret == result.as_ref()
                        }
                        _ => false,
                    }
                });
                if compatible {
                    return Ok(value);
                }
            }
        }
        if *declared == HirType::Json {
            let actual = self.infer_expr_type(&value)?;
            if actual == HirType::Json {
                return Ok(value);
            }
            // A `JsValue` (an opaque handle to a live QuickJS-retained
            // object, e.g. zod's `z.string()` returning a `ZodString`
            // schema instance) has no JSON representation and so is left
            // untouched here rather than routed through
            // `wrap_native_value_as_json` -- thaw-llvm's dynamic-call
            // argument marshaling (`compile_dynamic_value_placeholder` in
            // json_bridge.rs) recognizes the resulting type mismatch
            // (a `JsValue` value reaching a `Json`-declared slot) and
            // threads the real handle to the call alongside the JSON args
            // instead of trying to serialize it.
            if actual == HirType::JsValue {
                return Ok(value);
            }
            // A real compiled (native) closure (e.g. `(n: number) => n >
            // 0` passed to zod's `z.number().refine(...)`) has no JSON
            // representation either -- but unlike `JsValue` above, it
            // can't just pass through unchanged: there is no existing
            // *live* value on the QuickJS side to reference yet, only a
            // native function pointer thaw-llvm needs to bridge into one.
            // Wrapped in a manual `registerNativeCallback` call (the same
            // convention `getDynamicProperty`/`callDynamicMethod`/etc.
            // already use for a hand-built intrinsic call thaw-llvm
            // recognizes by name -- see `compile_register_native_
            // callback`, thaw-llvm's `dynamic_host.rs`), which builds a
            // native-to-JS adapter (reusing the existing N-API `compile_
            // napi_value_callback`, itself backend-agnostic) and registers
            // it as a real, retained QuickJS value -- yielding an ordinary
            // `JsValue` handle from here on, so everything downstream
            // (`compile_dynamic_value_placeholder`'s own `is_int_value()`
            // detection, the args-JSON reviver, ...) treats it exactly
            // like any other live value crossing into a dynamic call.
            if let HirType::Function(_, _) = &actual {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("registerNativeCallback".to_string())),
                    vec![value],
                ));
            }
            // A bare `undefined` literal (`schema.safeParse(undefined)`,
            // real zod) has no JSON representation either -- JSON has no
            // `undefined`, only the case a nested field can omit itself
            // entirely (already handled: see `compile_json_object_set_
            // native_with_undefined`'s own `HirType::Undefined => Ok(())`
            // arm), which doesn't apply to a *standalone* value with no
            // field to omit. Encoded instead as the same `{"$__thaw_
            // napi_undefined$": true}` sentinel `compile_napi_undefined_
            // json` already uses for a NAPI return value -- a shared,
            // already-recognized-in-principle shape rather than a new
            // one, even though this crosses a different boundary
            // (`callDynamic`'s JSON argument array, not a NAPI result).
            // `__thaw_json_date_reviver` (QuickJS-NG, `dates.js`) is
            // taught to convert it back to the real literal on the far
            // side, the same way it already does for `Date`/`JsValue`.
            // Built via `HirExpr::JsonObjectLit` -- an existing, already-
            // exercised construct (a dictionary literal, `Bool` element
            // type covers the one `true` field) -- rather than new
            // codegen, avoiding the same int-value-kind ambiguity with
            // `JsValue`'s own placeholder detection (both would compile
            // to a plain integer at the LLVM level) a bare pass-through
            // like the `JsValue` case above would risk. `value` itself
            // is still evaluated once for any side effect it might have
            // (`wrap_call_argument_bindings`), even though `Undefined`
            // has only one possible runtime value and the result doesn't
            // reference it.
            if actual == HirType::Undefined {
                let temp = format!("__thaw_json_undefined_source_{}", self.next_binding);
                self.next_binding += 1;
                let sentinel = HirExpr::JsonObjectLit(
                    vec![(
                        "$__thaw_napi_undefined$".to_string(),
                        HirExpr::Lit(HirLit::Bool(true)),
                    )],
                    HirType::Bool,
                );
                return self.wrap_call_argument_bindings(
                    sentinel,
                    &[(temp, HirType::Undefined, value)],
                );
            }
            if let HirType::Map(key, element) = &actual {
                if !json_convertible_native_type(key) || !json_convertible_native_type(element) {
                    return Err(format!(
                        "Map<{key:?}, {element:?}> cannot cross a dynamic boundary"
                    ));
                }
                let entries_type = HirType::Array(Box::new(HirType::Tuple(vec![
                    key.as_ref().clone(),
                    element.as_ref().clone(),
                ])));
                let entries = HirExpr::TypedClosure(
                    entries_type.clone(),
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_map_snapshot_entries".into())),
                        vec![value],
                    )),
                );
                let entries = self.wrap_native_value_as_json(entries, entries_type)?;
                return Ok(HirExpr::JsonObjectLit(
                    vec![("__thaw_map_entries__".into(), entries)],
                    HirType::Json,
                ));
            }
            if let HirType::Set(element) = &actual {
                if !json_convertible_native_type(element) {
                    return Err(format!(
                        "Set<{element:?}> cannot cross a dynamic boundary"
                    ));
                }
                let values_type = HirType::Array(element.clone());
                let values = HirExpr::TypedClosure(
                    values_type.clone(),
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_map_snapshot_keys".into())),
                        vec![value],
                    )),
                );
                let values = self.wrap_native_value_as_json(values, values_type)?;
                return Ok(HirExpr::JsonObjectLit(
                    vec![("__thaw_set_values__".into(), values)],
                    HirType::Json,
                ));
            }
            if actual == regex_object_type() {
                let pattern = self.wrap_native_value_as_json(value, actual)?;
                return Ok(HirExpr::JsonObjectLit(
                    vec![("__thaw_regexp__".into(), pattern)],
                    HirType::Json,
                ));
            }
            if json_convertible_native_type(&actual) {
                return self.wrap_native_value_as_json(value, actual);
            }
        }
        if let Some(adapted) = self.adapt_named_function_to_callable(declared, &value)? {
            return Ok(adapted);
        }
        if let HirType::CallableFunction(fixed, _, rest, ret) = declared {
            let actual = self.infer_expr_type(&value)?;
            let mut abi_params = fixed.clone();
            if let Some(rest) = rest {
                abi_params.push(HirType::Array(rest.clone()));
            }
            if actual == HirType::Function(abi_params, ret.clone()) {
                return Ok(HirExpr::TypedClosure(declared.clone(), Box::new(value)));
            }
        }
        if let HirType::Dictionary(element) = declared {
            if self.infer_expr_type(&value)? == *declared {
                return Ok(value);
            }
            let HirExpr::ObjectLit(fields) = value else {
                return Err(format!(
                    "dictionary value must be an object literal with {element:?} values"
                ));
            };
            let fields = fields
                .into_iter()
                .map(|(name, value)| Ok((name, self.coerce_to_declared(element.as_ref(), value)?)))
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::JsonObjectLit(fields, element.as_ref().clone()));
        }
        if let HirType::Union(elements) = declared {
            let actual = self.infer_expr_type(&value)?;
            if &actual == declared {
                return Ok(value);
            }
            if let HirType::Union(source) = &actual {
                if equivalent_union_members(source, elements) {
                    let parameter = "__thaw_union_retag_value".to_string();
                    let mut statements = Vec::with_capacity(source.len());
                    for (source_index, member) in source.iter().enumerate() {
                        let target_index = elements
                            .iter()
                            .position(|target| target == member)
                            .expect("equivalent union contains every source member");
                        let converted = HirExpr::UnionInject(
                            Box::new(HirExpr::UnionValue(
                                Box::new(HirExpr::Var(parameter.clone())),
                                source_index,
                                source.clone(),
                            )),
                            target_index,
                            elements.clone(),
                        );
                        if source_index + 1 == source.len() {
                            statements.push(HirStmt::Return(Some(converted)));
                        } else {
                            statements.push(HirStmt::If(
                                HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::UnionTag(
                                        Box::new(HirExpr::Var(parameter.clone())),
                                        source.clone(),
                                    )),
                                    Box::new(HirExpr::Lit(HirLit::F64(source_index as f64))),
                                ),
                                vec![HirStmt::Return(Some(converted))],
                                Vec::new(),
                            ));
                        }
                    }
                    let adapter = HirExpr::Lambda(
                        Vec::new(),
                        vec![HirParam {
                            name: parameter,
                            ty: actual,
                        }],
                        declared.clone(),
                        Box::new(HirExpr::Block(statements)),
                    );
                    return Ok(HirExpr::Call(Box::new(adapter), vec![value]));
                }
            }
            if let Some(index) = elements.iter().position(|element| element == &actual) {
                return Ok(HirExpr::UnionInject(
                    Box::new(value),
                    index,
                    elements.clone(),
                ));
            }
            // A `T | null` / `T | undefined` (`T | null | undefined`)
            // value flowing into a wider union that already covers every
            // one of those cases -- e.g. a `.d.ts` overload whose own
            // return is `string | null` reaching the merged
            // `string | number | null` signature thaw builds for the
            // whole function. Re-tag it: test the presence flag, then
            // `UnionInject` the payload or the absent literal into the
            // matching slot.
            if let Some(retagged) = self.retag_nullish_into_union(&actual, elements, &value)? {
                return Ok(retagged);
            }
            if matches!(value, HirExpr::ObjectLit(_)) {
                for (index, member) in elements.iter().enumerate() {
                    if !matches!(member, HirType::Object(_)) {
                        continue;
                    }
                    if let Ok(adapted) = self.coerce_to_declared(member, value.clone()) {
                        return Ok(HirExpr::UnionInject(
                            Box::new(adapted),
                            index,
                            elements.clone(),
                        ));
                    }
                }
            }
            return Err(format!(
                "value has type {actual:?}, which is not a member of {declared:?}"
            ));
        }
        if let HirType::Optional(payload) = declared {
            return match self.infer_expr_type(&value)? {
                HirType::Undefined => Ok(HirExpr::OptionalNone(payload.as_ref().clone())),
                actual if actual == *declared => Ok(value),
                _ => {
                    let value = self.coerce_to_declared(payload.as_ref(), value)?;
                    Ok(HirExpr::OptionalSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let HirType::Nullable(payload) = declared {
            return match self.infer_expr_type(&value)? {
                HirType::Null => Ok(HirExpr::NullableNone(payload.as_ref().clone())),
                actual if actual == *declared => Ok(value),
                _ => {
                    let value = self.coerce_to_declared(payload.as_ref(), value)?;
                    Ok(HirExpr::NullableSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let HirType::Nullish(payload) = declared {
            return match self.infer_expr_type(&value)? {
                HirType::Null => Ok(HirExpr::NullishNull(payload.as_ref().clone())),
                HirType::Undefined => Ok(HirExpr::NullishUndefined(payload.as_ref().clone())),
                actual if actual == *declared => Ok(value),
                _ => {
                    let value = self.coerce_to_declared(payload.as_ref(), value)?;
                    Ok(HirExpr::NullishSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let (HirType::Array(element), HirExpr::ArrayLit(values)) = (declared, &value) {
            let values = values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    self.coerce_to_declared(element, value.clone())
                        .map_err(|error| format!("array element {index}: {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(HirExpr::ArrayLit(values));
        }
        if let (HirType::Tuple(expected), HirExpr::ArrayLit(values)) = (declared, &value) {
            let required = expected
                .iter()
                .take_while(|ty| !matches!(ty, HirType::Optional(_)))
                .count();
            if values.len() < required || values.len() > expected.len() {
                return Err(format!(
                    "tuple literal has {} element(s), expected {required}..={}",
                    values.len(),
                    expected.len()
                ));
            }
            let values = expected
                .iter()
                .zip(values.iter().cloned().chain(std::iter::repeat(HirExpr::Lit(
                    HirLit::Undefined,
                ))))
                .enumerate()
                .map(|(index, (expected, value))| {
                    self.coerce_to_declared(expected, value.clone())
                        .map_err(|error| format!("tuple element {index}: {error}"))
                })
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::ArrayLit(values));
        }
        let (HirType::Object(declared_fields), HirExpr::ObjectLit(lit_fields)) = (declared, &value)
        else {
            self.expect_type(declared, &value, "value")?;
            return Ok(value);
        };

        if lit_fields.len() > declared_fields.len()
            || lit_fields
                .iter()
                .any(|(name, _)| !declared_fields.iter().any(|(declared, _)| declared == name))
        {
            return Err(format!(
                "object literal has {} field(s), expected {} for this type",
                lit_fields.len(),
                declared_fields.len()
            ));
        }

        let reordered = declared_fields
            .iter()
            .map(|(name, expected_ty)| {
                let Some((_, field_value)) = lit_fields.iter().find(|(n, _)| n == name) else {
                    let value = omitted_parameter_value(expected_ty)
                        .map_err(|_| format!("object literal is missing required property `{name}`"))?;
                    return Ok((name.clone(), value));
                };
                let field_value = self
                    .coerce_to_declared(expected_ty, field_value.clone())
                    .map_err(|error| format!("field `{name}`: {error}"))?;
                Ok((name.clone(), field_value))
            })
            .collect::<Result<Vec<_>, String>>()?;

        Ok(HirExpr::ObjectLit(reordered))
    }

    /// Re-tags an `Optional`/`Nullable`/`Nullish` value into a `Union`
    /// that already contains its payload type and its absent form(s)
    /// (`Undefined` / `Null`). Returns `None` when `actual` isn't one of
    /// those, or the union doesn't fully cover it (so the caller can fall
    /// through to its existing error).
    fn retag_nullish_into_union(
        &self,
        actual: &HirType,
        elements: &[HirType],
        value: &HirExpr,
    ) -> Result<Option<HirExpr>, String> {
        let slot = |ty: &HirType| elements.iter().position(|element| element == ty);
        // (payload type, absent-form arms as (is-none test, injected literal))
        let (payload, arms): (&HirType, Vec<(HirExpr, HirLit)>) = match actual {
            HirType::Nullable(payload) => (
                payload.as_ref(),
                vec![(
                    HirExpr::NullableIsNone(
                        Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                        payload.as_ref().clone(),
                    ),
                    HirLit::Null,
                )],
            ),
            HirType::Optional(payload) => (
                payload.as_ref(),
                vec![(
                    HirExpr::OptionalIsNone(
                        Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                        payload.as_ref().clone(),
                    ),
                    HirLit::Undefined,
                )],
            ),
            HirType::Nullish(payload) => (
                payload.as_ref(),
                vec![
                    (
                        HirExpr::NullishIsNull(
                            Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                            payload.as_ref().clone(),
                        ),
                        HirLit::Null,
                    ),
                    (
                        HirExpr::NullishIsUndefined(
                            Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                            payload.as_ref().clone(),
                        ),
                        HirLit::Undefined,
                    ),
                ],
            ),
            _ => return Ok(None),
        };
        let Some(payload_slot) = slot(payload) else {
            return Ok(None);
        };
        let mut absent_slots = Vec::with_capacity(arms.len());
        for (_, literal) in &arms {
            let absent_ty = match literal {
                HirLit::Null => HirType::Null,
                HirLit::Undefined => HirType::Undefined,
                _ => unreachable!("retag arms only carry Null/Undefined"),
            };
            match slot(&absent_ty) {
                Some(index) => absent_slots.push(index),
                None => return Ok(None),
            }
        }

        let payload_value = match actual {
            HirType::Nullable(payload) => HirExpr::NullableValue(
                Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                payload.as_ref().clone(),
            ),
            HirType::Optional(payload) => HirExpr::OptionalValue(
                Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                payload.as_ref().clone(),
            ),
            HirType::Nullish(payload) => HirExpr::NullishValue(
                Box::new(HirExpr::Var("__thaw_nullish_retag".into())),
                payload.as_ref().clone(),
            ),
            _ => unreachable!(),
        };
        let mut statements = Vec::with_capacity(arms.len() + 1);
        for ((test, literal), absent_slot) in arms.into_iter().zip(absent_slots) {
            statements.push(HirStmt::If(
                test,
                vec![HirStmt::Return(Some(HirExpr::UnionInject(
                    Box::new(HirExpr::Lit(literal)),
                    absent_slot,
                    elements.to_vec(),
                )))],
                Vec::new(),
            ));
        }
        statements.push(HirStmt::Return(Some(HirExpr::UnionInject(
            Box::new(payload_value),
            payload_slot,
            elements.to_vec(),
        ))));
        let adapter = HirExpr::Lambda(
            Vec::new(),
            vec![HirParam {
                name: "__thaw_nullish_retag".into(),
                ty: actual.clone(),
            }],
            HirType::Union(elements.to_vec()),
            Box::new(HirExpr::Block(statements)),
        );
        Ok(Some(HirExpr::Call(Box::new(adapter), vec![value.clone()])))
    }
}
