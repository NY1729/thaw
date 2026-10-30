impl<'ctx> HirCompiler<'ctx> {
    fn compile_validate_native_argument(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<(), String> {
        if !self.validating_native_argument {
            return Ok(());
        }
        let valid = match ty {
            HirType::F64 | HirType::I64 | HirType::Str | HirType::Bool | HirType::Object(_) => {
                let kind = self
                    .compile_json_as_value(json, "thaw_json_typeof")?
                    .into_pointer_value();
                let expected = match ty {
                    HirType::F64 => "number",
                    HirType::I64 => "bigint",
                    HirType::Str => "string",
                    HirType::Bool => "boolean",
                    _ => "object",
                };
                let mut valid = self.compile_typeof_matches(kind, expected)?;
                if let HirType::Object(fields) = ty {
                    let null = self.compile_json_is_null_value(json)?;
                    let present = self
                        .builder
                        .build_not(null, "native_argument_not_null")
                        .map_err(|error| error.to_string())?;
                    valid = self
                        .builder
                        .build_and(valid, present, "native_argument_object")
                        .map_err(|error| error.to_string())?;
                    if let Some(class) = fields
                        .first()
                        .and_then(|(marker, _)| marker.strip_prefix("__thaw_class_identity_\u{1e}"))
                    {
                        if !self.thaw_js_routed {
                            self.uses_quickjs = true;
                            self.uses_quickjs_handles = true;
                        }
                        let (pointer, present) =
                            self.query_native_receiver(json.into_pointer_value(), "")?;
                        let pointer = self
                            .builder
                            .build_int_to_ptr(
                                pointer,
                                self.context.ptr_type(AddressSpace::default()),
                                "native_argument_class_owner",
                            )
                            .map_err(|error| error.to_string())?;
                        let class = class.split('\u{1f}').next().unwrap();
                        let class = self
                            .builder
                            .build_global_string_ptr(class, "native_argument_class")
                            .map_err(|error| error.to_string())?;
                        let branded = self
                            .builder
                            .build_call(
                                self.module
                                    .get_function("thaw_object_has_class_identity")
                                    .unwrap(),
                                &[pointer.into(), class.as_pointer_value().into()],
                                "native_argument_brand",
                            )
                            .map_err(|error| error.to_string())?
                            .try_as_basic_value()
                            .basic()
                            .ok_or("class brand result")?
                            .into_int_value();
                        let branded = self
                            .builder
                            .build_and(branded, present, "native_argument_authenticated")
                            .map_err(|error| error.to_string())?;
                        valid = self
                            .builder
                            .build_and(valid, branded, "native_argument_class_valid")
                            .map_err(|error| error.to_string())?;
                    }
                }
                valid
            }
            HirType::Array(_) | HirType::Tuple(_) => {
                let array = self
                    .compile_json_as_value(json, "thaw_json_is_array")?
                    .into_int_value();
                self.builder
                    .build_int_compare(
                        IntPredicate::NE,
                        array,
                        array.get_type().const_zero(),
                        "native_argument_is_array",
                    )
                    .map_err(|error| error.to_string())?
            }
            _ => return Ok(()),
        };
        self.compile_check_json_host_error(json, None)?;
        let function = self.current_function();
        let accepted = self
            .context
            .append_basic_block(function, "native_argument_valid");
        let rejected = self
            .context
            .append_basic_block(function, "native_argument_invalid");
        self.builder
            .build_conditional_branch(valid, accepted, rejected)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(rejected);
        self.compile_throw_type_error("Value is incompatible with the native argument type")?;
        self.builder.position_at_end(accepted);
        Ok(())
    }

    fn compile_json_to_native_object_argument(
        &mut self,
        json: PointerValue<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirType::Object(fields) = ty else {
            unreachable!()
        };
        let (owner, present) = self.query_native_object_receiver(json, fields)?;
        let function = self.current_function();
        let borrowed = self
            .context
            .append_basic_block(function, "object_argument_borrowed");
        let copied = self
            .context
            .append_basic_block(function, "object_argument_copied");
        let done = self
            .context
            .append_basic_block(function, "object_argument_ready");
        self.builder
            .build_conditional_branch(present, borrowed, copied)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(borrowed);
        let owner = self
            .builder
            .build_int_to_ptr(
                owner,
                self.context.ptr_type(AddressSpace::default()),
                "object_argument_owner",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(copied);
        let copy = self.compile_json_to_native_object(json.into(), ty)?;
        let copy_exit = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        let result = self
            .builder
            .build_phi(
                self.context.ptr_type(AddressSpace::default()),
                "object_argument",
            )
            .map_err(|error| error.to_string())?;
        result.add_incoming(&[(&owner, borrowed), (&copy, copy_exit)]);
        Ok(result.as_basic_value())
    }

    fn compile_json_to_native_object(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirType::Object(fields) = ty else {
            return Err("dynamic result requires an object type".to_string());
        };
        // Date is represented across the JSON bridge by its authenticated
        // runtime identity and timestamp, not by enumerable JS properties.
        // Decode it through the Date helper before allocating the native
        // result so a host-side conversion error cannot leave arena garbage.
        if matches!(fields.as_slice(),
            [(marker, HirType::Bool), (timestamp, HirType::F64)]
                if marker == "__thaw_class_identity_\u{1e}Date" && timestamp == "timestamp")
        {
            let timestamp = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_json_date_timestamp")
                        .unwrap(),
                    &[json.into()],
                    "decoded_date_timestamp",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_json_date_timestamp returned no value")?
                .into_float_value();
            self.compile_check_json_host_error(json, None)?;

            let object = self
                .builder
                .build_call(
                    self.module.get_function("thaw_arena_alloc").unwrap(),
                    &[
                        self.context
                            .i64_type()
                            .const_int(object_storage_bytes(fields)?, false)
                            .into(),
                        self.context.i64_type().const_int(8, false).into(),
                    ],
                    "dynamic_result_date_object",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_pointer_value();
            let values: [BasicValueEnum<'ctx>; 2] = [
                self.context.bool_type().const_int(1, false).into(),
                timestamp.into(),
            ];
            for (index, value) in values.into_iter().enumerate() {
                let offset = self
                    .context
                    .i64_type()
                    .const_int(object_field_offset(fields, index)?, false);
                let pointer = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            object,
                            &[offset],
                            "decoded_date_field",
                        )
                        .map_err(|error| error.to_string())?
                };
                self.builder
                    .build_store(pointer, value)
                    .map_err(|error| error.to_string())?;
            }
            let marker = self
                .builder
                .build_global_string_ptr(&fields[0].0, "decoded_date_identity")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_object_set_class_identity")
                        .unwrap(),
                    &[object.into(), marker.as_pointer_value().into()],
                    "register_decoded_date_identity",
                )
                .map_err(|error| error.to_string())?;
            return Ok(object.into());
        }
        self.compile_validate_native_argument(json, ty)?;
        let object = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    self.context
                        .i64_type()
                        .const_int(object_storage_bytes(fields)?, false)
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
            if self.validating_native_argument && name.starts_with("__thaw_class_identity_\u{1e}") {
                let pointer = self.compile_field_ptr_from_pointer(object, fields, index)?;
                self.builder
                    .build_store(pointer, self.context.bool_type().const_int(1, false))
                    .map_err(|error| error.to_string())?;
                continue;
            }
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
            self.compile_check_json_host_error(field_json, Some("thaw_json_destroy"))?;
            let field = match field_ty {
                HirType::Optional(payload) => self.compile_json_to_optional_field(
                    json,
                    key.as_pointer_value(),
                    field_json,
                    payload,
                    false,
                )?,
                HirType::Nullable(payload) => {
                    self.compile_json_to_nullable_field(field_json, payload)?
                }
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
                .const_int(object_field_offset(fields, index)?, false);
            let pointer = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), object, &[offset], "result_field")
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(pointer, field)
                .map_err(|error| error.to_string())?;
            self.compile_destroy_decoded_owned_json(field_json, field_ty)?;
        }
        Ok(object.into())
    }

    /// Decodes a `Json` value into the object a `Recursive` back-edge names, through one internal
    /// helper per layout (see `compile_recursive_to_json`).
    fn compile_recursive_from_json(
        &mut self,
        json: BasicValueEnum<'ctx>,
        key: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let name = format!(
            "__thaw_recursive_from_json_{}",
            key.replace(|c: char| !c.is_ascii_alphanumeric(), "_"),
        );
        let pointer = self.context.ptr_type(AddressSpace::default());
        let helper = match self.module.get_function(&name) {
            Some(helper) => helper,
            None => {
                let layout = HirType::Recursive(key.to_string()).expand();
                if !matches!(layout, HirType::Object(_)) {
                    return Err(format!("unresolved recursive layout `{key}`"));
                }
                let helper = self.module.add_function(
                    &name,
                    pointer.fn_type(&[pointer.into()], false),
                    Some(Linkage::Internal),
                );
                let parent_block = self.builder.get_insert_block().ok_or("recursive decoder outside a function")?;
                let saved_variables = std::mem::take(&mut self.variables);
                let saved_scope_context = self.take_codegen_scope_context();
                let saved_for_iteration_frame_slots = std::mem::take(&mut self.for_iteration_frame_slots);
                let saved_catch_native_text = std::mem::take(&mut self.catch_native_text);
                let saved_variable_hir_types = std::mem::take(&mut self.variable_hir_types);
                let saved_arena_variables = std::mem::take(&mut self.arena_variables);
                let result = (|| -> Result<(), String> {
                    let entry = self.context.append_basic_block(helper, "entry");
                    self.builder.position_at_end(entry);
                    self.emit_stack_overflow_guard()?;
                    let node = helper.get_first_param().ok_or("recursive decoder has no parameter")?;
                    let object = self.compile_json_to_native_object(node, &layout)?;
                    self.builder.build_return(Some(&object)).map_err(|error| error.to_string())?;
                    Ok(())
                })();
                self.variables = saved_variables;
                self.replace_codegen_scope_context(saved_scope_context);
                self.for_iteration_frame_slots = saved_for_iteration_frame_slots;
                self.catch_native_text = saved_catch_native_text;
                self.variable_hir_types = saved_variable_hir_types;
                self.arena_variables = saved_arena_variables;
                self.builder.position_at_end(parent_block);
                result?;
                helper
            }
        };
        self.builder
            .build_call(helper, &[json.into()], "recursive_from_json")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "recursive decoder returned no value".to_string())
    }

    fn compile_json_to_native(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            HirType::OverloadedFunction(_, physical) => {
                self.compile_json_value_to_native(json, physical)
            }
            HirType::F64 | HirType::I64 | HirType::Str | HirType::Bool => {
                self.compile_json_value_to_native(json, ty)
            }
            HirType::Array(element) => self.compile_json_to_native_array(json, element),
            HirType::Map(..) | HirType::Set(_) => self.compile_json_to_native_collection(json, ty),
            HirType::Tuple(elements) => self.compile_json_to_native_tuple(json, elements),
            HirType::Object(_) => self.compile_json_to_native_object(json, ty),
            HirType::Recursive(key) => self.compile_recursive_from_json(json, key),
            HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_) => {
                self.compile_json_value_to_native(json, ty)
            }
            HirType::Union(elements) => self.compile_json_to_union_result(json, elements),
            // A `Json`-typed value that is genuinely the
            // `{"__thaw_js_handle_id__": id}` placeholder `JsValueAsJson`
            // builds for a live handle coerced into a `Json`-declared
            // slot (real trigger: `const rs: any = new ReadStream(...)`)
            // recovers that handle here -- the same native reader
            // `compile_json_value_to_native`'s own `HirType::JsValue` arm
            // already uses for a `JsValue`-typed native-callback argument,
            // just reachable from `JsonAsNative`'s own dispatch too now.
            HirType::JsValue => self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_handle_id").unwrap(),
                    &[json.into()],
                    "json_as_js_value",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "thaw_json_handle_id did not return a value".into()),
            other => Err(format!(
                "JSON-backed dictionary value cannot be restored as {other:?}"
            )),
        }
    }

    fn compile_json_to_native_collection(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (pointer, valid) =
            self.query_native_receiver(json.into_pointer_value(), &format!("collection:{ty:?}"))?;
        let function = self.current_function();
        let accepted = self
            .context
            .append_basic_block(function, "native_collection_accepted");
        let rejected = self
            .context
            .append_basic_block(function, "native_collection_rejected");
        let done = self
            .context
            .append_basic_block(function, "native_collection_ready");
        self.builder
            .build_conditional_branch(valid, accepted, rejected)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(accepted);
        let native = self
            .builder
            .build_int_to_ptr(
                pointer,
                self.context.ptr_type(AddressSpace::default()),
                "native_collection_argument",
            )
            .map_err(|error| error.to_string())?;
        let native_end = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(rejected);
        let (key_ty, value_ty, is_set) = match ty {
            HirType::Map(key, value) => (key.as_ref(), Some(value.as_ref()), false),
            HirType::Set(element) => (element.as_ref(), None, true),
            _ => unreachable!(),
        };
        if !self.validating_native_argument {
            self.compile_throw_type_error(
                "Expected a native collection with a compatible key and value type",
            )?;
        } else {
            if value_ty.is_some_and(|value| !Self::ffi_map_copyin_value_is_word_compatible(value)) {
                self.compile_throw_type_error(
                    "JavaScript Map copy-in requires a single-word value type",
                )?;
            } else if !matches!(key_ty, HirType::F64 | HirType::Str | HirType::Json) {
                self.compile_throw_type_error(if is_set {
                    "JavaScript Set copy-in supports number, string, or any elements"
                } else {
                    "JavaScript Map copy-in supports number, string, or any keys"
                })?;
            } else {
                let snapshot = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_json_host_collection_snapshot")
                            .unwrap(),
                        &[
                            json.into(),
                            self.context
                                .i8_type()
                                .const_int(if is_set { 2 } else { 1 }, false)
                                .into(),
                        ],
                        "js_collection_snapshot",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("collection snapshot returned no value")?
                    .into_pointer_value();
                self.compile_check_json_host_error(snapshot.into(), Some("thaw_json_destroy"))?;
                let snapshot_present = self
                    .builder
                    .build_is_not_null(snapshot, "js_map_snapshot_present")
                    .map_err(|error| error.to_string())?;
                let snapshot_ok = self
                    .context
                    .append_basic_block(function, "js_map_snapshot_ok");
                let snapshot_failed = self
                    .context
                    .append_basic_block(function, "js_map_snapshot_failed");
                self.builder
                    .build_conditional_branch(snapshot_present, snapshot_ok, snapshot_failed)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(snapshot_failed);
                self.compile_throw_type_error(if is_set {
                    "Expected a JavaScript Set"
                } else {
                    "Expected a JavaScript Map"
                })?;
                self.builder.position_at_end(snapshot_ok);

                let map = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_map_new").unwrap(),
                        &[],
                        "ffi_copyin_map",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_map_new returned no value")?
                    .into_pointer_value();
                let allocated = self
                    .builder
                    .build_is_not_null(map, "ffi_copyin_map_allocated")
                    .map_err(|error| error.to_string())?;
                let map_ok = self
                    .context
                    .append_basic_block(function, "ffi_copyin_map_allocated");
                let map_failed = self
                    .context
                    .append_basic_block(function, "ffi_copyin_map_allocation_failed");
                self.builder
                    .build_conditional_branch(allocated, map_ok, map_failed)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(map_failed);
                self.builder
                    .build_call(
                        self.module.get_function("thaw_json_destroy").unwrap(),
                        &[snapshot.into()],
                        "destroy_failed_map_snapshot",
                    )
                    .map_err(|error| error.to_string())?;
                self.compile_throw_type_error(if is_set {
                    "Unable to allocate native Set"
                } else {
                    "Unable to allocate native Map"
                })?;
                self.builder.position_at_end(map_ok);

                let length = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_array_length").unwrap(),
                        &[snapshot.into()],
                        "js_map_entry_count",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_array_length returned no value")?
                    .into_int_value();
                let loop_header = self
                    .context
                    .append_basic_block(function, "js_map_copyin_header");
                let loop_body = self
                    .context
                    .append_basic_block(function, "js_map_copyin_body");
                let loop_done = self
                    .context
                    .append_basic_block(function, "js_map_copyin_done");
                self.builder
                    .build_unconditional_branch(loop_header)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(loop_header);
                let index = self
                    .builder
                    .build_phi(self.context.i64_type(), "js_map_copyin_index")
                    .map_err(|error| error.to_string())?;
                index.add_incoming(&[(&self.context.i64_type().const_zero(), map_ok)]);
                let has_entry = self
                    .builder
                    .build_int_compare(
                        IntPredicate::ULT,
                        index.as_basic_value().into_int_value(),
                        length,
                        "js_map_has_entry",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(has_entry, loop_body, loop_done)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(loop_body);
                let entry = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_index").unwrap(),
                        &[
                            snapshot.into(),
                            self.builder
                                .build_unsigned_int_to_float(
                                    index.as_basic_value().into_int_value(),
                                    self.context.f64_type(),
                                    "js_map_entry_index",
                                )
                                .map_err(|error| error.to_string())?
                                .into(),
                            self.context
                                .ptr_type(AddressSpace::default())
                                .const_null()
                                .into(),
                        ],
                        "js_map_entry",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Map entry index returned no value")?;
                self.compile_check_json_host_error(entry, Some("thaw_json_destroy"))?;
                let (key_json, value_json) = if is_set {
                    (entry, None)
                } else {
                    let read_pair = |this: &mut Self,
                                     item: BasicValueEnum<'ctx>,
                                     at: f64,
                                     name: &str|
                     -> Result<BasicValueEnum<'ctx>, String> {
                        let value = this
                            .builder
                            .build_call(
                                this.module.get_function("thaw_json_index").unwrap(),
                                &[
                                    item.into(),
                                    this.context.f64_type().const_float(at).into(),
                                    this.context
                                        .ptr_type(AddressSpace::default())
                                        .const_null()
                                        .into(),
                                ],
                                name,
                            )
                            .map_err(|error| error.to_string())?
                            .try_as_basic_value()
                            .basic()
                            .ok_or("Map pair index returned no value")?;
                        this.compile_check_json_host_error(value, Some("thaw_json_destroy"))?;
                        Ok(value)
                    };
                    (
                        read_pair(self, entry, 0.0, "js_map_key")?,
                        Some(read_pair(self, entry, 1.0, "js_map_value")?),
                    )
                };
                self.compile_validate_native_argument(key_json, key_ty)?;
                let key = self.compile_json_value_to_native(key_json, key_ty)?;
                if matches!(key_ty, HirType::Json) {
                    let rooted = self
                        .builder
                        .build_call(
                            self.module
                                .get_function("thaw_json_track_arena_owned_root")
                                .unwrap(),
                            &[key.into()],
                            "root_ffi_map_key",
                        )
                        .map_err(|error| error.to_string())?
                        .try_as_basic_value()
                        .basic()
                        .ok_or("failed to root Json Map key")?;
                    self.compile_check_json_host_error(rooted, None)?;
                }
                let value = if let (Some(value_json), Some(value_ty)) = (value_json, value_ty) {
                    let value = self.compile_json_value_to_native(value_json, value_ty)?;
                    if matches!(value_ty, HirType::Json | HirType::Dictionary(_)) {
                        let rooted = self
                            .builder
                            .build_call(
                                self.module
                                    .get_function("thaw_json_track_arena_owned_root")
                                    .unwrap(),
                                &[value.into()],
                                "root_ffi_map_value",
                            )
                            .map_err(|error| error.to_string())?
                            .try_as_basic_value()
                            .basic()
                            .ok_or("failed to root Json Map value")?;
                        self.compile_check_json_host_error(rooted, None)?;
                    }
                    value
                } else {
                    self.context.i64_type().const_zero().into()
                };
                let key_suffix = match key_ty {
                    HirType::F64 => "num",
                    HirType::Str => "str",
                    _ => "any",
                };
                let value_word = self.encode_word(value)?;
                let set = self
                    .builder
                    .build_call(
                        self.module
                            .get_function(&format!("thaw_map_{key_suffix}_set"))
                            .unwrap(),
                        &[map.into(), key.into(), value_word.into()],
                        "ffi_copyin_map_insert",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Map insert returned no status")?
                    .into_int_value();
                self.compile_check_json_host_error(
                    value_json.unwrap_or(key_json),
                    Some("thaw_json_destroy"),
                )?;
                let inserted = self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        set,
                        self.context.i8_type().const_zero(),
                        "ffi_copyin_map_inserted",
                    )
                    .map_err(|error| error.to_string())?;
                let inserted_block = self
                    .context
                    .append_basic_block(function, "ffi_copyin_map_inserted");
                let insert_failed = self
                    .context
                    .append_basic_block(function, "ffi_copyin_map_insert_failed");
                self.builder
                    .build_conditional_branch(inserted, inserted_block, insert_failed)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(insert_failed);
                self.compile_throw_type_error(if is_set {
                    "Unable to grow native Set"
                } else {
                    "Unable to grow native Map"
                })?;
                self.builder.position_at_end(inserted_block);
                self.compile_destroy_decoded_owned_json(key_json, key_ty)?;
                if let (Some(value_json), Some(value_ty)) = (value_json, value_ty) {
                    self.compile_destroy_decoded_owned_json(value_json, value_ty)?;
                }
                if !is_set {
                    self.builder
                        .build_call(
                            self.module.get_function("thaw_json_destroy").unwrap(),
                            &[entry.into()],
                            "destroy_map_entry_pair",
                        )
                        .map_err(|error| error.to_string())?;
                }
                let next = self
                    .builder
                    .build_int_add(
                        index.as_basic_value().into_int_value(),
                        self.context.i64_type().const_int(1, false),
                        "js_map_next_index",
                    )
                    .map_err(|error| error.to_string())?;
                let body_end = self.builder.get_insert_block().unwrap();
                self.builder
                    .build_unconditional_branch(loop_header)
                    .map_err(|error| error.to_string())?;
                index.add_incoming(&[(&next, body_end)]);
                self.builder.position_at_end(loop_done);
                self.builder
                    .build_call(
                        self.module.get_function("thaw_json_destroy").unwrap(),
                        &[snapshot.into()],
                        "destroy_map_snapshot",
                    )
                    .map_err(|error| error.to_string())?;
                let copyin_end = self.builder.get_insert_block().unwrap();
                self.builder
                    .build_unconditional_branch(done)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(done);
                let result = self
                    .builder
                    .build_phi(
                        self.context.ptr_type(AddressSpace::default()),
                        "native_collection_result",
                    )
                    .map_err(|error| error.to_string())?;
                result.add_incoming(&[(&native, native_end), (&map, copyin_end)]);
                return Ok(result.as_basic_value());
            }
        }
        self.builder.position_at_end(done);
        let result = self
            .builder
            .build_phi(
                self.context.ptr_type(AddressSpace::default()),
                "native_collection_result",
            )
            .map_err(|error| error.to_string())?;
        result.add_incoming(&[(&native, native_end)]);
        Ok(result.as_basic_value())
    }

    /// The native Map ABI stores one 64-bit word per value. Keep wrappers and
    /// unions on the native-pointer fast path, but reject them in the JS
    /// copy-in branch unless they have an existing single-word representation.
    fn ffi_map_copyin_value_is_word_compatible(ty: &HirType) -> bool {
        match ty {
            HirType::Optional(_)
            | HirType::Nullable(_)
            | HirType::Nullish(_)
            | HirType::Union(_)
            | HirType::JsValue => false,
            HirType::OverloadedFunction(_, physical) => {
                Self::ffi_map_copyin_value_is_word_compatible(physical)
            }
            _ => true,
        }
    }

    fn compile_json_value_to_native(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_validate_native_argument(json, ty)?;
        match ty {
            HirType::OverloadedFunction(_, physical) => {
                self.compile_json_value_to_native(json, physical)
            }
            HirType::F64 => self.compile_json_as_value(json, "thaw_json_as_number"),
            HirType::I64 => {
                let value = self.compile_json_as_value(json, "thaw_json_as_bigint_i64")?;
                self.compile_check_json_host_error(json, None)?;
                Ok(value)
            }
            HirType::Str => self.compile_json_as_value(json, "thaw_json_as_string_value"),
            HirType::Bool => self.compile_json_as_bool_value(json),
            HirType::Json | HirType::Dictionary(_) => Ok(json),
            HirType::Null => Ok(self.context.bool_type().const_int(1, false).into()),
            HirType::Undefined => Ok(self.context.bool_type().const_zero().into()),
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
            HirType::Recursive(key) => self.compile_recursive_from_json(json, key),
            // A `Union`-typed object field (or callback argument) decodes by
            // runtime `typeof` discrimination -- the same
            // `compile_json_to_union_result` `compile_json_to_native`'s own
            // `Union` arm uses. Without this, an object-shaped JS value's
            // union field failed with "unsupported dynamic result value
            // Union(...)".
            HirType::Union(elements) => self.compile_json_to_union_result(json, elements),
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
            HirType::JsValue => self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_handle_id").unwrap(),
                    &[json.into()],
                    "native_callback_jsvalue_arg",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "thaw_json_handle_id did not return a value".into()),
            HirType::Map(..) | HirType::Set(_) => self.compile_json_to_native_collection(json, ty),
            HirType::Function(params, ret) => self.compile_js_callback_from_json(json, params, ret),
            HirType::CallableFunction(params, _, None, ret) => {
                self.compile_js_callback_from_json(json, params, ret)
            }
            // A rest callable's physical ABI is its fixed prefix plus one packed `Array` of the
            // rest element; the adapter spreads that array back into individual JS arguments.
            HirType::CallableFunction(params, _, Some(rest), ret) => {
                let mut physical = params.clone();
                physical.push(HirType::Array(rest.clone()));
                self.compile_js_callback_from_json_spreading(json, &physical, ret, true)
            }
            // A live JS Promise handed to a `Promise<T>`-typed callback parameter (e.g.
            // `process.on("unhandledRejection", (reason, promise: Promise<unknown>) => ...)`)
            // becomes a fresh native Promise that never settles: it is a real Promise for
            // `instanceof`/identity use and does not abort the callback.
            // ponytail: no settlement mirroring; adopt the JS promise's state if callers need `.then`.
            HirType::Promise(_) => Ok(self
                .builder
                .build_call(
                    self.module.get_function("thaw_promise_new").unwrap(),
                    &[],
                    "foreign_promise_placeholder",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_promise_new returned no value")?),
            other => Err(format!("unsupported dynamic result value {other:?}")),
        }
    }

    /// A decoded Json/Dictionary field transfers its owned Box into native
    /// storage. Other fields have detached storage; tagged Json payloads keep
    /// the Box only on a present branch. The parent input is independent.
    fn compile_destroy_decoded_owned_json(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<(), String> {
        if matches!(ty, HirType::Json | HirType::Dictionary(_)) {
            return Ok(());
        }
        let payload = match ty {
            HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
                Some(payload.as_ref())
            }
            _ => None,
        };
        let absent = if payload.is_some_and(Self::typed_result_embeds_input_json) {
            Some(self.compile_typed_input_absent(json, ty)?)
        } else {
            None
        };
        self.compile_destroy_typed_input_if_detached(json, ty, absent)
    }

    fn compile_destroy_optional_result_container(
        &mut self,
        object: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[object.into()],
                "destroy_optional_result_container",
            )
            .map_err(|error| error.to_string())?;
        Ok(())
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
            // `Nullable`'s tagged representation only has two states --
            // a real payload, or "null" -- with nothing to represent a
            // genuinely *missing* key/index as (unlike `Nullish`, which
            // has a separate `undefined` state below). Since `thaw_
            // json_get`/`thaw_json_index` (thaw-std) now distinguish
            // "missing" from real `null` at the `Json` representation
            // level (a fresh `$__thaw_napi_undefined$`-tagged sentinel,
            // not `Value::Null`), a strict null check alone would treat
            // that sentinel as "present" and try to decode it as a real
            // payload value -- garbage (e.g. `0` for a numeric payload)
            // instead of the closest available state, "null". Treat
            // either one as absent.
            let is_null = self.compile_json_is_null_value(json)?;
            let is_undefined = self.compile_json_is_napi_undefined(json)?;
            let is_absent = self
                .builder
                .build_or(is_null, is_undefined, "json_nullable_absent")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_not(is_absent, "json_nullable_present")
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
            self.compile_check_json_host_error(has_own.into(), None)?;
            let has_own = self
                .builder
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
        let value_block = self
            .context
            .append_basic_block(function, "json_tagged_value");
        let absent_block = self
            .context
            .append_basic_block(function, "json_tagged_absent");
        let done = self
            .context
            .append_basic_block(function, "json_tagged_done");
        self.builder
            .build_conditional_branch(present, value_block, absent_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_block);
        let payload_value = self.compile_json_value_to_native(json, payload)?;
        let value = self.build_optional_value(payload_value, payload, true)?;
        let value_end = self
            .builder
            .get_insert_block()
            .ok_or("lost tagged value block")?;
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
        self.compile_check_json_host_error(has_own.into(), None)?;
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
        let present_block = self
            .context
            .append_basic_block(function, "json_nullish_present");
        let undefined_block = self
            .context
            .append_basic_block(function, "json_nullish_undefined");
        let null_block = self
            .context
            .append_basic_block(function, "json_nullish_null");
        let value_block = self
            .context
            .append_basic_block(function, "json_nullish_value");
        let done = self
            .context
            .append_basic_block(function, "json_nullish_done");
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
        self.compile_validate_native_argument(json, &HirType::Array(Box::new(element.clone())))?;
        // A native array view passed through a dynamic callback still carries its original owner.
        // Recover it so typed callback arguments keep the same array handle instead of decoding a copy.
        if !self.thaw_js_routed {
            self.uses_quickjs = true;
            self.uses_quickjs_handles = true;
        }
        let layout = format!("array:{element:?}");
        let (pointer, present) = self.query_native_receiver(json.into_pointer_value(), &layout)?;
        let function = self.current_function();
        let native = self
            .context
            .append_basic_block(function, "json_native_array_owner");
        let copy = self
            .context
            .append_basic_block(function, "json_native_array_copy");
        let done = self
            .context
            .append_basic_block(function, "json_native_array_ready");
        self.builder
            .build_conditional_branch(present, native, copy)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(native);
        let native = self
            .builder
            .build_int_to_ptr(
                pointer,
                self.context.ptr_type(AddressSpace::default()),
                "json_native_array_owner_handle",
            )
            .map_err(|error| error.to_string())?;
        let native_end = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(copy);
        let copied = self.compile_json_to_native_array_copy(json, element)?;
        let copy_end = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(done);
        let result = self
            .builder
            .build_phi(
                self.context.ptr_type(AddressSpace::default()),
                "json_native_array",
            )
            .map_err(|error| error.to_string())?;
        result.add_incoming(&[(&native, native_end), (&copied, copy_end)]);
        Ok(result.as_basic_value())
    }

    fn compile_json_to_native_array_copy(
        &mut self,
        json: BasicValueEnum<'ctx>,
        element: &HirType,
    ) -> Result<PointerValue<'ctx>, String> {
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
        self.compile_check_json_host_error(length.into(), None)?;
        let element_bytes = array_element_storage_bytes(element)?;
        let allocation_size =
            self.checked_array_allocation_size(length, element_bytes, "json_native_array")?;
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    allocation_size.into(),
                    i64_type.const_int(element_bytes.min(8), false).into(),
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
        let body = self
            .context
            .append_basic_block(function, "json_array_element");
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
        self.compile_check_json_host_error(element_json, Some("thaw_json_destroy"))?;
        self.compile_validate_native_argument(element_json, element)?;
        let value = match element {
            HirType::F64 => self.compile_json_as_value(element_json, "thaw_json_as_number")?,
            HirType::Str => self.compile_json_as_value(element_json, "thaw_json_as_string")?,
            HirType::Bool => self.compile_json_as_bool_value(element_json)?,
            HirType::Json | HirType::Dictionary(_) => element_json,
            HirType::Optional(payload) => {
                let (object, key) = self.compile_napi_optional_result_container(element_json)?;
                let value =
                    self.compile_json_to_optional_field(object, key, element_json, payload, false)?;
                self.compile_destroy_optional_result_container(object)?;
                value
            }
            HirType::Nullable(payload) => {
                self.compile_json_to_nullable_field(element_json, payload)?
            }
            HirType::Nullish(payload) => {
                let (object, key) = self.compile_napi_optional_result_container(element_json)?;
                let value =
                    self.compile_json_to_nullish_field(object, key, element_json, payload)?;
                self.compile_destroy_optional_result_container(object)?;
                value
            }
            HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) | HirType::Union(_) => {
                self.compile_json_to_native(element_json, element)?
            }
            other => return Err(format!("unsupported JSON array element {other:?}")),
        };
        let offset = self
            .builder
            .build_int_mul(
                index.as_basic_value().into_int_value(),
                self.context.i64_type().const_int(element_bytes, false),
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
        self.compile_destroy_decoded_owned_json(element_json, element)?;
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
        self.compile_array_wrap(array)
    }

    fn compile_json_to_native_tuple(
        &mut self,
        json: BasicValueEnum<'ctx>,
        elements: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_validate_native_argument(json, &HirType::Tuple(elements.to_vec()))?;
        let i64_type = self.context.i64_type();
        let element_bytes = tuple_element_storage_bytes(elements)?;
        let size = checked_array_offset(element_bytes, elements.len())?;
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
                        self.context
                            .ptr_type(AddressSpace::default())
                            .const_null()
                            .into(),
                    ],
                    "json_tuple_element",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_json_index returned no tuple element")?;
            self.compile_check_json_host_error(element_json, Some("thaw_json_destroy"))?;
            self.compile_validate_native_argument(element_json, element)?;
            let value = match element {
                HirType::F64 => self.compile_json_as_value(element_json, "thaw_json_as_number")?,
                HirType::Str => self.compile_json_as_value(element_json, "thaw_json_as_string")?,
                HirType::Bool => self.compile_json_as_bool_value(element_json)?,
                HirType::Json | HirType::Dictionary(_) => element_json,
                // A statically `null`/`undefined`-typed tuple position
                // carries no information beyond its own type -- there's
                // nothing to read from `element_json`, just the same
                // fixed dummy-bool sentinel every other `Null`/
                // `Undefined` value compiles to (see
                // `values/expressions.rs`'s literal codegen).
                HirType::Null => self.context.bool_type().const_int(1, false).into(),
                HirType::Undefined => self.context.bool_type().const_zero().into(),
                HirType::Optional(payload) => {
                    let (object, key) =
                        self.compile_napi_optional_result_container(element_json)?;
                    let value = self.compile_json_to_optional_field(
                        object,
                        key,
                        element_json,
                        payload,
                        false,
                    )?;
                    self.compile_destroy_optional_result_container(object)?;
                    value
                }
                HirType::Nullable(payload) => {
                    self.compile_json_to_nullable_field(element_json, payload)?
                }
                HirType::Nullish(payload) => {
                    let (object, key) =
                        self.compile_napi_optional_result_container(element_json)?;
                    let value =
                        self.compile_json_to_nullish_field(object, key, element_json, payload)?;
                    self.compile_destroy_optional_result_container(object)?;
                    value
                }
                HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) | HirType::Union(_) => {
                    self.compile_json_to_native(element_json, element)?
                }
                other => return Err(format!("unsupported JSON tuple element {other:?}")),
            };
            let offset = i64_type.const_int(checked_array_offset(element_bytes, index)?, false);
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
            self.compile_destroy_decoded_owned_json(element_json, element)?;
        }
        // `tuple` is a freshly built raw buffer; wrap it in a handle before
        // treating it as this call's tuple-typed return value (see
        // `compile_array_wrap`'s doc comment).
        Ok(self.compile_array_wrap(tuple)?.into())
    }
}
