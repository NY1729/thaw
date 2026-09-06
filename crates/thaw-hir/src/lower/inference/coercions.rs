impl<'a> FnLowerer<'a> {
    /// Coerces a value into its declared native layout.
    fn coerce_to_declared(&mut self, declared: &HirType, value: HirExpr) -> Result<HirExpr, String> {
        if *declared == HirType::Dynamic {
            return Ok(value);
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
            HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_)
        ) {
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
                return Ok(HirExpr::JsonAsNative(Box::new(value), declared.clone()));
            }
        }
        if let (HirType::Object(declared_fields), HirType::Object(actual_fields)) =
            (declared, self.infer_expr_type(&value)?)
        {
            if actual_fields.starts_with(declared_fields) {
                return Ok(value);
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
            if json_convertible_native_type(&actual) {
                return self.wrap_native_value_as_json(value, actual);
            }
        }
        if let Some(adapted) = self.adapt_named_function_to_callable(declared, &value)? {
            return Ok(adapted);
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
            if expected.len() != values.len() {
                return Err(format!(
                    "tuple literal has {} element(s), expected {}",
                    values.len(),
                    expected.len()
                ));
            }
            let values = expected
                .iter()
                .zip(values)
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
                    return Ok((name.clone(), omitted_parameter_value(expected_ty)?));
                };
                let field_value = self
                    .coerce_to_declared(expected_ty, field_value.clone())
                    .map_err(|error| format!("field `{name}`: {error}"))?;
                Ok((name.clone(), field_value))
            })
            .collect::<Result<Vec<_>, String>>()?;

        Ok(HirExpr::ObjectLit(reordered))
    }
}
