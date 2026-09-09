impl<'ctx> HirCompiler<'ctx> {
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
            HirType::Json | HirType::Dictionary(_) | HirType::Undefined | HirType::Null => Ok(json),
            HirType::Optional(payload) => {
                let absent = self.compile_json_is_napi_undefined(json)?;
                let present = self
                    .builder
                    .build_not(absent, "json_optional_present")
                    .map_err(|error| error.to_string())?;
                self.compile_json_to_optional_value(json, payload, present)
            }
            HirType::Nullable(payload) => self.compile_json_to_nullable_field(json, payload),
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
            HirType::Function(params, ret) if **ret == HirType::Void => {
                self.compile_js_void_callback_from_json(json, params)
            }
            HirType::CallableFunction(params, _, None, ret) if **ret == HirType::Void => {
                self.compile_js_void_callback_from_json(json, params)
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
        self.compile_json_to_optional_value(json, payload, present)
    }

    fn compile_json_to_optional_value(
        &mut self,
        json: BasicValueEnum<'ctx>,
        payload: &HirType,
        present: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
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
