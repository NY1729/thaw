enum JsonTaggedKind {
    Optional,
    Nullable,
    Nullish,
}

impl<'ctx> HirCompiler<'ctx> {
    fn compile_json_backend_call(
        &mut self,
        args: &[HirExpr],
        backend_symbol: &str,
        source_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if backend_symbol.starts_with("thaw_js_") {
            self.uses_quickjs = true;
        }
        if backend_symbol.starts_with("thaw_napi_") {
            self.uses_napi = true;
        }
        let [name, call_args] = args else {
            return Err(format!(
                "{source_name} expects exactly two arguments (name, args)"
            ));
        };
        let name_val = self.compile_expr(name)?;
        // See the matching comment on `compile_call_dynamic_method`: a
        // `JsValue`-typed element nested in `call_args` (e.g. a
        // `registerNativeCallback` result) needs this flag set while it's
        // being marshaled, same as the method-call and typed-ambient-
        // declaration paths -- only meaningful for the QuickJS-NG backend,
        // N-API has no such reviver.
        let outer_compiling_quickjs_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        if backend_symbol.starts_with("thaw_js_") {
            self.compiling_quickjs_dynamic_arguments = true;
        }
        let args_json_val = self.compile_expr(call_args);
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let args_json_val = args_json_val?;

        self.compile_json_backend_values(name_val, args_json_val, backend_symbol)
    }

    fn compile_json_backend_values(
        &mut self,
        name_val: BasicValueEnum<'ctx>,
        args_json_val: BasicValueEnum<'ctx>,
        backend_symbol: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let stringify_fn = self.module.get_function("thaw_json_stringify").unwrap();
        let args_json_str = self
            .builder
            .build_call(
                stringify_fn,
                &[args_json_val.into()],
                "call_dynamic_args_json",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_stringify did not return a value")?;

        let call_fn = self.module.get_function(backend_symbol).unwrap();
        let result = self
            .builder
            .build_call(
                call_fn,
                &[name_val.into(), args_json_str.into()],
                "call_dynamic_result_abi",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("{backend_symbol} did not return a value"))?
            .into_struct_value();
        let result_json_str = self
            .builder
            .build_extract_value(result, 0, "call_dynamic_value")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        let error = self
            .builder
            .build_extract_value(result, 1, "call_dynamic_error")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|e| e.to_string())?;
        self.branch_on_pending_exception()?;

        let parse_fn = self.module.get_function("thaw_json_parse").unwrap();
        self.builder
            .build_call(parse_fn, &[result_json_str.into()], "call_dynamic_result")
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse did not return a value".to_string())
    }

    fn compile_json_as_value(
        &mut self,
        json: BasicValueEnum<'ctx>,
        symbol: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.builder
            .build_call(
                self.module.get_function(symbol).unwrap(),
                &[json.into()],
                symbol,
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("{symbol} returned no value"))
    }

    fn compile_json_as_bool_value(
        &mut self,
        json: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self
            .compile_json_as_value(json, "thaw_json_as_bool")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                value,
                self.context.i8_type().const_zero(),
                "dynamic_bool_result",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_native_object_to_json(
        &mut self,
        object: PointerValue<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_native_object_to_json_with_undefined(object, ty, false)
    }

    fn compile_native_object_to_json_with_undefined(
        &mut self,
        object: PointerValue<'ctx>,
        ty: &HirType,
        preserve_undefined: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirType::Object(fields) = ty else {
            return Err("dynamic object marshaling requires an object type".to_string());
        };
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_object_new").unwrap(),
                &[],
                "dynamic_object_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        for (index, (name, field_ty)) in fields.iter().enumerate() {
            let offset = self
                .context
                .i64_type()
                .const_int(object_field_offset(fields, index), false);
            let pointer = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), object, &[offset], "marshal_field")
                    .map_err(|error| error.to_string())?
            };
            let value = self
                .builder
                .build_load(self.basic_type(field_ty)?, pointer, "marshal_field_value")
                .map_err(|error| error.to_string())?;
            let key = self
                .builder
                .build_global_string_ptr(name, "dynamic_object_key")
                .map_err(|error| error.to_string())?;
            self.compile_json_object_set_native_with_undefined(
                json,
                key.as_pointer_value(),
                value,
                field_ty,
                preserve_undefined,
            )?;
        }
        Ok(json)
    }

    fn compile_json_object_set_native(
        &mut self,
        json: BasicValueEnum<'ctx>,
        key: PointerValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        field_type: &HirType,
    ) -> Result<(), String> {
        self.compile_json_object_set_native_with_undefined(json, key, value, field_type, false)
    }

    fn compile_json_object_set_native_with_undefined(
        &mut self,
        json: BasicValueEnum<'ctx>,
        key: PointerValue<'ctx>,
        mut value: BasicValueEnum<'ctx>,
        field_type: &HirType,
        preserve_undefined: bool,
    ) -> Result<(), String> {
        match field_type {
            HirType::Optional(payload) => {
                return self.compile_json_object_set_tagged(
                    json,
                    key,
                    value.into_struct_value(),
                    payload,
                    JsonTaggedKind::Optional,
                    preserve_undefined,
                );
            }
            HirType::Nullable(payload) => {
                return self.compile_json_object_set_tagged(
                    json,
                    key,
                    value.into_struct_value(),
                    payload,
                    JsonTaggedKind::Nullable,
                    preserve_undefined,
                );
            }
            HirType::Nullish(payload) => {
                return self.compile_json_object_set_tagged(
                    json,
                    key,
                    value.into_struct_value(),
                    payload,
                    JsonTaggedKind::Nullish,
                    preserve_undefined,
                );
            }
            HirType::Union(elements) => {
                return self.compile_json_object_set_union(
                    json,
                    key,
                    value.into_struct_value(),
                    elements,
                    preserve_undefined,
                );
            }
            HirType::Undefined => return Ok(()),
            HirType::Null => value = self.compile_json_null()?,
            _ => {}
        }
        let setter = match field_type {
                HirType::F64 => "thaw_json_object_set_number",
                HirType::Str => "thaw_json_object_set_string",
                HirType::Bool => {
                    value = self
                        .builder
                        .build_int_z_extend(
                            value.into_int_value(),
                            self.context.i8_type(),
                            "marshal_object_bool",
                        )
                        .map_err(|error| error.to_string())?
                        .into();
                    "thaw_json_object_set_bool"
                }
                HirType::Json | HirType::Dictionary(_) => {
                    // A field whose *declared* type is the generic `Json`
                    // (an unresolvable/fallback npm parameter type, e.g.
                    // zod's `optional`) can still carry a real `JsValue`
                    // handle at runtime -- thaw-hir's `coerce_to_declared`
                    // passes one through unchanged rather than erroring
                    // (see its own doc comment). `Json`/`Dictionary` are
                    // always pointers (see `basic_type`); a `JsValue` is
                    // always an `i64`, so this is an unambiguous way to
                    // tell the two apart here.
                    if value.is_int_value() {
                        value = self.compile_dynamic_value_placeholder(value)?;
                    }
                    "thaw_json_object_set_json"
                }
                HirType::Array(element) => {
                    value = self.compile_native_array_to_json_with_undefined(
                        value.into_pointer_value(),
                        element,
                        preserve_undefined,
                    )?;
                    "thaw_json_object_set_json"
                }
                HirType::Tuple(elements) => {
                    value = self.compile_native_tuple_to_json_with_undefined(
                        value.into_pointer_value(),
                        elements,
                        preserve_undefined,
                    )?;
                    "thaw_json_object_set_json"
                }
                HirType::Object(_) => {
                    value = self.compile_native_object_to_json_with_undefined(
                        value.into_pointer_value(),
                        field_type,
                        preserve_undefined,
                    )?;
                    "thaw_json_object_set_json"
                }
                HirType::Null => "thaw_json_object_set_json",
                HirType::JsValue => {
                    value = self.compile_dynamic_value_placeholder(value)?;
                    "thaw_json_object_set_json"
                }
                other => return Err(format!("unsupported dynamic object field {other:?}")),
            };
        self.builder
            .build_call(
                self.module.get_function(setter).unwrap(),
                &[json.into(), key.into(), value.into()],
                "set_dynamic_object_field",
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Encodes a `JsValue` (an opaque, permanent id into QuickJS's own
    /// retained-value table -- there is no JSON representation of "a live
    /// JS object") as a `{"__thaw_js_handle_id__": <id>}` placeholder
    /// object, which the QuickJS-side JSON reviver (see its doc comment in
    /// `dates.js`) splices back into the real value the moment the args
    /// JSON this placeholder sits in gets parsed on the other end -- bare,
    /// or nested inside an object/array literal at any depth, the same way
    /// that reviver already reconstructs a `Date`. Unlike a per-call
    /// index into a side channel, the handle's own id is permanent and
    /// meaningful on its own, so no extra plumbing back to the call site
    /// is needed here. Only meaningful while compiling a QuickJS-backed
    /// dynamic call's own arguments (`compile_typed_dynamic_call` sets
    /// `compiling_quickjs_dynamic_arguments` before marshaling them);
    /// anywhere else -- console.log formatting, a `Dictionary` literal, an
    /// N-API call (which has no such reviver) -- there is no receiving end
    /// for this placeholder, so this fails clearly instead of silently
    /// discarding the live value. Real-world example: zod's
    /// `z.object({ name: z.string() })`, whose `z.string()` argument is
    /// itself a `JsValue` (a live `ZodString` schema instance) nested
    /// inside the object-literal argument passed to `z.object`.
    fn compile_dynamic_value_placeholder(
        &mut self,
        handle: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if !self.compiling_quickjs_dynamic_arguments {
            return Err(
                "a dynamic (JsValue) value can only be passed as an argument to another QuickJS-backed dynamic call"
                    .into(),
            );
        }
        self.compile_dynamic_value_placeholder_unchecked(handle)
    }

    /// The gate-free core of `compile_dynamic_value_placeholder`, for a
    /// call site that already knows (by construction, not by the
    /// per-compilation `compiling_quickjs_dynamic_arguments` flag) that a
    /// reviver exists downstream -- see `build_call_with`'s own use of
    /// this, which reaches it only for an *ordinary* function call
    /// (never N-API, which rejects a `JsValue` argument outright well
    /// before codegen).
    fn compile_dynamic_value_placeholder_unchecked(
        &mut self,
        handle: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let handle = self
            .builder
            .build_unsigned_int_to_float(
                handle.into_int_value(),
                self.context.f64_type(),
                "dynamic_handle_id",
            )
            .map_err(|error| error.to_string())?;
        let placeholder = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_object_new").unwrap(),
                &[],
                "dynamic_handle_placeholder",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let key = self
            .builder
            .build_global_string_ptr("__thaw_js_handle_id__", "dynamic_handle_placeholder_key")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_object_set_number")
                    .unwrap(),
                &[placeholder.into(), key.as_pointer_value().into(), handle.into()],
                "dynamic_handle_placeholder_id",
            )
            .map_err(|error| error.to_string())?;
        Ok(placeholder)
    }

    /// Sets a `Union`-typed object field's JSON value -- the same
    /// per-member tag dispatch as `compile_json_array_push_union` (see
    /// its own doc comment), just recursing into
    /// `compile_json_object_set_native_with_undefined` for the same
    /// `key` instead of pushing onto an array. Real-world example:
    /// camelcase's own `Options` fields being read out and marshaled
    /// for a dynamic call, several of which are themselves optional
    /// unions once resolved.
    fn compile_json_object_set_union(
        &mut self,
        json: BasicValueEnum<'ctx>,
        key: PointerValue<'ctx>,
        value: StructValue<'ctx>,
        elements: &[HirType],
        preserve_undefined: bool,
    ) -> Result<(), String> {
        if elements.is_empty() {
            return Err("cannot serialize an empty union to JSON".into());
        }
        let tag = self
            .builder
            .build_extract_value(value, 0, "json_object_union_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(value, 1, "json_object_union_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let merge = self.context.append_basic_block(function, "json_object_union_set");
        for (index, member) in elements.iter().enumerate() {
            let matched = self
                .context
                .append_basic_block(function, "json_object_union_member");
            if index + 1 == elements.len() {
                self.builder
                    .build_unconditional_branch(matched)
                    .map_err(|error| error.to_string())?;
            } else {
                let next = self
                    .context
                    .append_basic_block(function, "json_object_union_next");
                let is_match = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_int(index as u64, false),
                        "json_object_union_tag_match",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(is_match, matched, next)
                    .map_err(|error| error.to_string())?;
            }
            self.builder.position_at_end(matched);
            let member_value = self.unpack_union_payload(payload, member)?;
            self.compile_json_object_set_native_with_undefined(
                json,
                key,
                member_value,
                member,
                preserve_undefined,
            )?;
            self.builder
                .build_unconditional_branch(merge)
                .map_err(|error| error.to_string())?;
            if index + 1 != elements.len() {
                let next = matched
                    .get_next_basic_block()
                    .ok_or("union JSON dispatch lost its next comparison block")?;
                self.builder.position_at_end(next);
            }
        }
        self.builder.position_at_end(merge);
        Ok(())
    }

    fn compile_json_object_set_tagged(
        &mut self,
        json: BasicValueEnum<'ctx>,
        key: PointerValue<'ctx>,
        tagged: StructValue<'ctx>,
        payload_type: &HirType,
        kind: JsonTaggedKind,
        preserve_undefined: bool,
    ) -> Result<(), String> {
        let three_state = matches!(kind, JsonTaggedKind::Nullish);
        let absent_is_null = matches!(kind, JsonTaggedKind::Nullable);
        let tag = self
            .builder
            .build_extract_value(tagged, 0, "json_object_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(tagged, 1, "json_object_payload")
            .map_err(|error| error.to_string())?;
        let present = if three_state {
            self.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    self.context.i8_type().const_zero(),
                    "json_object_has_value",
                )
                .map_err(|error| error.to_string())?
        } else {
            tag
        };
        let function = self.current_function();
        let value_block = self.context.append_basic_block(function, "json_object_value");
        let absent_block = self.context.append_basic_block(function, "json_object_absent");
        let done = self.context.append_basic_block(function, "json_object_tagged_done");
        self.builder
            .build_conditional_branch(present, value_block, absent_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(absent_block);
        if three_state {
            let null_block = self.context.append_basic_block(function, "json_object_null");
            let undefined_block = self
                .context
                .append_basic_block(function, "json_object_undefined");
            let is_null = self
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    self.context.i8_type().const_int(1, false),
                    "json_object_is_null",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_conditional_branch(is_null, null_block, undefined_block)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(null_block);
            let null = self.compile_json_null()?;
            self.compile_json_object_set_native_with_undefined(
                json,
                key,
                null,
                &HirType::Null,
                preserve_undefined,
            )?;
            self.builder
                .build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(undefined_block);
            if preserve_undefined {
                let undefined = self.compile_napi_undefined_json()?;
                self.compile_json_object_set_native_with_undefined(
                    json,
                    key,
                    undefined,
                    &HirType::Json,
                    true,
                )?;
            }
        } else if absent_is_null {
            let null = self.compile_json_null()?;
            self.compile_json_object_set_native_with_undefined(
                json,
                key,
                null,
                &HirType::Null,
                preserve_undefined,
            )?;
        } else if preserve_undefined {
            let undefined = self.compile_napi_undefined_json()?;
            self.compile_json_object_set_native_with_undefined(
                json,
                key,
                undefined,
                &HirType::Json,
                true,
            )?;
        }
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_block);
        self.compile_json_object_set_native_with_undefined(
            json,
            key,
            payload,
            payload_type,
            preserve_undefined,
        )?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn compile_native_array_to_json(
        &mut self,
        array: PointerValue<'ctx>,
        element_type: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_native_array_to_json_with_undefined(array, element_type, false)
    }

    fn compile_native_array_to_json_with_undefined(
        &mut self,
        array: PointerValue<'ctx>,
        element_type: &HirType,
        preserve_undefined: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // `array` is a handle (see `compile_array_wrap`'s doc comment);
        // unwrap it once here so every existing byte-level access below
        // keeps working against the raw buffer unchanged.
        let array = self.compile_array_data(array)?;
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "console_array_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_array_new returned no value")?;
        let i64_type = self.context.i64_type();
        let length = self
            .builder
            .build_load(i64_type, array, "console_array_length")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let entry = self
            .builder
            .get_insert_block()
            .ok_or("array conversion has no current block")?;
        let condition = self.context.append_basic_block(function, "console_array_next");
        let body = self.context.append_basic_block(function, "console_array_element");
        let done = self.context.append_basic_block(function, "console_array_done");
        self.builder
            .build_unconditional_branch(condition)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(condition);
        let index = self
            .builder
            .build_phi(i64_type, "console_array_index")
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&i64_type.const_zero(), entry)]);
        let has_element = self
            .builder
            .build_int_compare(
                IntPredicate::ULT,
                index.as_basic_value().into_int_value(),
                length,
                "console_array_has_element",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(has_element, body, done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(body);
        let offset = self
            .builder
            .build_int_mul(
                index.as_basic_value().into_int_value(),
                i64_type.const_int(array_element_storage_bytes(element_type), false),
                "console_array_element_offset",
            )
            .map_err(|error| error.to_string())?;
        let offset = self
            .builder
            .build_int_add(
                offset,
                i64_type.const_int(ARRAY_HEADER_BYTES, false),
                "console_array_payload_offset",
            )
            .map_err(|error| error.to_string())?;
        let pointer = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    array,
                    &[offset],
                    "console_array_element_pointer",
                )
                .map_err(|error| error.to_string())?
        };
        let value = self
            .builder
            .build_load(
                self.basic_type(element_type)?,
                pointer,
                "console_array_element_value",
            )
            .map_err(|error| error.to_string())?;
        self.compile_json_array_push_native_with_undefined(
            json,
            value,
            element_type,
            preserve_undefined,
        )?;
        let next = self
            .builder
            .build_int_add(
                index.as_basic_value().into_int_value(),
                i64_type.const_int(1, false),
                "console_array_increment",
            )
            .map_err(|error| error.to_string())?;
        let body_end = self
            .builder
            .get_insert_block()
            .ok_or("array conversion lost its body block")?;
        self.builder
            .build_unconditional_branch(condition)
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&next, body_end)]);

        self.builder.position_at_end(done);
        Ok(json)
    }

    fn compile_native_tuple_to_json(
        &mut self,
        tuple: PointerValue<'ctx>,
        element_types: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_native_tuple_to_json_with_undefined(tuple, element_types, false)
    }

    fn compile_native_tuple_to_json_with_undefined(
        &mut self,
        tuple: PointerValue<'ctx>,
        element_types: &[HirType],
        preserve_undefined: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // See `compile_native_array_to_json_with_undefined` -- `tuple` is a
        // handle, unwrap once up front.
        let tuple = self.compile_array_data(tuple)?;
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "console_tuple_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_array_new returned no value")?;
        let stride = element_types
            .iter()
            .map(array_element_storage_bytes)
            .max()
            .unwrap_or(ARRAY_ELEM_BYTES);
        for (index, element_type) in element_types.iter().enumerate() {
            let offset = self
                .context
                .i64_type()
                .const_int(ARRAY_HEADER_BYTES + stride * index as u64, false);
            let pointer = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        tuple,
                        &[offset],
                        "console_tuple_element_pointer",
                    )
                    .map_err(|error| error.to_string())?
            };
            let value = self
                .builder
                .build_load(
                    self.basic_type(element_type)?,
                    pointer,
                    "console_tuple_element_value",
                )
                .map_err(|error| error.to_string())?;
            self.compile_json_array_push_native_with_undefined(
                json,
                value,
                element_type,
                preserve_undefined,
            )?;
        }
        Ok(json)
    }

    fn compile_json_array_push_native(
        &mut self,
        json: BasicValueEnum<'ctx>,
        value: BasicValueEnum<'ctx>,
        element_type: &HirType,
    ) -> Result<(), String> {
        self.compile_json_array_push_native_with_undefined(json, value, element_type, false)
    }

    fn compile_json_array_push_native_with_undefined(
        &mut self,
        json: BasicValueEnum<'ctx>,
        mut value: BasicValueEnum<'ctx>,
        element_type: &HirType,
        preserve_undefined: bool,
    ) -> Result<(), String> {
        match element_type {
            HirType::Optional(payload) => {
                return self.compile_json_array_push_tagged(
                    json,
                    value.into_struct_value(),
                    payload,
                    false,
                    false,
                    preserve_undefined,
                );
            }
            HirType::Nullable(payload) => {
                return self.compile_json_array_push_tagged(
                    json,
                    value.into_struct_value(),
                    payload,
                    true,
                    false,
                    preserve_undefined,
                );
            }
            HirType::Nullish(payload) => {
                return self.compile_json_array_push_tagged(
                    json,
                    value.into_struct_value(),
                    payload,
                    false,
                    true,
                    preserve_undefined,
                );
            }
            HirType::Union(elements) => {
                return self.compile_json_array_push_union(
                    json,
                    value.into_struct_value(),
                    elements,
                    preserve_undefined,
                );
            }
            HirType::Null | HirType::Undefined => {
                value = self.compile_json_null()?;
            }
            _ => {}
        }
        let push = match element_type {
            HirType::F64 => "thaw_json_array_push_number",
            HirType::Str => "thaw_json_array_push_string",
            HirType::Bool => {
                value = self
                    .builder
                    .build_int_z_extend(
                        value.into_int_value(),
                        self.context.i8_type(),
                        "console_array_bool",
                    )
                    .map_err(|error| error.to_string())?
                    .into();
                "thaw_json_array_push_bool"
            }
            HirType::Json | HirType::Dictionary(_) => {
                // See the identical check in
                // `compile_json_object_set_native_with_undefined`: a
                // `Json`-declared slot can carry a real `JsValue` handle at
                // runtime (an unambiguous `i64` vs. pointer distinction).
                if value.is_int_value() {
                    value = self.compile_dynamic_value_placeholder(value)?;
                }
                "thaw_json_array_push_json"
            }
            HirType::Array(nested) => {
                value = self.compile_native_array_to_json_with_undefined(
                    value.into_pointer_value(),
                    nested,
                    preserve_undefined,
                )?;
                "thaw_json_array_push_json"
            }
            HirType::Tuple(elements) => {
                value = self.compile_native_tuple_to_json_with_undefined(
                    value.into_pointer_value(),
                    elements,
                    preserve_undefined,
                )?;
                "thaw_json_array_push_json"
            }
            HirType::Object(_) => {
                value = self.compile_native_object_to_json(value.into_pointer_value(), element_type)?;
                "thaw_json_array_push_json"
            }
            HirType::Null | HirType::Undefined => "thaw_json_array_push_json",
            HirType::JsValue => {
                value = self.compile_dynamic_value_placeholder(value)?;
                "thaw_json_array_push_json"
            }
            other => {
                return Err(format!(
                    "cannot serialize collection element {other:?} to JSON"
                ))
            }
        };
        self.builder
            .build_call(
                self.module.get_function(push).unwrap(),
                &[json.into(), value.into()],
                "console_array_push",
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Pushes a `Union` value onto a JSON array (or, by the same
    /// mechanism, packs one into a dynamic call's own JSON argument
    /// array -- both share this function through
    /// `compile_json_array_push_native_with_undefined`): reads the
    /// value's own runtime tag and, exactly like `compile_console_union`
    /// does to print one, dispatches to whichever member is actually
    /// active, unpacks its payload (`unpack_union_payload`, the same
    /// helper `HirExpr::UnionValue`'s own codegen uses), and recurses
    /// into this same push logic with that member's real type -- so a
    /// union nested inside another union, or one whose active member is
    /// itself an array/object/etc., serializes correctly too. Real-world
    /// example: camelcase's own `camelCase(input: string | readonly
    /// string[], options?): string`.
    fn compile_json_array_push_union(
        &mut self,
        json: BasicValueEnum<'ctx>,
        value: StructValue<'ctx>,
        elements: &[HirType],
        preserve_undefined: bool,
    ) -> Result<(), String> {
        if elements.is_empty() {
            return Err("cannot serialize an empty union to JSON".into());
        }
        let tag = self
            .builder
            .build_extract_value(value, 0, "json_union_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(value, 1, "json_union_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let merge = self.context.append_basic_block(function, "json_union_pushed");
        for (index, member) in elements.iter().enumerate() {
            let matched = self.context.append_basic_block(function, "json_union_member");
            if index + 1 == elements.len() {
                self.builder
                    .build_unconditional_branch(matched)
                    .map_err(|error| error.to_string())?;
            } else {
                let next = self.context.append_basic_block(function, "json_union_next");
                let is_match = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_int(index as u64, false),
                        "json_union_tag_match",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(is_match, matched, next)
                    .map_err(|error| error.to_string())?;
            }
            self.builder.position_at_end(matched);
            let member_value = self.unpack_union_payload(payload, member)?;
            self.compile_json_array_push_native_with_undefined(
                json,
                member_value,
                member,
                preserve_undefined,
            )?;
            self.builder
                .build_unconditional_branch(merge)
                .map_err(|error| error.to_string())?;
            if index + 1 != elements.len() {
                let next = matched
                    .get_next_basic_block()
                    .ok_or("union JSON dispatch lost its next comparison block")?;
                self.builder.position_at_end(next);
            }
        }
        self.builder.position_at_end(merge);
        Ok(())
    }

    fn compile_json_null(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        self.builder
            .build_call(
                self.module.get_function("thaw_json_null").unwrap(),
                &[],
                "json_null",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_null returned no value".to_string())
    }

    fn compile_json_array_push_tagged(
        &mut self,
        json: BasicValueEnum<'ctx>,
        tagged: StructValue<'ctx>,
        payload_type: &HirType,
        absent_is_null: bool,
        three_state: bool,
        preserve_undefined: bool,
    ) -> Result<(), String> {
        let tag = self
            .builder
            .build_extract_value(tagged, 0, "json_collection_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(tagged, 1, "json_collection_payload")
            .map_err(|error| error.to_string())?;
        let present = if three_state {
            self.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    self.context.i8_type().const_zero(),
                    "json_collection_has_value",
                )
                .map_err(|error| error.to_string())?
        } else {
            tag
        };
        let function = self.current_function();
        let value_block = self.context.append_basic_block(function, "json_collection_value");
        let absent_block = self
            .context
            .append_basic_block(function, "json_collection_absent");
        let done = self.context.append_basic_block(function, "json_collection_tagged_done");
        self.builder
            .build_conditional_branch(present, value_block, absent_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(absent_block);
        let absent = if preserve_undefined && !absent_is_null {
            if three_state {
                let null_block = self.context.append_basic_block(function, "json_collection_null");
                let undefined_block = self
                    .context
                    .append_basic_block(function, "json_collection_undefined");
                let absent_done = self
                    .context
                    .append_basic_block(function, "json_collection_absent_done");
                let is_null = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_int(1, false),
                        "json_collection_is_null",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(is_null, null_block, undefined_block)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(null_block);
                let null = self.compile_json_null()?;
                self.builder
                    .build_unconditional_branch(absent_done)
                    .map_err(|error| error.to_string())?;
                let null_end = self.builder.get_insert_block().ok_or("lost JSON null block")?;
                self.builder.position_at_end(undefined_block);
                let undefined = self.compile_napi_undefined_json()?;
                self.builder
                    .build_unconditional_branch(absent_done)
                    .map_err(|error| error.to_string())?;
                let undefined_end = self
                    .builder
                    .get_insert_block()
                    .ok_or("lost JSON undefined block")?;
                self.builder.position_at_end(absent_done);
                let result = self
                    .builder
                    .build_phi(
                        self.context.ptr_type(AddressSpace::default()),
                        "json_collection_absent_value",
                    )
                    .map_err(|error| error.to_string())?;
                result.add_incoming(&[(&null, null_end), (&undefined, undefined_end)]);
                result.as_basic_value()
            } else {
                self.compile_napi_undefined_json()?
            }
        } else {
            self.compile_json_null()?
        };
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_array_push_json")
                    .unwrap(),
                &[json.into(), absent.into()],
                "push_collection_absent",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_block);
        self.compile_json_array_push_native_with_undefined(
            json,
            payload,
            payload_type,
            preserve_undefined,
        )?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn compile_json_to_native_object(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirType::Object(fields) = ty else {
            return Err("dynamic result requires an object type".to_string());
        };
        let object = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    self.context
                        .i64_type()
                        .const_int(object_storage_bytes(fields), false)
                        .into(),
                    self.context.i64_type().const_int(8, false).into(),
                ],
                "dynamic_result_object",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        for (index, (name, field_ty)) in fields.iter().enumerate() {
            let key = self
                .builder
                .build_global_string_ptr(name, "dynamic_result_key")
                .map_err(|error| error.to_string())?;
            let field_json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_get").unwrap(),
                    &[json.into(), key.as_pointer_value().into()],
                    "dynamic_result_field_json",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let field = match field_ty {
                HirType::Optional(payload) => self.compile_json_to_optional_field(
                    json,
                    key.as_pointer_value(),
                    field_json,
                    payload,
                    false,
                )?,
                HirType::Nullable(payload) => self.compile_json_to_nullable_field(
                    field_json,
                    payload,
                )?,
                HirType::Nullish(payload) => self.compile_json_to_nullish_field(
                    json,
                    key.as_pointer_value(),
                    field_json,
                    payload,
                )?,
                _ => self.compile_json_value_to_native(field_json, field_ty)?,
            };
            let offset = self
                .context
                .i64_type()
                .const_int(object_field_offset(fields, index), false);
            let pointer = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), object, &[offset], "result_field")
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(pointer, field)
                .map_err(|error| error.to_string())?;
        }
        Ok(object.into())
    }

    fn compile_json_to_native(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            HirType::Array(element) => self.compile_json_to_native_array(json, element),
            HirType::Tuple(elements) => self.compile_json_to_native_tuple(json, elements),
            HirType::Object(_) => self.compile_json_to_native_object(json, ty),
            other => Err(format!(
                "JSON-backed dictionary value cannot be restored as {other:?}"
            )),
        }
    }

    fn compile_json_value_to_native(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            HirType::F64 => self.compile_json_as_value(json, "thaw_json_as_number"),
            HirType::Str => self.compile_json_as_value(json, "thaw_json_as_string"),
            HirType::Bool => self.compile_json_as_bool_value(json),
            HirType::Json | HirType::Dictionary(_) => Ok(json),
            HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) => {
                self.compile_json_to_native(json, ty)
            }
            // A `JsValue`-typed native-callback parameter (e.g. zod's
            // `.superRefine((val, ctx: JsValue) => { ctx.addIssue(...); })`
            // -- `ctx`, a live object with methods, has no JSON
            // representation to decode at all). Unlike every other case
            // here, this doesn't come from real JSON data: `compile_
            // register_native_callback`'s own `jsvalue_param_mask` tells
            // the JS-side wrapper (`thaw_js_register_native_callback`,
            // thaw-quickjs) to retain exactly the arguments at a
            // `JsValue`-typed position and encode them as the same
            // `{"__thaw_js_handle_id__": N}` marker `compile_dynamic_
            // value_placeholder` already builds for the opposite
            // direction -- so this position is *guaranteed* to hold that
            // marker shape, not decoded defensively the way a value from
            // real user JSON would need to be.
            HirType::JsValue => {
                let key = self
                    .builder
                    .build_global_string_ptr(
                        "__thaw_js_handle_id__",
                        "native_callback_jsvalue_key",
                    )
                    .map_err(|error| error.to_string())?;
                let id = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_get").unwrap(),
                        &[json.into(), key.as_pointer_value().into()],
                        "native_callback_jsvalue_marker",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_get did not return a value")?;
                let id = self.compile_json_as_value(id, "thaw_json_as_number")?;
                self.builder
                    .build_float_to_unsigned_int(
                        id.into_float_value(),
                        self.context.i64_type(),
                        "native_callback_jsvalue_arg",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string())
            }
            other => Err(format!("unsupported dynamic result value {other:?}")),
        }
    }

    fn compile_json_to_optional_field(
        &mut self,
        object: BasicValueEnum<'ctx>,
        key: PointerValue<'ctx>,
        json: BasicValueEnum<'ctx>,
        payload: &HirType,
        nullable: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let present = if nullable {
            let is_null = self.compile_json_is_null_value(json)?;
            self.builder
                .build_not(is_null, "json_nullable_present")
                .map_err(|error| error.to_string())?
        } else {
            let has_own = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_has_own").unwrap(),
                    &[object.into(), key.into()],
                    "json_optional_has_own",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_json_has_own returned no value")?
                .into_int_value();
            let has_own = self.builder
                .build_int_compare(
                    IntPredicate::NE,
                    has_own,
                    self.context.i8_type().const_zero(),
                    "json_optional_present",
                )
                .map_err(|error| error.to_string())?;
            let is_undefined = self.compile_json_is_napi_undefined(json)?;
            self.builder
                .build_and(
                    has_own,
                    self.builder
                        .build_not(is_undefined, "json_optional_not_sentinel")
                        .map_err(|error| error.to_string())?,
                    "json_optional_present_value",
                )
                .map_err(|error| error.to_string())?
        };
        let function = self.current_function();
        let value_block = self.context.append_basic_block(function, "json_tagged_value");
        let absent_block = self.context.append_basic_block(function, "json_tagged_absent");
        let done = self.context.append_basic_block(function, "json_tagged_done");
        self.builder
            .build_conditional_branch(present, value_block, absent_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_block);
        let payload_value = self.compile_json_value_to_native(json, payload)?;
        let value = self.build_optional_value(payload_value, payload, true)?;
        let value_end = self.builder.get_insert_block().ok_or("lost tagged value block")?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(absent_block);
        let zero = self.compile_zero_value(payload)?;
        let absent = self.build_optional_value(zero, payload, false)?;
        let absent_end = self
            .builder
            .get_insert_block()
            .ok_or("lost tagged absent block")?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(done);
        let tagged_type = self.basic_type(&HirType::Optional(Box::new(payload.clone())))?;
        let result = self
            .builder
            .build_phi(tagged_type, "json_tagged_field")
            .map_err(|error| error.to_string())?;
        result.add_incoming(&[(&value, value_end), (&absent, absent_end)]);
        Ok(result.as_basic_value())
    }

    fn compile_json_to_nullable_field(
        &mut self,
        json: BasicValueEnum<'ctx>,
        payload: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_json_to_optional_field(json, json.into_pointer_value(), json, payload, true)
    }

    fn compile_json_is_null_value(
        &mut self,
        json: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        let result = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_is_null").unwrap(),
                &[json.into()],
                "json_is_null",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_is_null returned no value")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                result,
                self.context.i8_type().const_zero(),
                "json_is_null_bool",
            )
            .map_err(|error| error.to_string())
    }

    fn compile_json_to_nullish_field(
        &mut self,
        object: BasicValueEnum<'ctx>,
        key: PointerValue<'ctx>,
        json: BasicValueEnum<'ctx>,
        payload: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let has_own = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_has_own").unwrap(),
                &[object.into(), key.into()],
                "json_nullish_has_own",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_has_own returned no value")?
            .into_int_value();
        let has_own = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                has_own,
                self.context.i8_type().const_zero(),
                "json_nullish_present",
            )
            .map_err(|error| error.to_string())?;
        let is_undefined = self.compile_json_is_napi_undefined(json)?;
        let has_own = self
            .builder
            .build_and(
                has_own,
                self.builder
                    .build_not(is_undefined, "json_nullish_not_sentinel")
                    .map_err(|error| error.to_string())?,
                "json_nullish_has_value",
            )
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let present_block = self.context.append_basic_block(function, "json_nullish_present");
        let undefined_block = self
            .context
            .append_basic_block(function, "json_nullish_undefined");
        let null_block = self.context.append_basic_block(function, "json_nullish_null");
        let value_block = self.context.append_basic_block(function, "json_nullish_value");
        let done = self.context.append_basic_block(function, "json_nullish_done");
        self.builder
            .build_conditional_branch(has_own, present_block, undefined_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(undefined_block);
        let zero = self.compile_zero_value(payload)?;
        let undefined = self.build_nullish_value(zero, payload, 2)?;
        let undefined_end = self
            .builder
            .get_insert_block()
            .ok_or("lost nullish undefined block")?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(present_block);
        let is_null = self.compile_json_is_null_value(json)?;
        self.builder
            .build_conditional_branch(is_null, null_block, value_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(null_block);
        let zero = self.compile_zero_value(payload)?;
        let null = self.build_nullish_value(zero, payload, 1)?;
        let null_end = self
            .builder
            .get_insert_block()
            .ok_or("lost nullish null block")?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_block);
        let payload_value = self.compile_json_value_to_native(json, payload)?;
        let value = self.build_nullish_value(payload_value, payload, 0)?;
        let value_end = self
            .builder
            .get_insert_block()
            .ok_or("lost nullish value block")?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(done);
        let tagged_type = self.basic_type(&HirType::Nullish(Box::new(payload.clone())))?;
        let result = self
            .builder
            .build_phi(tagged_type, "json_nullish_field")
            .map_err(|error| error.to_string())?;
        result.add_incoming(&[
            (&undefined, undefined_end),
            (&null, null_end),
            (&value, value_end),
        ]);
        Ok(result.as_basic_value())
    }

    fn compile_json_to_native_array(
        &mut self,
        json: BasicValueEnum<'ctx>,
        element: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let length = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_length").unwrap(),
                &[json.into()],
                "json_array_length",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_array_length returned no value")?
            .into_int_value();
        let element_bytes = i64_type.const_int(array_element_storage_bytes(element), false);
        let payload_size = self
            .builder
            .build_int_mul(length, element_bytes, "json_array_payload_size")
            .map_err(|error| error.to_string())?;
        let allocation_size = self
            .builder
            .build_int_add(
                payload_size,
                i64_type.const_int(ARRAY_HEADER_BYTES, false),
                "json_array_allocation_size",
            )
            .map_err(|error| error.to_string())?;
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    allocation_size.into(),
                    i64_type
                        .const_int(array_element_storage_bytes(element).min(8), false)
                        .into(),
                ],
                "json_native_array",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc returned no array")?
            .into_pointer_value();
        self.builder
            .build_store(array, length)
            .map_err(|error| error.to_string())?;

        let function = self.current_function();
        let entry = self
            .builder
            .get_insert_block()
            .ok_or("JSON array conversion has no current block")?;
        let condition = self.context.append_basic_block(function, "json_array_next");
        let body = self.context.append_basic_block(function, "json_array_element");
        let done = self.context.append_basic_block(function, "json_array_done");
        self.builder
            .build_unconditional_branch(condition)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(condition);
        let index = self
            .builder
            .build_phi(i64_type, "json_array_index")
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&i64_type.const_zero(), entry)]);
        let has_element = self
            .builder
            .build_int_compare(
                IntPredicate::ULT,
                index.as_basic_value().into_int_value(),
                length,
                "json_array_has_element",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(has_element, body, done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(body);
        let json_index = self
            .builder
            .build_signed_int_to_float(
                index.as_basic_value().into_int_value(),
                self.context.f64_type(),
                "json_array_float_index",
            )
            .map_err(|error| error.to_string())?;
        let null_key = self.context.ptr_type(AddressSpace::default()).const_null();
        let element_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_index").unwrap(),
                &[json.into(), json_index.into(), null_key.into()],
                "json_array_element_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_index returned no value")?;
        let value = match element {
            HirType::F64 => self.compile_json_as_value(element_json, "thaw_json_as_number")?,
            HirType::Str => self.compile_json_as_value(element_json, "thaw_json_as_string")?,
            HirType::Bool => self.compile_json_as_bool_value(element_json)?,
            HirType::Json | HirType::Dictionary(_) => element_json,
            HirType::Optional(payload) => {
                let (object, key) = self.compile_napi_optional_result_container(element_json)?;
                self.compile_json_to_optional_field(object, key, element_json, payload, false)?
            }
            HirType::Nullable(payload) => {
                self.compile_json_to_nullable_field(element_json, payload)?
            }
            HirType::Nullish(payload) => {
                let (object, key) = self.compile_napi_optional_result_container(element_json)?;
                self.compile_json_to_nullish_field(object, key, element_json, payload)?
            }
            HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) => {
                self.compile_json_to_native(element_json, element)?
            }
            other => return Err(format!("unsupported JSON array element {other:?}")),
        };
        let offset = self
            .builder
            .build_int_mul(
                index.as_basic_value().into_int_value(),
                element_bytes,
                "json_array_element_offset",
            )
            .map_err(|error| error.to_string())?;
        let offset = self
            .builder
            .build_int_add(
                offset,
                i64_type.const_int(ARRAY_HEADER_BYTES, false),
                "json_array_payload_offset",
            )
            .map_err(|error| error.to_string())?;
        let pointer = unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), array, &[offset], "json_array_slot")
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(pointer, value)
            .map_err(|error| error.to_string())?;
        let next = self
            .builder
            .build_int_add(
                index.as_basic_value().into_int_value(),
                i64_type.const_int(1, false),
                "json_array_increment",
            )
            .map_err(|error| error.to_string())?;
        let body_end = self
            .builder
            .get_insert_block()
            .ok_or("JSON array conversion lost its body block")?;
        self.builder
            .build_unconditional_branch(condition)
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&next, body_end)]);

        self.builder.position_at_end(done);
        // `array` is a freshly built raw buffer; wrap it in a handle before
        // treating it as this call's array-typed return value (see
        // `compile_array_wrap`'s doc comment).
        Ok(self.compile_array_wrap(array)?.into())
    }

    fn compile_json_to_native_tuple(
        &mut self,
        json: BasicValueEnum<'ctx>,
        elements: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let element_bytes = elements
            .iter()
            .map(array_element_storage_bytes)
            .max()
            .unwrap_or(ARRAY_ELEM_BYTES);
        let size = ARRAY_HEADER_BYTES + element_bytes * elements.len() as u64;
        let tuple = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    i64_type.const_int(size, false).into(),
                    i64_type.const_int(element_bytes.min(8), false).into(),
                ],
                "json_native_tuple",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc returned no tuple")?
            .into_pointer_value();
        self.builder
            .build_store(tuple, i64_type.const_int(elements.len() as u64, false))
            .map_err(|error| error.to_string())?;

        for (index, element) in elements.iter().enumerate() {
            let element_json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_index").unwrap(),
                    &[
                        json.into(),
                        self.context.f64_type().const_float(index as f64).into(),
                        self.context.ptr_type(AddressSpace::default()).const_null().into(),
                    ],
                    "json_tuple_element",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_json_index returned no tuple element")?;
            let value = match element {
                HirType::F64 => {
                    self.compile_json_as_value(element_json, "thaw_json_as_number")?
                }
                HirType::Str => {
                    self.compile_json_as_value(element_json, "thaw_json_as_string")?
                }
                HirType::Bool => self.compile_json_as_bool_value(element_json)?,
                HirType::Json | HirType::Dictionary(_) => element_json,
                HirType::Optional(payload) => {
                    let (object, key) = self.compile_napi_optional_result_container(element_json)?;
                    self.compile_json_to_optional_field(
                        object,
                        key,
                        element_json,
                        payload,
                        false,
                    )?
                }
                HirType::Nullable(payload) => {
                    self.compile_json_to_nullable_field(element_json, payload)?
                }
                HirType::Nullish(payload) => {
                    let (object, key) = self.compile_napi_optional_result_container(element_json)?;
                    self.compile_json_to_nullish_field(object, key, element_json, payload)?
                }
                HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) => {
                    self.compile_json_to_native(element_json, element)?
                }
                other => return Err(format!("unsupported JSON tuple element {other:?}")),
            };
            let offset = i64_type.const_int(
                ARRAY_HEADER_BYTES + element_bytes * index as u64,
                false,
            );
            let pointer = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        tuple,
                        &[offset],
                        "json_tuple_slot",
                    )
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(pointer, value)
                .map_err(|error| error.to_string())?;
        }
        // `tuple` is a freshly built raw buffer; wrap it in a handle before
        // treating it as this call's tuple-typed return value (see
        // `compile_array_wrap`'s doc comment).
        Ok(self.compile_array_wrap(tuple)?.into())
    }
}
