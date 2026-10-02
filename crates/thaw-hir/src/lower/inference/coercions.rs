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
        if *declared == HirType::Str {
            let actual = self.infer_expr_type(&value)?;
            if object_type_is_error_family(&actual) {
                return self.coerce_primitive_to_string(value);
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
        // `arr[i]`'s hole/out-of-bounds-aware read is typed `Optional<T>`,
        // so a callback that returns one makes `.map`/`.flatMap` produce
        // `Array<Optional<T>>`. Assigning that to a declared `T[]` has no
        // natural coercion -- real trigger: `const words: string[] =
        // Array.from(s.matchAll(re)).map((m) => m[0])`, which broke once
        // array reads became hole-aware. Unwrap element-wise through the
        // existing array-map lowerer, reusing the same `Optional<T> -> T`
        // scalar coercion (`undefined` -> "undefined"/NaN) already applied
        // to `const value: T = arr[i]` assignments.
        if let HirType::Array(declared_element) = declared {
            if !matches!(value, HirExpr::ArrayLit(_)) {
                if let HirType::Array(actual_element) = &self.infer_expr_type(&value)? {
                    if let HirType::Optional(payload) = actual_element.as_ref() {
                        if payload.as_ref() == declared_element.as_ref()
                            && matches!(payload.as_ref(), HirType::F64 | HirType::Str)
                        {
                            let parameter = format!("__thaw_array_unwrap_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope
                                .insert(parameter.clone(), actual_element.as_ref().clone());
                            let bound = HirExpr::Var(parameter.clone());
                            let body = match payload.as_ref() {
                                HirType::F64 => self.coerce_primitive_to_number(bound)?,
                                _ => self.coerce_primitive_to_string(bound)?,
                            };
                            let callback = HirExpr::Lambda(
                                Vec::new(),
                                vec![HirParam {
                                    name: parameter,
                                    ty: actual_element.as_ref().clone(),
                                }],
                                payload.as_ref().clone(),
                                Box::new(body),
                            );
                            return self.lower_array_map(
                                value,
                                HirType::Array(actual_element.clone()),
                                actual_element.as_ref().clone(),
                                actual_element.as_ref().clone(),
                                callback,
                                None,
                            );
                        }
                    }
                }
            }
        }
        let inferred = self.infer_expr_type(&value)?;
        // A callback value whose parameter object types are a width-subtype
        // (prefix) of the declared callback's -- e.g. a `createServer`
        // handler annotated with a narrower request/response than
        // `IncomingMessage`/`ServerResponse` -- is layout-compatible and
        // needs no adapter. Deliberately narrower than
        // `callable_value_compatible`: an `Optional`/`CallableFunction`
        // difference still falls through to the adapter path (e.g. a
        // default-argument wrapper).
        if callable_abi_compatible(declared, &inferred) {
            return Ok(value);
        }
        if *declared == HirType::JsValue && matches!(inferred, HirType::Function(_, _)) {
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("registerNativeCallback".to_string())),
                vec![HirExpr::TypedClosure(inferred, Box::new(value))],
            ));
        }
        if *declared == HirType::JsValue
            && matches!(
                self.infer_expr_type(&value)?,
                HirType::Array(_)
                    | HirType::Tuple(_)
                    | HirType::Object(_)
                    | HirType::Dictionary(_)
            )
        {
            let json = self.coerce_to_declared(&HirType::Json, value)?;
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("retainDynamicJson".to_string())),
                vec![json],
            ));
        }
        // The symmetric case to the `HirType::JsValue`-into-`Json` branch
        // above: a value whose own real, live type genuinely is `JsValue`
        // (e.g. a real npm class instance) can end up statically typed
        // `Json` instead -- real trigger: `const rs: any = new ReadStream
        // (...)`, an explicit `: any` annotation, whose declared type
        // lowers to `HirType::Json` regardless of the initializer's own
        // real type. Calling a method on it later needs a real `JsValue`
        // handle again, but `rs`'s stored type says `Json`. `JsValueAsJson`
        // above always encodes a live handle as exactly the same
        // `{"__thaw_js_handle_id__": id}` placeholder object
        // `compile_dynamic_value_placeholder` builds for the opposite
        // direction -- `thaw_json_handle_id` (thaw-std) is the existing,
        // already-wired native-side reader for that exact shape (used
        // today only for a `JsValue`-typed native-callback argument
        // decoded from real JSON, `compile_json_value_to_native`'s own
        // `HirType::JsValue` arm) -- reused here via `JsonAsNative`'s own
        // codegen (`compile_json_to_native`, extended with the identical
        // one case) to recover the live handle a `Json`-typed value that
        // is genuinely this placeholder shape was never really without.
        if *declared == HirType::JsValue && self.infer_expr_type(&value)? == HirType::Json {
            return Ok(HirExpr::JsonAsNative(Box::new(value), HirType::JsValue));
        }
        if let (HirType::Object(declared_fields), HirType::Object(actual_fields)) =
            (declared, self.infer_expr_type(&value)?)
        {
            if actual_fields.as_slice() != declared_fields.as_slice()
                && actual_fields.iter().any(|(name, _)| is_hidden_accessor_field(name))
            {
                if let HirExpr::ObjectLit(fields) = &value {
                    let mut reordered = declared_fields
                        .iter()
                        .map(|(name, expected)| {
                            let (_, field) = fields
                                .iter()
                                .find(|(field, _)| field == name)
                                .ok_or_else(|| {
                                    format!("object literal is missing required property `{name}`")
                                })?;
                            Ok((
                                name.clone(),
                                self.coerce_to_declared(expected, field.clone())?,
                            ))
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    reordered.extend(
                        fields
                            .iter()
                            .filter(|(name, _)| is_hidden_accessor_field(name))
                            .cloned(),
                    );
                    return Ok(HirExpr::ObjectLit(reordered));
                }
                let visible = actual_fields
                    .iter()
                    .filter(|(name, _)| !is_hidden_accessor_field(name))
                    .collect::<Vec<_>>();
                if visible
                    .iter()
                    .zip(declared_fields)
                    .all(|((name, actual), (declared, expected))| {
                        name == declared && actual == expected
                    })
                    && visible.len() == declared_fields.len()
                {
                    return Ok(value);
                }
                return Err("cannot reorder a non-literal accessor object layout".into());
            }
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
            // schema instance) has no direct JSON representation. Wrapped
            // via `HirExpr::JsValueAsJson` -- a real, valid `Json` value
            // (the same `{"__thaw_js_handle_id__": <id>}` placeholder
            // object `compile_dynamic_value_placeholder` already builds
            // for a `JsValue` crossing into a dynamic call's own JSON
            // argument array; the QuickJS-side reviver splices the real
            // live value back in whenever this placeholder is later
            // JSON-parsed there, same as it already does for a `Date`) --
            // rather than left as the raw, unwrapped handle. `HirType::
            // Json` and `HirType::JsValue` have different native layouts
            // (an opaque pointer to a boxed `serde_json::Value` vs. a
            // plain `i64` handle): simply returning `value` unchanged here
            // produced a value whose own inferred type stayed `JsValue`
            // while the declared slot said `Json` -- undetected by a
            // `let`/`const` declaration (nothing re-validates a coerced
            // initializer's own type against the variable's declared
            // one), and a segfault the moment anything later dereferenced
            // the raw handle bits as if they were a real `Json` pointer
            // (real trigger: csv-parse's `parser.read(): Json` assigned
            // to an already-`Json`-typed local, then passed to `Array.
            // push`/`JSON.stringify`). thaw-llvm's own dynamic-call
            // argument marshaling (`json_bridge.rs`'s own call sites)
            // still separately detects a bare `JsValue` handle reaching a
            // `Json`-declared parameter slot by LLVM value kind (an `i64`
            // where a pointer is expected) and wraps it the same way --
            // this node just does the identical wrapping earlier, at
            // `coerce_to_declared` time, so every other consumer (not
            // just a dynamic call's own argument list) gets a genuinely
            // valid `Json` value too.
            //
            // A statically-`JsValue`-typed value can still turn out, at
            // runtime, to genuinely *be* `null` or `undefined` (real
            // trigger: `Readable.read(): Json`, whose real runtime
            // result -- despite the declared type -- is `null` once the
            // stream is exhausted, the exact value Node's own docs say
            // to loop on: `while ((record = parser.read()) !== null)`).
            // The placeholder-object encoding above is only correct for
            // a genuinely live object; `null`/`undefined` need the real
            // `Json` literal instead, or `record !== null` would never
            // observe termination (a wrapped placeholder object is never
            // `===` to a literal `null`) -- an infinite loop, not a
            // crash, but just as wrong. Checked at runtime (the same
            // live-engine query `dynamic_value_is_null`/`_undefined`
            // already use for `JsValue === null`/`undefined` comparisons
            // elsewhere) since nothing static can know which case a
            // given call actually returns.
            if actual == HirType::JsValue {
                let temp = format!("__thaw_json_wrap_source_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(temp.clone(), HirType::JsValue);
                let null_json =
                    self.wrap_native_value_as_json(HirExpr::Lit(HirLit::Null), HirType::Null)?;
                let undefined_json = HirExpr::JsonObjectLit(
                    vec![(
                        "$__thaw_napi_undefined$".to_string(),
                        HirExpr::Lit(HirLit::Bool(true)),
                    )],
                    HirType::Bool,
                );
                let is_null = self.dynamic_value_is_null(HirExpr::Var(temp.clone()));
                let is_undefined = self.dynamic_value_is_undefined(HirExpr::Var(temp.clone()));
                let body = HirExpr::Conditional(
                    Box::new(is_null),
                    Box::new(null_json),
                    Box::new(HirExpr::Conditional(
                        Box::new(is_undefined),
                        Box::new(undefined_json),
                        Box::new(HirExpr::JsValueAsJson(Box::new(HirExpr::Var(temp.clone())))),
                        HirType::Json,
                    )),
                    HirType::Json,
                );
                return self.wrap_call_argument_bindings(body, &[(temp, HirType::JsValue, value)]);
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
            // A generator's own internal "producer" function (thaw's own
            // ABI, no dedicated `HirType` -- see `is_generator_producer_
            // type`'s own doc comment, `lower/generators.rs`) is *not* a
            // plain callable a JS caller could ever legitimately invoke
            // directly (its first parameter is an internal `i64` resume
            // point, not a real argument) -- reject it here, before it
            // falls into the generic native-closure-to-`JsValue`
            // wrapping path below. That generic path has no way to
            // expose an `i64` parameter to real JS code and previously
            // failed confusingly deep inside dynamic-call argument
            // decoding ("unsupported dynamic result value I64") instead
            // of at the coercion itself -- `Promise<T>` coerced to `any`
            // similarly has no dedicated case anywhere in this function
            // and simply falls through every special case here to the
            // generic type-mismatch error at the very end, which is at
            // least an honest "unsupported" rather than a confusing
            // internal-ABI leak; this is the same fix in spirit for
            // `Generator`, just needed an explicit early check instead
            // of falling through cleanly, since the generic `Function`
            // case right below *would* otherwise (wrongly) accept it.
            if is_generator_producer_type(&actual) {
                return Err(
                    "cannot coerce a Generator to a dynamic (any) value".into(),
                );
            }
            if let HirType::Function(_, _) = &actual {
                // `registerNativeCallback`'s own inferred type is always
                // `HirType::JsValue` (`inference/types.rs`'s hardcoded
                // intrinsic-name case) -- a raw `i64` handle, not the
                // pointer representation a genuinely `Json`-typed value
                // needs. Reaching a dynamic call's own JSON argument
                // array/object field already tolerates this raw shape
                // (that call site's own codegen separately detects an
                // `i64` reaching a `Json`-declared slot via `is_int_
                // value()` and wraps it there, see `compile_json_
                // object_set_native_with_undefined`'s `HirType::Json`
                // arm) -- but nothing else does. A `let`/`const c: any
                // = someFunctionOrClass;` (real trigger: any plain
                // function *or* class reference -- `new User(...)`
                // itself desugars to an ordinary function call, so a
                // bare `User` reference hits this exact branch too --
                // coerced into an `any`-typed local) stored this raw
                // `i64` directly into a slot whose declared type says
                // `Json` (a pointer), corrupting it -- a real,
                // reproducible segfault the moment anything (`typeof`,
                // `console.log`, ...) later read that slot back as if
                // it held a real `serde_json::Value` pointer. `JsValueAsJson`
                // (the same wrapping the sibling `actual == JsValue`
                // case below already applies) produces the identical
                // `{"__thaw_js_handle_id__": id}` placeholder `compile_
                // dynamic_value_placeholder`'s own `is_int_value()` path
                // would otherwise build downstream -- doing it here
                // instead makes every consumer correct, the dynamic-
                // call-argument case included (its own `is_int_value()`
                // check now simply sees an already-wrapped pointer and
                // skips, never double-wrapping).
                return Ok(HirExpr::JsValueAsJson(Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("registerNativeCallback".to_string())),
                    vec![HirExpr::TypedClosure(actual.clone(), Box::new(value))],
                ))));
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
            // A `Buffer`/`Uint8Array` (`HirType::Bytes`) has no direct
            // JSON representation either -- real trigger: archiver's own
            // `Archiver.append(source: Readable | Buffer | string, ...)`,
            // called with a real `Buffer.from(...)` value. `Bytes` is a
            // lowering-time-only distinction (`lower/bytes_erasure.rs`
            // rewrites every one, program-wide, to a plain `Array(F64)`
            // *after* lowering finishes) -- deliberately *not* added to
            // `json_convertible_native_type`'s own generic native-array
            // path below, since that would just JSON-encode the bytes as
            // a bare `[n1, n2, ...]` array, indistinguishable from an
            // ordinary `number[]` argument and silently losing "this was
            // a Buffer" the same way it already was. Wrapped instead as
            // the same `{"type":"Buffer","data":[...]}` shape Node's own
            // `Buffer.prototype.toJSON`/`JSON.stringify` already use --
            // `__thaw_json_date_reviver` (`platform_globals/dates.js`)
            // already recognizes this exact shape and reconstructs a
            // real `Buffer` from it (previously only reachable via a
            // native addon's own JSON round trip, never a `callDynamic`
            // argument). The `data` field's own elements erasing to plain
            // JSON numbers is correct and expected here -- the `"type"`
            // wrapper is what carries "this was a Buffer", not the
            // erased element type, so losing `Bytes` as a type tag after
            // lowering doesn't lose the information this fix depends on.
            // `infer_expr_type` (used for `actual` above) deliberately
            // normalizes `Bytes` to `Array(F64)` for every other consumer
            // (see its own doc comment) -- `infer_expr_type_inner` is the
            // one place that still sees the real, un-normalized tag,
            // needed here specifically to tell a `Buffer`/`Uint8Array`
            // apart from an ordinary `number[]` of the same runtime
            // layout.
            if self.infer_expr_type_inner(&value)? == HirType::Bytes {
                let data = self.wrap_native_value_as_json(value, HirType::Bytes)?;
                let kind = self.wrap_native_value_as_json(
                    HirExpr::Lit(HirLit::Str("Buffer".to_string())),
                    HirType::Str,
                )?;
                return Ok(HirExpr::JsonObjectLit(
                    vec![("type".into(), kind), ("data".into(), data)],
                    HirType::Json,
                ));
            }
            // An instance of a *decorated* class (see `module/globals.rs`'s
            // `lower_class_decorator_tokens`/`DECORATOR_CLASS_TOKENS`) gets
            // one extra `constructor` field pointing at that class's own
            // real, live "class token" `JsValue` -- the exact same token a
            // decorator's own `target` argument was built from
            // (`lower_member_decorator_call`/`lower_class_decorator_call`).
            // That shared identity is what lets real `object.constructor`-
            // keyed metadata storage (class-validator's own
            // `MetadataStorage`, keyed by real `Map` reference equality)
            // find its own registration again once the instance crosses
            // this same dynamic-call boundary a second time (e.g.
            // `validate(instance)`). Scoped to only classes that actually
            // have a decorator -- an ordinary class's instances keep
            // marshaling exactly as before, so this can't regress any
            // existing (pre-decorator) dynamic-call argument shape.
            if let HirType::Object(fields) = &actual {
                if let Some(token_symbol) = fields.first().and_then(|(marker, _)| {
                    DECORATOR_CLASS_TOKENS.with(|tokens| tokens.borrow().get(marker).cloned())
                }) {
                    let object_json = self.wrap_native_value_as_json(value, actual.clone())?;
                    let temp = format!("__thaw_decorator_instance_json_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(temp.clone(), HirType::Json);
                    let token_as_json =
                        self.coerce_to_declared(&HirType::Json, HirExpr::Var(token_symbol))?;
                    let result = HirExpr::Block(vec![
                        HirStmt::Expr(HirExpr::JsonSet(
                            Box::new(HirExpr::Var(temp.clone())),
                            Box::new(HirExpr::Lit(HirLit::Str("constructor".to_string()))),
                            Box::new(token_as_json),
                            HirType::Json,
                            false,
                        )),
                        HirStmt::Return(Some(HirExpr::Var(temp.clone()))),
                    ]);
                    return self.wrap_call_argument_bindings(
                        result,
                        &[(temp, HirType::Json, object_json)],
                    );
                }
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
            let actual = self.infer_expr_type(&value)?;
            // `Dictionary<T>` shares `Json`'s own native layout at the
            // LLVM level (see `compile_json_value_to_native`'s own
            // `HirType::Dictionary(_) => Ok(json)` case, thaw-llvm's
            // `json_bridge/decoding.rs`) -- decoding a `Json`/`JsValue`
            // source into a `Dictionary` target is a pure type-level
            // relabeling, the same "readDynamicValue, then pass through
            // unchanged" shape the `Json`-target branch above already
            // uses for a `JsValue` source. Without this, a `Dictionary`-
            // declared slot fed a genuinely dynamic value (e.g. `new
            // Proxy(target, handler)`'s own `target` re-read through a
            // live QuickJS handle) hit the `ObjectLit`-only check below
            // and hard-errored instead.
            if matches!(actual, HirType::Json | HirType::JsValue) {
                return if actual == HirType::JsValue {
                    Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("readDynamicValue".to_string())),
                        vec![value],
                    ))
                } else {
                    Ok(value)
                };
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
        // A referenced union alias can nest an absence wrapper inside
        // another wrapper or a union. Flatten those tags before assigning
        // the canonical result layout; each branch still owns one value.
        if matches!(declared, HirType::Union(_) | HirType::Optional(_)
            | HirType::Nullable(_) | HirType::Nullish(_)) {
            let actual = self.infer_expr_type(&value)?;
            let nested_wrapper = match &actual {
                HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) => {
                    matches!(inner.as_ref(), HirType::Optional(_) | HirType::Nullable(_)
                        | HirType::Nullish(_))
                    || matches!(inner.as_ref(), HirType::Union(parts)
                        if parts.iter().any(|part| matches!(part, HirType::Optional(_)
                            | HirType::Nullable(_) | HirType::Nullish(_))))
                }
                _ => false,
            };
            let union_into_wrapper = matches!((&actual, declared),
                (HirType::Union(_), HirType::Optional(_) | HirType::Nullable(_)
                    | HirType::Nullish(_)));
            let union_with_nested_wrapper = matches!((&actual, declared),
                (HirType::Union(parts), HirType::Union(_)) if parts.iter().any(|part|
                    matches!(part, HirType::Optional(_) | HirType::Nullable(_)
                        | HirType::Nullish(_))));
            let direct_member = matches!(declared, HirType::Union(elements)
                if elements.contains(&actual));
            if actual != *declared && !direct_member && (nested_wrapper || union_into_wrapper || union_with_nested_wrapper) {
                return self.coerce_composite_to_declared(declared, value, actual);
            }
        }
        if let HirType::Union(elements) = declared {
            let actual = self.infer_expr_type(&value)?;
            if &actual == declared {
                return Ok(value);
            }
            if let HirType::Union(source) = &actual {
                // Widen a source union as well as reordering equivalent
                // members. Logical expressions can add a right-hand type
                // while retaining every left-hand alternative.
                if source.iter().all(|member| elements.contains(member)) {
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
            let actual = self.infer_expr_type(&value)?;
            // Widen a one-absence wrapper without losing which absence it
            // carried. This is needed when the two sides of `&&` contribute
            // null and undefined separately.
            if let HirType::Optional(source) | HirType::Nullable(source) = &actual {
                if source == payload {
                    let parameter = "__thaw_nullish_widen".to_string();
                    let left = HirExpr::Var(parameter.clone());
                    let (absent, absent_value, present_value) = match &actual {
                        HirType::Optional(_) => (
                            HirExpr::OptionalIsNone(Box::new(left.clone()), source.as_ref().clone()),
                            HirExpr::NullishUndefined(payload.as_ref().clone()),
                            HirExpr::OptionalValue(Box::new(left), source.as_ref().clone()),
                        ),
                        HirType::Nullable(_) => (
                            HirExpr::NullableIsNone(Box::new(left.clone()), source.as_ref().clone()),
                            HirExpr::NullishNull(payload.as_ref().clone()),
                            HirExpr::NullableValue(Box::new(left), source.as_ref().clone()),
                        ),
                        _ => unreachable!(),
                    };
                    let adapter = HirExpr::Lambda(
                        Vec::new(),
                        vec![HirParam { name: parameter, ty: actual }],
                        declared.clone(),
                        Box::new(HirExpr::Block(vec![
                            HirStmt::If(absent, vec![HirStmt::Return(Some(absent_value))], Vec::new()),
                            HirStmt::Return(Some(HirExpr::NullishSome(
                                Box::new(present_value), payload.as_ref().clone(),
                            ))),
                        ])),
                    );
                    return Ok(HirExpr::Call(Box::new(adapter), vec![value]));
                }
            }
            return match actual {
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
                    if matches!(value, HirExpr::Lit(HirLit::ArrayHole)) {
                        return Ok(value.clone());
                    }
                    self.coerce_array_insert_value(value.clone(), element)
                        .map_err(|error| format!("array element {index}: {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            return self.lower_native_array_literal(values, element.as_ref().clone());
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

    /// Normalize nested alias wrappers and unions one tag at a time. The
    /// value is captured as one lambda argument before any branch inspects
    /// it; recursive coercion only evaluates the selected payload.
    fn coerce_composite_to_declared(
        &mut self,
        declared: &HirType,
        value: HirExpr,
        source: HirType,
    ) -> Result<HirExpr, String> {
        let parameter = format!("__thaw_flatten_absence_{}", self.next_binding);
        self.next_binding += 1;
        let previous = self.scope.insert(parameter.clone(), source.clone());
        let result = (|| -> Result<HirExpr, String> {
            let current = HirExpr::Var(parameter.clone());
            let mut statements = Vec::new();
            match &source {
                HirType::Optional(payload) => {
                    let absent = self.coerce_to_declared(declared, HirExpr::Lit(HirLit::Undefined))?;
                    statements.push(HirStmt::If(HirExpr::OptionalIsNone(Box::new(current.clone()),
                        payload.as_ref().clone()), vec![HirStmt::Return(Some(absent))], Vec::new()));
                    let inner = HirExpr::OptionalValue(Box::new(current), payload.as_ref().clone());
                    statements.push(HirStmt::Return(Some(self.coerce_to_declared(declared, inner)?)));
                }
                HirType::Nullable(payload) => {
                    let absent = self.coerce_to_declared(declared, HirExpr::Lit(HirLit::Null))?;
                    statements.push(HirStmt::If(HirExpr::NullableIsNone(Box::new(current.clone()),
                        payload.as_ref().clone()), vec![HirStmt::Return(Some(absent))], Vec::new()));
                    let inner = HirExpr::NullableValue(Box::new(current), payload.as_ref().clone());
                    statements.push(HirStmt::Return(Some(self.coerce_to_declared(declared, inner)?)));
                }
                HirType::Nullish(payload) => {
                    let null = self.coerce_to_declared(declared, HirExpr::Lit(HirLit::Null))?;
                    statements.push(HirStmt::If(HirExpr::NullishIsNull(Box::new(current.clone()),
                        payload.as_ref().clone()), vec![HirStmt::Return(Some(null))], Vec::new()));
                    let undefined = self.coerce_to_declared(declared, HirExpr::Lit(HirLit::Undefined))?;
                    statements.push(HirStmt::If(HirExpr::NullishIsUndefined(Box::new(current.clone()),
                        payload.as_ref().clone()), vec![HirStmt::Return(Some(undefined))], Vec::new()));
                    let inner = HirExpr::NullishValue(Box::new(current), payload.as_ref().clone());
                    statements.push(HirStmt::Return(Some(self.coerce_to_declared(declared, inner)?)));
                }
                HirType::Union(members) => {
                    if members.is_empty() {
                        return Err("cannot coerce an empty union".into());
                    }
                    for (index, member) in members.iter().enumerate() {
                        let inner = HirExpr::UnionValue(Box::new(current.clone()), index, members.clone());
                        let adapted = self.coerce_to_declared(declared, inner)?;
                        if index + 1 == members.len() {
                            statements.push(HirStmt::Return(Some(adapted)));
                        } else {
                            statements.push(HirStmt::If(HirExpr::BinOp(BinOp::EqEqEq,
                                Box::new(HirExpr::UnionTag(Box::new(current.clone()), members.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64)))),
                                vec![HirStmt::Return(Some(adapted))], Vec::new()));
                        }
                    }
                }
                _ => unreachable!("composite coercion requires a wrapper or union"),
            }
            let adapter = HirExpr::Lambda(Vec::new(), vec![HirParam { name: parameter.clone(), ty: source }],
                declared.clone(), Box::new(HirExpr::Block(statements)));
            Ok(HirExpr::Call(Box::new(adapter), vec![value]))
        })();
        match previous {
            Some(previous) => { self.scope.insert(parameter, previous); }
            None => { self.scope.remove(&parameter); }
        }
        result
    }

    /// Re-tags an `Optional`/`Nullable`/`Nullish` value into a `Union`
    /// that already contains its payload type and its absent form(s)
    /// (`Undefined` / `Null`). Returns `None` when `actual` isn't one of
    /// those, or the union doesn't fully cover it (so the caller can fall
    /// through to its existing error).
    fn retag_nullish_into_union(
        &mut self,
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
        // An alias can make the payload itself a union. The target may
        // spell its leaves separately, so re-tag that payload through the
        // ordinary subset-union adapter instead of requiring one slot.
        let payload_slot = slot(payload);
        if payload_slot.is_none() && !matches!(payload, HirType::Union(parts)
            if parts.iter().all(|part| slot(part).is_some())) {
            return Ok(None);
        }
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
        let tagged_payload = if let Some(payload_slot) = payload_slot {
            HirExpr::UnionInject(Box::new(payload_value), payload_slot, elements.to_vec())
        } else {
            let parameter = "__thaw_nullish_retag".to_string();
            let previous = self.scope.insert(parameter.clone(), actual.clone());
            let result = self.coerce_to_declared(&HirType::Union(elements.to_vec()), payload_value);
            match previous {
                Some(previous) => { self.scope.insert(parameter, previous); }
                None => { self.scope.remove(&parameter); }
            }
            result?
        };
        statements.push(HirStmt::Return(Some(tagged_payload)));
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
