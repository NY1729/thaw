impl<'ctx> HirCompiler<'ctx> {
    fn apply_ffi_string_ownership(
        &mut self,
        source: PointerValue<'ctx>,
        ownership: &FfiOwnership,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        if *ownership == FfiOwnership::Borrowed {
            return Ok(source);
        }

        let function = self.current_function();
        let null_bb = self
            .context
            .append_basic_block(function, &format!("{name}_null"));
        let copy_bb = self
            .context
            .append_basic_block(function, &format!("{name}_copy"));
        let merge_bb = self
            .context
            .append_basic_block(function, &format!("{name}_merge"));
        let is_null = self
            .builder
            .build_is_null(source, &format!("{name}_is_null"))
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(is_null, null_bb, copy_bb)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(null_bb);
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(copy_bb);
        let strlen = self.module.get_function("thaw_string_byte_length").unwrap();
        let length = self
            .builder
            .build_call(strlen, &[source.into()], &format!("{name}_length"))
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let size = self
            .builder
            .build_int_add(
                length,
                self.context.i64_type().const_int(1, false),
                &format!("{name}_size"),
            )
            .map_err(|error| error.to_string())?;
        let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
        let copied = self
            .builder
            .build_call(
                arena_alloc,
                &[
                    size.into(),
                    self.context.i64_type().const_int(1, false).into(),
                ],
                &format!("{name}_alloc"),
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        let memcpy = self.module.get_function("memcpy").unwrap();
        self.builder
            .build_call(
                memcpy,
                &[copied.into(), source.into(), size.into()],
                &format!("{name}_memcpy"),
            )
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_string_register").unwrap(),
            &[copied.into(), length.into()], "register_owned_string_length").map_err(|error| error.to_string())?;
        let destroy = match ownership {
            FfiOwnership::Owned { destroy } => Some(destroy),
            FfiOwnership::ArenaCopy { destroy } => destroy.as_ref(),
            FfiOwnership::Borrowed => None,
        };
        if let Some(destroy) = destroy {
            let destroy_fn = self
                .module
                .get_function(destroy)
                .ok_or_else(|| format!("FFI ownership destructor `{destroy}` was not declared"))?;
            self.builder
                .build_call(destroy_fn, &[source.into()], &format!("{name}_destroy"))
                .map_err(|error| error.to_string())?;
        }
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(merge_bb);
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let phi = self
            .builder
            .build_phi(ptr_ty, &format!("{name}_owned"))
            .map_err(|error| error.to_string())?;
        let null = ptr_ty.const_null();
        phi.add_incoming(&[(&null, null_bb), (&copied, copy_bb)]);
        Ok(phi.as_basic_value().into_pointer_value())
    }

    fn unpack_ffi_bool_array(
        &mut self,
        native: StructValue<'ctx>,
        ownership: &FfiOwnership,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let data = self
            .builder
            .build_extract_value(native, 0, "ffi_bool_array_data")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let length = self
            .builder
            .build_extract_value(native, 1, "ffi_bool_array_length")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let i64_type = self.context.i64_type();
        let payload_size = self
            .builder
            .build_int_mul(
                length,
                i64_type.const_int(ARRAY_ELEM_BYTES, false),
                "ffi_bool_array_payload_size",
            )
            .map_err(|error| error.to_string())?;
        let size = self
            .builder
            .build_int_add(
                payload_size,
                i64_type.const_int(ARRAY_HEADER_BYTES, false),
                "ffi_bool_array_size",
            )
            .map_err(|error| error.to_string())?;
        let result = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[size.into(), i64_type.const_int(8, false).into()],
                "ffi_bool_array_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc returned no boolean-array result")?
            .into_pointer_value();
        self.builder
            .build_store(result, length)
            .map_err(|error| error.to_string())?;

        let function = self.current_function();
        let entry = self
            .builder
            .get_insert_block()
            .ok_or("boolean-array return has no current block")?;
        let condition = self.context.append_basic_block(function, "ffi_bool_return_next");
        let body = self.context.append_basic_block(function, "ffi_bool_return_body");
        let done = self.context.append_basic_block(function, "ffi_bool_return_done");
        self.builder
            .build_unconditional_branch(condition)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(condition);
        let index = self
            .builder
            .build_phi(i64_type, "ffi_bool_return_index")
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&i64_type.const_zero(), entry)]);
        let current = index.as_basic_value().into_int_value();
        let has_element = self
            .builder
            .build_int_compare(IntPredicate::ULT, current, length, "ffi_bool_return_has_element")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(has_element, body, done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(body);
        let source = unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), data, &[current], "ffi_bool_return_source")
                .map_err(|error| error.to_string())?
        };
        let byte = self
            .builder
            .build_load(self.context.i8_type(), source, "ffi_bool_return_byte")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let boolean = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                byte,
                self.context.i8_type().const_zero(),
                "ffi_bool_return_value",
            )
            .map_err(|error| error.to_string())?;
        let target_offset = self
            .builder
            .build_int_mul(
                current,
                i64_type.const_int(ARRAY_ELEM_BYTES, false),
                "ffi_bool_return_target_offset",
            )
            .map_err(|error| error.to_string())?;
        let target_offset = self
            .builder
            .build_int_add(
                target_offset,
                i64_type.const_int(ARRAY_HEADER_BYTES, false),
                "ffi_bool_return_payload_offset",
            )
            .map_err(|error| error.to_string())?;
        let target = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    result,
                    &[target_offset],
                    "ffi_bool_return_target",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(target, boolean)
            .map_err(|error| error.to_string())?;
        let next = self
            .builder
            .build_int_add(current, i64_type.const_int(1, false), "ffi_bool_return_increment")
            .map_err(|error| error.to_string())?;
        let body_end = self
            .builder
            .get_insert_block()
            .ok_or("boolean-array return lost its body block")?;
        self.builder
            .build_unconditional_branch(condition)
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&next, body_end)]);
        self.builder.position_at_end(done);
        let destroy = match ownership {
            FfiOwnership::Owned { destroy } => Some(destroy),
            FfiOwnership::ArenaCopy { destroy } => destroy.as_ref(),
            FfiOwnership::Borrowed => None,
        };
        if let Some(destroy) = destroy {
            let destroy_fn = self.module.get_function(destroy).ok_or_else(|| {
                format!("FFI ownership destructor `{destroy}` was not declared")
            })?;
            self.builder
                .build_call(destroy_fn, &[data.into()], "ffi_bool_array_destroy")
                .map_err(|error| error.to_string())?;
        }
        Ok(self.compile_array_wrap(result)?.into())
    }

    fn marshal_ffi_tagged_return(
        &mut self,
        native: StructValue<'ctx>,
        ty: &HirType,
        payload: &HirType,
        ownership: &FfiOwnership,
        aggregate_abi: FfiAggregateAbi,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let tag = self
            .builder
            .build_extract_value(native, 0, "ffi_tagged_return_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let native_value = self
            .builder
            .build_extract_value(native, 1, "ffi_tagged_return_payload")
            .map_err(|error| error.to_string())?;
        let present = if matches!(ty, HirType::Nullish(_)) {
            self.builder.build_int_compare(IntPredicate::EQ, tag, tag.get_type().const_zero(), "ffi_tagged_return_present")
        } else {
            self.builder.build_int_compare(IntPredicate::NE, tag, tag.get_type().const_zero(), "ffi_tagged_return_present")
        }.map_err(|error| error.to_string())?;
        let function = self.current_function();
        let present_bb = self.context.append_basic_block(function, "ffi_tagged_payload_present");
        let absent_bb = self.context.append_basic_block(function, "ffi_tagged_payload_absent");
        let merge_bb = self.context.append_basic_block(function, "ffi_tagged_payload_merge");
        self.builder.build_conditional_branch(present, present_bb, absent_bb).map_err(|error| error.to_string())?;
        self.builder.position_at_end(present_bb);
        let value = match payload {
            HirType::Str if *ownership != FfiOwnership::Borrowed => self
                .apply_ffi_string_ownership(
                    native_value.into_pointer_value(),
                    ownership,
                    "ffi_tagged_return_string",
                )?
                .into(),
            HirType::Array(_)
            | HirType::Tuple(_)
            | HirType::Object(_)
            | HirType::Optional(_)
            | HirType::Nullable(_)
            | HirType::Nullish(_)
                if native_value.is_struct_value() => self.marshal_ffi_return(
                native_value,
                payload,
                FfiStringAbi::NullTerminated,
                ownership,
                aggregate_abi,
                None,
            )?,
            _ => native_value,
        };
        let present_end = self.builder.get_insert_block().unwrap();
        self.builder.build_unconditional_branch(merge_bb).map_err(|error| error.to_string())?;
        self.builder.position_at_end(absent_bb);
        self.builder.build_unconditional_branch(merge_bb).map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge_bb);
        let payload_type = self.basic_type(payload)?;
        let merged = self.builder.build_phi(payload_type, "ffi_tagged_payload").map_err(|error| error.to_string())?;
        let absent_value = payload_type.const_zero();
        merged.add_incoming(&[(&value, present_end), (&absent_value, absent_bb)]);
        let value = merged.as_basic_value();
        if matches!(ty, HirType::Nullish(_)) {
            return self.build_nullish_tagged_value(value, payload, tag);
        }
        let tagged_type = self.basic_type(ty)?.into_struct_type();
        let tagged = self
            .builder
            .build_insert_value(
                tagged_type.get_undef(),
                present,
                0,
                "ffi_tagged_return_with_tag",
            )
            .map_err(|error| error.to_string())?
            .into_struct_value();
        self.builder
            .build_insert_value(tagged, value, 1, "ffi_tagged_return_with_payload")
            .map(|value| value.into_struct_value().into())
            .map_err(|error| error.to_string())
    }

    fn marshal_ffi_tuple_return(
        &mut self,
        native: StructValue<'ctx>,
        elements: &[HirType],
        ownership: &FfiOwnership,
        aggregate_abi: FfiAggregateAbi,
    ) -> Result<BasicValueEnum<'ctx>, String> {
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
                "ffi_tuple_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc returned no tuple result")?
            .into_pointer_value();
        self.builder
            .build_store(tuple, i64_type.const_int(elements.len() as u64, false))
            .map_err(|error| error.to_string())?;
        for (index, element) in elements.iter().enumerate() {
            let mut value = self
                .builder
                .build_extract_value(native, index as u32, "ffi_tuple_return_element")
                .map_err(|error| error.to_string())?;
            value = match element {
                HirType::Str if *ownership != FfiOwnership::Borrowed => self
                    .apply_ffi_string_ownership(
                        value.into_pointer_value(),
                        ownership,
                        "ffi_tuple_return_string",
                    )?
                    .into(),
                HirType::Array(_)
                | HirType::Tuple(_)
                | HirType::Object(_)
                | HirType::Optional(_)
                | HirType::Nullable(_)
                | HirType::Nullish(_)
                    if value.is_struct_value() => self.marshal_ffi_return(
                    value,
                    element,
                    FfiStringAbi::NullTerminated,
                    ownership,
                    aggregate_abi,
                    None,
                )?,
                _ => value,
            };
            let offset = i64_type.const_int(
                checked_array_offset(element_bytes, index)?,
                false,
            );
            let pointer = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        tuple,
                        &[offset],
                        "ffi_tuple_result_slot",
                    )
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(pointer, value)
                .map_err(|error| error.to_string())?;
        }
        Ok(self.compile_array_wrap(tuple)?.into())
    }

    fn marshal_ffi_return(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &HirType,
        string_abi: FfiStringAbi,
        ownership: &FfiOwnership,
        aggregate_abi: FfiAggregateAbi,
        aggregate_layout: Option<&FfiAggregateLayout>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            HirType::Str if string_abi == FfiStringAbi::PointerLength => {
                let native = value.into_struct_value();
                let source = self
                    .builder
                    .build_extract_value(native, 0, "ffi_string_data")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(native, 1, "ffi_string_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let i64_type = self.context.i64_type();
                let size = self
                    .builder
                    .build_int_add(length, i64_type.const_int(1, false), "ffi_string_size")
                    .map_err(|error| error.to_string())?;
                let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
                let result = self
                    .builder
                    .build_call(
                        arena_alloc,
                        &[size.into(), i64_type.const_int(1, false).into()],
                        "ffi_string_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_pointer_value();
                let memcpy = self.module.get_function("memcpy").unwrap();
                self.builder
                    .build_call(
                        memcpy,
                        &[result.into(), source.into(), length.into()],
                        "ffi_string_copy",
                    )
                    .map_err(|error| error.to_string())?;
                let terminator = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            result,
                            &[length],
                            "ffi_string_terminator",
                        )
                        .map_err(|error| error.to_string())?
                };
                self.builder
                    .build_store(terminator, self.context.i8_type().const_zero())
                    .map_err(|error| error.to_string())?;
                self.builder.build_call(self.module.get_function("thaw_string_register").unwrap(),
                    &[result.into(), length.into()], "register_ffi_string_length").map_err(|error| error.to_string())?;
                let destroy = match ownership {
                    FfiOwnership::Owned { destroy } => Some(destroy),
                    FfiOwnership::ArenaCopy { destroy } => destroy.as_ref(),
                    FfiOwnership::Borrowed => None,
                };
                if let Some(destroy) = destroy {
                    let destroy_fn = self.module.get_function(destroy).ok_or_else(|| {
                        format!("FFI ownership destructor `{destroy}` was not declared")
                    })?;
                    self.builder
                        .build_call(destroy_fn, &[source.into()], "ffi_string_destroy")
                        .map_err(|error| error.to_string())?;
                }
                Ok(result.into())
            }
            HirType::Array(element)
                if **element == HirType::Bool && value.is_struct_value() =>
            {
                self.unpack_ffi_bool_array(value.into_struct_value(), ownership)
            }
            HirType::Array(element)
                if matches!(element.as_ref(), HirType::F64 | HirType::Str | HirType::JsValue)
                    && value.is_struct_value() =>
            {
                let native = value.into_struct_value();
                let data = self
                    .builder
                    .build_extract_value(native, 0, "ffi_array_data")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(native, 1, "ffi_array_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let i64_type = self.context.i64_type();
                let bytes = self
                    .builder
                    .build_int_mul(
                        length,
                        i64_type.const_int(ARRAY_ELEM_BYTES, false),
                        "ffi_array_bytes",
                    )
                    .map_err(|error| error.to_string())?;
                let size = self
                    .builder
                    .build_int_add(
                        bytes,
                        i64_type.const_int(ARRAY_HEADER_BYTES, false),
                        "ffi_array_size",
                    )
                    .map_err(|error| error.to_string())?;
                let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
                let result = self
                    .builder
                    .build_call(
                        arena_alloc,
                        &[size.into(), i64_type.const_int(8, false).into()],
                        "ffi_array_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_pointer_value();
                self.builder
                    .build_store(result, length)
                    .map_err(|error| error.to_string())?;
                let elements = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            result,
                            &[i64_type.const_int(ARRAY_HEADER_BYTES, false)],
                            "ffi_array_elements",
                        )
                        .map_err(|error| error.to_string())?
                };
                let memcpy = self.module.get_function("memcpy").unwrap();
                self.builder
                    .build_call(
                        memcpy,
                        &[elements.into(), data.into(), bytes.into()],
                        "ffi_array_copy",
                    )
                    .map_err(|error| error.to_string())?;
                if **element == HirType::Str && *ownership != FfiOwnership::Borrowed {
                    let function = self.current_function();
                    let entry = self.builder.get_insert_block().unwrap();
                    let loop_bb = self.context.append_basic_block(function, "ffi_string_array_loop");
                    let body_bb = self.context.append_basic_block(function, "ffi_string_array_body");
                    let done_bb = self.context.append_basic_block(function, "ffi_string_array_done");
                    self.builder.build_unconditional_branch(loop_bb).map_err(|error| error.to_string())?;
                    self.builder.position_at_end(loop_bb);
                    let index = self.builder.build_phi(i64_type, "ffi_string_array_index").map_err(|error| error.to_string())?;
                    index.add_incoming(&[(&i64_type.const_zero(), entry)]);
                    let current = index.as_basic_value().into_int_value();
                    let more = self.builder.build_int_compare(IntPredicate::ULT, current, length, "ffi_string_array_more").map_err(|error| error.to_string())?;
                    self.builder.build_conditional_branch(more, body_bb, done_bb).map_err(|error| error.to_string())?;
                    self.builder.position_at_end(body_bb);
                    let slot = unsafe { self.builder.build_in_bounds_gep(
                        self.context.ptr_type(AddressSpace::default()), elements, &[current], "ffi_string_array_slot",
                    ).map_err(|error| error.to_string())? };
                    let source = self.builder.build_load(
                        self.context.ptr_type(AddressSpace::default()), slot, "ffi_string_array_source",
                    ).map_err(|error| error.to_string())?.into_pointer_value();
                    let copied = self.apply_ffi_string_ownership(
                        source, &FfiOwnership::ArenaCopy { destroy: None }, "ffi_string_array_element",
                    )?;
                    self.builder.build_store(slot, copied).map_err(|error| error.to_string())?;
                    let next = self.builder.build_int_add(current, i64_type.const_int(1, false), "ffi_string_array_next").map_err(|error| error.to_string())?;
                    let body_end = self.builder.get_insert_block().unwrap();
                    self.builder.build_unconditional_branch(loop_bb).map_err(|error| error.to_string())?;
                    index.add_incoming(&[(&next, body_end)]);
                    self.builder.position_at_end(done_bb);
                }
                let destroy = match ownership {
                    FfiOwnership::Owned { destroy } => Some(destroy),
                    FfiOwnership::ArenaCopy { destroy } => destroy.as_ref(),
                    FfiOwnership::Borrowed => None,
                };
                if let Some(destroy) = destroy {
                    let destroy_fn = self.module.get_function(destroy).ok_or_else(|| {
                        format!("FFI ownership destructor `{destroy}` was not declared")
                    })?;
                    self.builder
                        .build_call(destroy_fn, &[data.into()], "ffi_array_destroy")
                        .map_err(|error| error.to_string())?;
                }
                Ok(self.compile_array_wrap(result)?.into())
            }
            HirType::Optional(payload) | HirType::Nullable(payload)
                if value.is_struct_value() && aggregate_abi != FfiAggregateAbi::Internal =>
            {
                self.marshal_ffi_tagged_return(
                    value.into_struct_value(),
                    ty,
                    payload,
                    ownership,
                    aggregate_abi,
                )
            }
            HirType::Nullish(payload)
                if value.is_struct_value() && aggregate_abi != FfiAggregateAbi::Internal =>
            {
                self.marshal_ffi_tagged_return(
                    value.into_struct_value(),
                    ty,
                    payload,
                    ownership,
                    aggregate_abi,
                )
            }
            HirType::Tuple(elements)
                if value.is_struct_value() && aggregate_abi != FfiAggregateAbi::Internal =>
            {
                self.marshal_ffi_tuple_return(
                    value.into_struct_value(),
                    elements,
                    ownership,
                    aggregate_abi,
                )
            }
            HirType::Object(fields)
                if value.is_struct_value() && aggregate_abi != FfiAggregateAbi::Internal =>
            {
                let native = value.into_struct_value();
                let field_indices = aggregate_layout
                    .map(|layout| {
                        self.ffi_explicit_object_type(fields, layout)
                            .map(|(_, indices)| indices)
                    })
                    .transpose()?;
                let i64_type = self.context.i64_type();
                let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
                let result = self
                    .builder
                    .build_call(
                        arena_alloc,
                        &[
                            i64_type
                                .const_int(object_storage_bytes(fields)?, false)
                                .into(),
                            i64_type.const_int(OBJECT_FIELD_BYTES, false).into(),
                        ],
                        "ffi_object_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_pointer_value();
                for (index, (name, field_ty)) in fields.iter().enumerate() {
                    let explicit_field = field_indices.as_ref().map(|indices| &indices[index]);
                    let mut field = self
                        .builder
                        .build_extract_value(
                            native,
                            explicit_field.map_or(index as u32, |field| field.native_index),
                            &format!("ffi_{name}"),
                        )
                        .map_err(|error| error.to_string())?;
                    if let Some(bitfield) = explicit_field.and_then(|field| field.bitfield.as_ref())
                    {
                        let storage = field.into_int_value();
                        let storage_type = storage.get_type();
                        let shifted = self
                            .builder
                            .build_right_shift(
                                storage,
                                storage_type.const_int(u64::from(bitfield.bit_offset), false),
                                false,
                                &format!("ffi_{name}_shift"),
                            )
                            .map_err(|error| error.to_string())?;
                        let storage_bits = storage_type.get_bit_width();
                        let width = u32::from(bitfield.bit_width);
                        let mask = if width == 64 {
                            u64::MAX
                        } else {
                            (1_u64 << width) - 1
                        };
                        let masked = self
                            .builder
                            .build_and(
                                shifted,
                                storage_type.const_int(mask, false),
                                &format!("ffi_{name}_mask"),
                            )
                            .map_err(|error| error.to_string())?;
                        field = match field_ty {
                            HirType::Bool => self
                                .builder
                                .build_int_compare(
                                    IntPredicate::NE,
                                    masked,
                                    storage_type.const_zero(),
                                    &format!("ffi_{name}_bool"),
                                )
                                .map_err(|error| error.to_string())?
                                .into(),
                            HirType::F64 => {
                                let integer = if bitfield.signed && width < storage_bits {
                                    let extend = storage_type
                                        .const_int(u64::from(storage_bits - width), false);
                                    let left = self
                                        .builder
                                        .build_left_shift(
                                            masked,
                                            extend,
                                            &format!("ffi_{name}_sign_left"),
                                        )
                                        .map_err(|error| error.to_string())?;
                                    self.builder
                                        .build_right_shift(
                                            left,
                                            extend,
                                            true,
                                            &format!("ffi_{name}_sign_right"),
                                        )
                                        .map_err(|error| error.to_string())?
                                } else {
                                    masked
                                };
                                if bitfield.signed {
                                    self.builder.build_signed_int_to_float(
                                        integer,
                                        self.context.f64_type(),
                                        &format!("ffi_{name}_number"),
                                    )
                                } else {
                                    self.builder.build_unsigned_int_to_float(
                                        integer,
                                        self.context.f64_type(),
                                        &format!("ffi_{name}_number"),
                                    )
                                }
                                .map_err(|error| error.to_string())?
                                .into()
                            }
                            _ => unreachable!("HIR validates bitfield field types"),
                        };
                    } else if explicit_field.is_some() && *field_ty == HirType::Bool {
                        let native_bool = field.into_int_value();
                        field = self
                            .builder
                            .build_int_compare(
                                IntPredicate::NE,
                                native_bool,
                                native_bool.get_type().const_zero(),
                                &format!("ffi_{name}_bool"),
                            )
                            .map_err(|error| error.to_string())?
                            .into();
                    }
                    field = match field_ty {
                        HirType::Str if *ownership != FfiOwnership::Borrowed => self
                            .apply_ffi_string_ownership(
                                field.into_pointer_value(),
                                ownership,
                                &format!("ffi_{name}"),
                            )?
                            .into(),
                        HirType::Object(_) | HirType::Array(_) | HirType::Tuple(_)
                        | HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_)
                            if field.is_struct_value() => self
                            .marshal_ffi_return(
                            field,
                            field_ty,
                            FfiStringAbi::NullTerminated,
                            ownership,
                            aggregate_abi,
                            aggregate_layout
                                .and_then(|layout| layout.field_layouts[index].as_deref()),
                        )?,
                        _ => field,
                    };
                    let slot = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i8_type(),
                                result,
                                &[i64_type.const_int(object_field_offset(fields, index)?, false)],
                                "ffi_object_field",
                            )
                            .map_err(|error| error.to_string())?
                    };
                    self.builder
                        .build_store(slot, field)
                        .map_err(|error| error.to_string())?;
                }
                Ok(result.into())
            }
            _ => Ok(value),
        }
    }

    fn unpack_ffi_register_return(
        &mut self,
        value: BasicValueEnum<'ctx>,
        sig: &FfiSignature,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let layout = sig
            .aggregate_return_layout
            .as_ref()
            .expect("register return has an explicit layout");
        let native_type = self.ffi_return_type(
            &sig.ret,
            sig.return_string_abi,
            sig.aggregate_return_abi,
            Some(layout),
        )?;
        let slot = self
            .builder
            .build_alloca(native_type, "ffi_register_result_storage")
            .map_err(|error| error.to_string())?;
        slot.as_instruction_value()
            .expect("alloca is an instruction")
            .set_alignment(layout.alignment)
            .map_err(|error| error.to_string())?;
        let registers = value.into_struct_value();
        for (index, _) in layout.register_classes.iter().enumerate() {
            let register = self
                .builder
                .build_extract_value(registers, index as u32, "ffi_result_register")
                .map_err(|error| error.to_string())?;
            let target = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        slot,
                        &[self.context.i64_type().const_int((index * 8) as u64, false)],
                        "ffi_result_register_slot",
                    )
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(target, register)
                .map_err(|error| error.to_string())?;
        }
        self.builder
            .build_load(native_type, slot, "ffi_register_result_value")
            .map_err(|error| error.to_string())
    }

    /// `HirExpr::FfiCall` -- an ambient `declare function` call (see
    /// docs/design/bridge.md). By the time codegen sees this, the symbol
    /// is already declared (`declare_extern_function` ran in the
    /// `compile_program` pre-pass) with the *adapted* C ABI parameter list
    /// `ffi_param_types` computed, so unlike a normal Thaw-to-Thaw call
    /// (`build_call_with`), an `Array`/`Object` argument here must be
    /// unpacked into that same adapted shape before the call -- each match
    /// arm below has a matching arm in `ffi_param_types`'s doc comment.
    fn pack_ffi_bool_array(
        &mut self,
        base: PointerValue<'ctx>,
        target_type: inkwell::types::IntType<'ctx>,
        target_bytes: u64,
    ) -> Result<(PointerValue<'ctx>, IntValue<'ctx>), String> {
        let i64_type = self.context.i64_type();
        let length = self
            .builder
            .build_load(i64_type, base, "ffi_bool_array_length")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let byte_count = self
            .builder
            .build_int_mul(
                length,
                i64_type.const_int(target_bytes, false),
                "ffi_bool_array_byte_count",
            )
            .map_err(|error| error.to_string())?;
        let empty = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                length,
                i64_type.const_zero(),
                "ffi_bool_array_empty",
            )
            .map_err(|error| error.to_string())?;
        let allocation_size = self
            .builder
            .build_select(
                empty,
                i64_type.const_int(target_bytes, false),
                byte_count,
                "ffi_bool_array_allocation_size",
            )
            .map_err(|error| error.to_string())?;
        let packed = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    allocation_size.into(),
                    i64_type.const_int(target_bytes.min(8), false).into(),
                ],
                "ffi_bool_array_alloc",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc returned no boolean-array pointer")?
            .into_pointer_value();
        let function = self.current_function();
        let entry = self
            .builder
            .get_insert_block()
            .ok_or("boolean-array marshalling has no current block")?;
        let condition = self.context.append_basic_block(function, "ffi_bool_array_next");
        let body = self.context.append_basic_block(function, "ffi_bool_array_body");
        let done = self.context.append_basic_block(function, "ffi_bool_array_done");
        self.builder
            .build_unconditional_branch(condition)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(condition);
        let index = self
            .builder
            .build_phi(i64_type, "ffi_bool_array_index")
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&i64_type.const_zero(), entry)]);
        let current = index.as_basic_value().into_int_value();
        let has_element = self
            .builder
            .build_int_compare(IntPredicate::ULT, current, length, "ffi_bool_array_has_element")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(has_element, body, done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(body);
        let source_offset = self
            .builder
            .build_int_mul(
                current,
                i64_type.const_int(ARRAY_ELEM_BYTES, false),
                "ffi_bool_array_source_offset",
            )
            .map_err(|error| error.to_string())?;
        let source_offset = self
            .builder
            .build_int_add(
                source_offset,
                i64_type.const_int(ARRAY_HEADER_BYTES, false),
                "ffi_bool_array_source_data_offset",
            )
            .map_err(|error| error.to_string())?;
        let source = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    base,
                    &[source_offset],
                    "ffi_bool_array_source",
                )
                .map_err(|error| error.to_string())?
        };
        let boolean = self
            .builder
            .build_load(self.context.bool_type(), source, "ffi_bool_array_value")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let promoted = self
            .builder
            .build_int_z_extend(boolean, target_type, "ffi_bool_array_promoted")
            .map_err(|error| error.to_string())?;
        let target_offset = self
            .builder
            .build_int_mul(
                current,
                i64_type.const_int(target_bytes, false),
                "ffi_bool_array_target_offset",
            )
            .map_err(|error| error.to_string())?;
        let target = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    packed,
                    &[target_offset],
                    "ffi_bool_array_target",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(target, promoted)
            .map_err(|error| error.to_string())?;
        let next = self
            .builder
            .build_int_add(current, i64_type.const_int(1, false), "ffi_bool_array_increment")
            .map_err(|error| error.to_string())?;
        let body_end = self
            .builder
            .get_insert_block()
            .ok_or("boolean-array marshalling lost its body block")?;
        self.builder
            .build_unconditional_branch(condition)
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&next, body_end)]);
        self.builder.position_at_end(done);
        Ok((packed, length))
    }

    fn append_ffi_fixed_argument(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &HirType,
        output: &mut Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> Result<(), String> {
        match ty {
            HirType::Array(element) if element.as_ref() == &HirType::Bool => {
                let handle = value.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let (data, length) =
                    self.pack_ffi_bool_array(buffer, self.context.i8_type(), 1)?;
                output.push(data.into());
                output.push(length.into());
            }
            HirType::Array(element)
                if matches!(element.as_ref(), HirType::F64 | HirType::Str | HirType::JsValue) =>
            {
                let handle = value.into_pointer_value();
                let base = self.compile_array_data(handle)?;
                let i64_type = self.context.i64_type();
                let length = self
                    .builder
                    .build_load(i64_type, base, "ffi_array_length")
                    .map_err(|error| error.to_string())?;
                let data = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            base,
                            &[i64_type.const_int(ARRAY_HEADER_BYTES, false)],
                            "ffi_array_data",
                        )
                        .map_err(|error| error.to_string())?
                };
                output.push(data.into());
                output.push(length.into());
            }
            HirType::Tuple(elements) => {
                let handle = value.into_pointer_value();
                let base = self.compile_array_data(handle)?;
                let i64_type = self.context.i64_type();
                let element_bytes = tuple_element_storage_bytes(elements)?;
                for (index, element) in elements.iter().enumerate() {
                    let offset = i64_type.const_int(
                        checked_array_offset(element_bytes, index)?,
                        false,
                    );
                    let pointer = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i8_type(),
                                base,
                                &[offset],
                                "ffi_tuple_element_pointer",
                            )
                            .map_err(|error| error.to_string())?
                    };
                    let element_value = self
                        .builder
                        .build_load(
                            self.basic_type(element).map_err(|error| {
                                format!("FFI tuple element {index}: {error}")
                            })?,
                            pointer,
                            "ffi_tuple_element",
                        )
                        .map_err(|error| error.to_string())?;
                    self.append_ffi_fixed_argument(element_value, element, output)
                        .map_err(|error| format!("FFI tuple element {index}: {error}"))?;
                }
            }
            HirType::Object(fields) => {
                let base = value.into_pointer_value();
                let i64_type = self.context.i64_type();
                for (index, (name, field_ty)) in fields.iter().enumerate() {
                    let pointer = self.compile_field_ptr_from_pointer(base, fields, index)?;
                    let field = self
                        .builder
                        .build_load(
                            self.basic_type(field_ty)
                                .map_err(|error| format!("FFI object field `{name}`: {error}"))?,
                            pointer,
                            "ffi_object_field",
                        )
                        .map_err(|error| error.to_string())?;
                    self.append_ffi_fixed_argument(field, field_ty, output)
                        .map_err(|error| format!("FFI object field `{name}`: {error}"))?;
                }
            }
            HirType::Optional(payload) | HirType::Nullable(payload) => {
                let tagged = value.into_struct_value();
                let tag = self
                    .builder
                    .build_extract_value(tagged, 0, "ffi_optional_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let tag = self
                    .builder
                    .build_int_z_extend(tag, self.context.i8_type(), "ffi_optional_tag_i8")
                    .map_err(|error| error.to_string())?;
                output.push(tag.into());
                let payload_value = self
                    .builder
                    .build_extract_value(tagged, 1, "ffi_optional_payload")
                    .map_err(|error| error.to_string())?;
                self.append_ffi_fixed_argument(payload_value, payload, output)?;
            }
            HirType::Nullish(payload) => {
                let tagged = value.into_struct_value();
                let tag = self
                    .builder
                    .build_extract_value(tagged, 0, "ffi_nullish_tag")
                    .map_err(|error| error.to_string())?;
                output.push(tag.into());
                let payload_value = self
                    .builder
                    .build_extract_value(tagged, 1, "ffi_nullish_payload")
                    .map_err(|error| error.to_string())?;
                self.append_ffi_fixed_argument(payload_value, payload, output)?;
            }
            _ => output.push(value.into()),
        }
        Ok(())
    }

    fn append_ffi_aggregate_vararg(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &HirType,
        output: &mut Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> Result<(), String> {
        match ty {
            HirType::F64 | HirType::Str | HirType::JsValue => output.push(value.into()),
            HirType::Bool => {
                let promoted = self
                    .builder
                    .build_int_z_extend(
                        value.into_int_value(),
                        self.context.i32_type(),
                        "ffi_aggregate_vararg_bool",
                    )
                    .map_err(|error| error.to_string())?;
                output.push(promoted.into());
            }
            HirType::Array(element)
                if matches!(
                    element.as_ref(),
                    HirType::F64 | HirType::Str | HirType::JsValue
                ) =>
            {
                let handle = value.into_pointer_value();
                let base = self.compile_array_data(handle)?;
                let i64_type = self.context.i64_type();
                let length = self
                    .builder
                    .build_load(i64_type, base, "ffi_aggregate_vararg_array_length")
                    .map_err(|error| error.to_string())?;
                let data = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            base,
                            &[i64_type.const_int(ARRAY_HEADER_BYTES, false)],
                            "ffi_aggregate_vararg_array_data",
                        )
                        .map_err(|error| error.to_string())?
                };
                output.push(data.into());
                output.push(length.into());
            }
            HirType::Array(element) if element.as_ref() == &HirType::Bool => {
                let handle = value.into_pointer_value();
                let base = self.compile_array_data(handle)?;
                let (packed, length) =
                    self.pack_ffi_bool_array(base, self.context.i32_type(), 4)?;
                output.push(packed.into());
                output.push(length.into());
            }
            HirType::Optional(payload) | HirType::Nullable(payload) => {
                let tagged = value.into_struct_value();
                let tag = self
                    .builder
                    .build_extract_value(tagged, 0, "ffi_aggregate_vararg_optional_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let promoted = self
                    .builder
                    .build_int_z_extend(
                        tag,
                        self.context.i32_type(),
                        "ffi_aggregate_vararg_optional_tag_i32",
                    )
                    .map_err(|error| error.to_string())?;
                output.push(promoted.into());
                let value = self
                    .builder
                    .build_extract_value(tagged, 1, "ffi_aggregate_vararg_optional_value")
                    .map_err(|error| error.to_string())?;
                self.append_ffi_aggregate_vararg(value, payload, output)?;
            }
            HirType::Nullish(payload) => {
                let tagged = value.into_struct_value();
                let tag = self
                    .builder
                    .build_extract_value(tagged, 0, "ffi_aggregate_vararg_nullish_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let promoted = self
                    .builder
                    .build_int_z_extend(
                        tag,
                        self.context.i32_type(),
                        "ffi_aggregate_vararg_nullish_tag_i32",
                    )
                    .map_err(|error| error.to_string())?;
                output.push(promoted.into());
                let value = self
                    .builder
                    .build_extract_value(tagged, 1, "ffi_aggregate_vararg_nullish_value")
                    .map_err(|error| error.to_string())?;
                self.append_ffi_aggregate_vararg(value, payload, output)?;
            }
            HirType::Object(fields) => {
                let base = value.into_pointer_value();
                for (index, (_, field_type)) in fields.iter().enumerate() {
                    let field_pointer = self.compile_field_ptr_from_pointer(base, fields, index)?;
                    let field = self
                        .builder
                        .build_load(
                            self.basic_type(field_type)?,
                            field_pointer,
                            "ffi_aggregate_vararg_value",
                        )
                        .map_err(|error| error.to_string())?;
                    self.append_ffi_aggregate_vararg(field, field_type, output)?;
                }
            }
            other => return Err(format!("unsupported aggregate variadic type {other:?}")),
        }
        Ok(())
    }

    fn guard_ffi_variadic_integer(
        &mut self,
        value: FloatValue<'ctx>,
        abi: FfiVariadicAbi,
    ) -> Result<FloatValue<'ctx>, String> {
        let (lower, upper) = match abi {
            FfiVariadicAbi::I32 => (-(2.0_f64).powi(31), (2.0_f64).powi(31)),
            FfiVariadicAbi::I64 => (-(2.0_f64).powi(63), (2.0_f64).powi(63)),
            FfiVariadicAbi::U32 => (0.0, (2.0_f64).powi(32)),
            FfiVariadicAbi::U64 => (0.0, (2.0_f64).powi(64)),
            FfiVariadicAbi::Native => return Ok(value),
        };
        let truncated = self.builder.build_call(
            self.module.get_function("llvm.trunc.f64").unwrap(),
            &[value.into()],
            "ffi_vararg_truncated",
        ).map_err(|error| error.to_string())?
        .try_as_basic_value().basic().ok_or("llvm.trunc returned no FFI variadic value")?
        .into_float_value();
        let f64_type = self.context.f64_type();
        let above_lower = self.builder.build_float_compare(
            FloatPredicate::OGE, truncated, f64_type.const_float(lower), "ffi_vararg_above_lower",
        ).map_err(|error| error.to_string())?;
        let below_upper = self.builder.build_float_compare(
            FloatPredicate::OLT, truncated, f64_type.const_float(upper), "ffi_vararg_below_upper",
        ).map_err(|error| error.to_string())?;
        let valid = self.builder.build_and(above_lower, below_upper, "ffi_vararg_in_range")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let error_bb = self.context.append_basic_block(function, "ffi_vararg_range_error");
        let valid_bb = self.context.append_basic_block(function, "ffi_vararg_valid");
        self.builder.build_conditional_branch(valid, valid_bb, error_bb)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(error_bb);
        let error = self.builder.build_global_string_ptr(
            "\u{1}RangeError\u{1}FFI variadic integer out of range",
            "ffi_vararg_range_message",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), error.as_pointer_value())
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error.as_pointer_value())?;
        self.branch_on_pending_exception()?;
        self.builder.build_unconditional_branch(valid_bb)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(valid_bb);
        Ok(truncated)
    }

    /// This is a producer proof, not a Promise type test: these HIR forms
    /// return the `thaw_promise_new` creator allocated in the same callback
    /// invocation. A `Var`, call, or conditional may return a borrowed alias.
    fn http_fresh_promise_callback(&self, callback: &HirExpr) -> Option<(Vec<HirType>, HirType, u64)> {
        match callback {
            HirExpr::FunctionRef(name, params, ret)
                if self.frame_async_functions.contains_key(name)
                    && matches!(ret, HirType::Promise(_)) =>
            {
                Some((params.clone(), ret.clone(), 1))
            }
            HirExpr::Lambda(_, params, ret, body)
                if matches!(ret, HirType::Promise(_))
                    && Self::http_fresh_promise_body(body) =>
            {
                Some((params.iter().map(|param| param.ty.clone()).collect(), ret.clone(), 4))
            }
            _ => None,
        }
    }

    fn http_fresh_promise_body(body: &HirExpr) -> bool {
        match body {
            HirExpr::PromiseNew(..) => true,
            HirExpr::Conditional(_, yes, no, _) =>
                Self::http_fresh_promise_body(yes)
                    && Self::http_fresh_promise_body(no),
            HirExpr::Block(stmts) => match stmts.as_slice() {
                [HirStmt::Let(name, HirType::Promise(_), HirExpr::PromiseNew(..)),
                 HirStmt::Return(Some(HirExpr::Var(returned)))] => name == returned,
                [.., HirStmt::Return(Some(value))]
                    if stmts[..stmts.len() - 1].iter().all(|stmt|
                        matches!(stmt, HirStmt::Let(..) | HirStmt::Expr(_))) =>
                {
                    Self::http_fresh_promise_body(value)
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// The frame ramp constructs a fresh completion for each invocation.
    /// Only that proven producer can transfer a creator share through the
    /// private HTTP out-parameter ABI. Other Promise callbacks still need
    /// per-return ownership tagging and cannot be relabeled by type alone.
    fn compile_http_frame_callback_adapter(
        &mut self,
        original: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
        ownership_tag: u64,
    ) -> Result<PointerValue<'ctx>, String> {
        let HirType::Promise(resolved) = ret else {
            return Err("HTTP frame callback must return Promise".into());
        };
        if params.len() != 2 {
            return Err("HTTP frame callback must accept request and response and return Promise".into());
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        if self.module.get_global(HTTP_SAVED_EXCEPTION_ROOT_SYMBOL).is_none() {
            let root = self.module.add_global(ptr, None, HTTP_SAVED_EXCEPTION_ROOT_SYMBOL);
            root.set_initializer(&ptr.const_null());
            self.module_exception_roots.push(root.as_pointer_value());
        }
        self.tracks_owned_json_roots = true;
        let callback_type = self.context.void_type().fn_type(
            &[ptr.into(), ptr.into(), ptr.into(), ptr.into()], false,
        );
        let name = format!("__thaw_http_frame_callback_{}", self.next_lambda);
        self.next_lambda += 1;
        let adapter = self.module.add_function(&name, callback_type, Some(Linkage::Internal));
        let parent = self.builder.get_insert_block().ok_or("HTTP adapter has no parent block")?;
        // A generated callback is a separate function. A rejected-Promise
        // registration failure must not branch into its creator's catch or
        // settle its creator's async completion.
        let outer_catch_stack = std::mem::take(&mut self.catch_stack);
        let outer_catch_owner_roots = std::mem::take(&mut self.catch_owner_roots);
        let outer_async_completion = self.active_async_completion.take();
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        // Isolate all nine pending fields, including the explicit Json Box
        // owner, before invoking user code.
        let output = adapter.get_nth_param(3).unwrap().into_pointer_value();
        let (previous, snapshot_cell) = self.snapshot_and_clear_pending_exception_tuple(output)?;
        let environment = adapter.get_nth_param(0).unwrap().into_pointer_value();
        let stored_original = unsafe { self.builder.build_in_bounds_gep(
            self.context.i8_type(), environment,
            &[self.context.i64_type().const_int(8, false)], "http_original_slot",
        ).map_err(|error| error.to_string())? };
        let original_closure = self.builder.build_load(ptr, stored_original, "http_original")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let original_code = self.builder.build_load(ptr, original_closure, "http_original_code")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let request = adapter.get_nth_param(1).unwrap();
        let response = adapter.get_nth_param(2).unwrap();
        let original_type = self.function_type(params, ret)?;
        let called = self.builder.build_indirect_call(
            original_type, original_code,
            &[original_closure.into(), request.into(), response.into()],
            "http_frame_result",
        ).map_err(|error| error.to_string())?
            .try_as_basic_value().basic()
            .ok_or("HTTP frame callback returned no Promise pointer")?;
        let own_error = self.builder.build_load(ptr, self.pending_exception().as_pointer_value(),
            "http_callback_pending_error").map_err(|error| error.to_string())?.into_pointer_value();
        let has_error = self.builder.build_is_not_null(own_error, "http_callback_raised")
            .map_err(|error| error.to_string())?;
        let failed = self.context.append_basic_block(adapter, "http_callback_failed");
        let succeeded = self.context.append_basic_block(adapter, "http_callback_succeeded");
        let publish = self.context.append_basic_block(adapter, "http_callback_publish");
        self.builder.build_conditional_branch(has_error, failed, succeeded)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failed);
        // The producer's implicit exception return is null. Do not release a
        // speculative non-null return: the producer retains its own shares.
        // This peer helper registers the physical finisher and rejects from
        // the complete active pending tuple, or returns null on failure.
        let rejected = self.compile_rejected_callback_promise(
            resolved, own_error, false, "http_callback_failure",
        )?;
        // The rejected Promise either took an independent Json share or the
        // helper returned null after cleaning its unsuccessful Promise. This
        // callback does not publish its own pending tuple in either case.
        // Clear every field before destroying the original Box so release
        // callbacks cannot observe stale pending ownership.
        self.discard_isolated_pending_exception_tuple(snapshot_cell)?;
        self.restore_pending_exception_tuple(&previous, snapshot_cell)?;
        let rejected_done = self.builder.get_insert_block().unwrap();
        self.builder.build_unconditional_branch(publish).map_err(|error| error.to_string())?;
        self.builder.position_at_end(succeeded);
        self.restore_pending_exception_tuple(&previous, snapshot_cell)?;
        let succeeded_done = self.builder.get_insert_block().unwrap();
        self.builder.build_unconditional_branch(publish).map_err(|error| error.to_string())?;
        self.builder.position_at_end(publish);
        let returned_value = self.builder.build_phi(ptr, "http_callback_result")
            .map_err(|error| error.to_string())?;
        returned_value.add_incoming(&[
            (&rejected, rejected_done),
            (&called, succeeded_done),
        ]);
        // The synthetic rejection is a settled, newly created Promise even
        // when the original producer was a direct PromiseNew (tag 4). It has
        // no escaped executor and follows the ordinary fresh-creator lane.
        let source_tag = self.builder.build_phi(self.context.i8_type(), "http_callback_source_tag")
            .map_err(|error| error.to_string())?;
        source_tag.add_incoming(&[
            (&self.context.i8_type().const_int(1, false), rejected_done),
            (&self.context.i8_type().const_int(ownership_tag, false), succeeded_done),
        ]);
        let returned = returned_value.as_basic_value().into_pointer_value();
        self.builder.build_store(output, returned)
            .map_err(|error| error.to_string())?;
        let ownership = unsafe { self.builder.build_in_bounds_gep(
            self.context.i8_type(), output,
            &[self.context.i64_type().const_int(8, false)], "http_ownership_slot",
        ).map_err(|error| error.to_string())? };
        let missing = self.builder.build_is_null(returned, "http_callback_no_promise")
            .map_err(|error| error.to_string())?;
        let tag = self.builder.build_select(missing,
            self.context.i8_type().const_int(3, false),
            source_tag.as_basic_value().into_int_value(), "http_callback_ownership")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(ownership, tag)
            .map_err(|error| error.to_string())?;
        self.builder.build_return(None).map_err(|error| error.to_string())?;
        self.builder.position_at_end(parent);
        self.catch_stack = outer_catch_stack;
        self.catch_owner_roots = outer_catch_owner_roots.clone();
        self.active_async_completion = outer_async_completion;

        let closure = self.builder.build_call(
            self.module.get_function("thaw_arena_alloc").unwrap(),
            &[self.context.i64_type().const_int(16, false).into(),
              self.context.i64_type().const_int(8, false).into()],
            "http_adapter_closure",
        ).map_err(|error| error.to_string())?
            .try_as_basic_value().basic()
            .ok_or("HTTP adapter allocation returned no value")?.into_pointer_value();
        let current = self.current_function();
        let failed = self.context.append_basic_block(current, "http_adapter_allocation_failed");
        let ready = self.context.append_basic_block(current, "http_adapter_allocation_ready");
        let missing = self.builder.build_is_null(closure, "http_adapter_missing")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(missing, failed, ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failed);
        self.compile_throw_builtin_error("Error", "HTTP callback adapter allocation failed")?;
        self.builder.position_at_end(ready);
        self.builder.build_store(closure, adapter.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let original_slot = unsafe { self.builder.build_in_bounds_gep(
            self.context.i8_type(), closure,
            &[self.context.i64_type().const_int(8, false)], "http_adapter_original_slot",
        ).map_err(|error| error.to_string())? };
        self.builder.build_store(original_slot, original)
            .map_err(|error| error.to_string())?;
        Ok(closure)
    }

    fn compile_ffi_call(
        &mut self,
        sig: &FfiSignature,
        args: &[HirExpr],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let sig = self
            .ffi_signatures
            .get(&sig.symbol)
            .cloned()
            .unwrap_or_else(|| sig.clone());
        let sig = &sig;
        let function = match self.module.get_function(&sig.symbol) {
            Some(function) => function,
            None if matches!(sig.symbol.as_str(),
                "__thaw_http_createServerAsync" | "__thaw_http_createServerOnceAsync") => {
                // HIR selects this compiler-private sibling after the
                // ambient declaration pre-pass. Its pointer ABI is otherwise
                // identical to that declaration, and Rust receives the
                // separately adapted callback value below.
                self.declare_extern_function(sig)?
            }
            None => return Err(format!("FFI function `{}` was not declared", sig.symbol)),
        };

        // Evaluate every source argument before ABI conversion can reject one.
        let values = args.iter().map(|arg| self.compile_expr(arg)).collect::<Result<Vec<_>, _>>()?;
        let mut compiled_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len());
        for (index, (param_ty, value)) in sig.params.iter().zip(&values).enumerate() {
            let value = if (sig.symbol == "__thaw_http_createServerAsync" && index == 0)
                || (sig.symbol == "__thaw_http_createServerOnceAsync" && index == 1)
            {
                if let Some((callback_params, callback_ret, ownership_tag)) =
                    self.http_fresh_promise_callback(&args[index])
                {
                    self.compile_http_frame_callback_adapter(
                        (*value).into_pointer_value(), &callback_params, &callback_ret,
                        ownership_tag,
                    )?.into()
                } else {
                    return Err("async HTTP callback return ownership requires a producer-tagged adapter".into());
                }
            } else { *value };
            match param_ty {
                HirType::Str
                    if sig.param_string_abis.get(index) == Some(&FfiStringAbi::PointerLength) =>
                {
                    let pointer = value.into_pointer_value();
                    let strlen = self.module.get_function("thaw_string_byte_length").unwrap();
                    let length = self
                        .builder
                        .build_call(strlen, &[pointer.into()], "ffi_string_length")
                        .map_err(|error| error.to_string())?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    compiled_args.push(pointer.into());
                    compiled_args.push(length.into());
                }
                _ => self.append_ffi_fixed_argument(value, param_ty, &mut compiled_args)?,
            }
        }
        if let Some(variadic) = &sig.variadic {
            for value in &values[sig.params.len()..] {
                let value = *value;
                match variadic {
                    HirType::F64 => {
                        let value = value.into_float_value();
                        let value = self.guard_ffi_variadic_integer(value, sig.variadic_abi)?;
                        let converted: BasicValueEnum<'ctx> = match sig.variadic_abi {
                            FfiVariadicAbi::Native => value.into(),
                            FfiVariadicAbi::I32 => self
                                .builder
                                .build_float_to_signed_int(
                                    value,
                                    self.context.i32_type(),
                                    "ffi_vararg_i32",
                                )
                                .map_err(|error| error.to_string())?
                                .into(),
                            FfiVariadicAbi::I64 => self
                                .builder
                                .build_float_to_signed_int(
                                    value,
                                    self.context.i64_type(),
                                    "ffi_vararg_i64",
                                )
                                .map_err(|error| error.to_string())?
                                .into(),
                            FfiVariadicAbi::U32 => self
                                .builder
                                .build_float_to_unsigned_int(
                                    value,
                                    self.context.i32_type(),
                                    "ffi_vararg_u32",
                                )
                                .map_err(|error| error.to_string())?
                                .into(),
                            FfiVariadicAbi::U64 => self
                                .builder
                                .build_float_to_unsigned_int(
                                    value,
                                    self.context.i64_type(),
                                    "ffi_vararg_u64",
                                )
                                .map_err(|error| error.to_string())?
                                .into(),
                        };
                        compiled_args.push(converted.into());
                    }
                    HirType::Bool => {
                        let promoted = self
                            .builder
                            .build_int_z_extend(
                                value.into_int_value(),
                                self.context.i32_type(),
                                "ffi_vararg_bool",
                            )
                            .map_err(|error| error.to_string())?;
                        compiled_args.push(promoted.into());
                    }
                    HirType::Str | HirType::JsValue => compiled_args.push(value.into()),
                    HirType::Array(_)
                    | HirType::Object(_)
                    | HirType::Optional(_)
                    | HirType::Nullable(_)
                    | HirType::Nullish(_) => self
                        .append_ffi_aggregate_vararg(value, variadic, &mut compiled_args)
                        .map_err(|error| format!("FFI function `{}`: {error}", sig.symbol))?,
                    other => {
                        return Err(format!(
                            "FFI function `{}` has unsupported variadic element type {other:?}",
                            sig.symbol
                        ))
                    }
                }
            }
        }

        let indirect_return = if Self::uses_indirect_ffi_return(sig) {
            let return_type = self
                .ffi_call_return_type(sig)?
                .expect("packed aggregate returns always have a value type");
            let slot = self
                .builder
                .build_alloca(return_type, "ffi_indirect_result")
                .map_err(|error| error.to_string())?;
            if let Some(layout) = &sig.aggregate_return_layout {
                slot.as_instruction_value()
                    .expect("alloca is an instruction")
                    .set_alignment(layout.alignment)
                    .map_err(|error| error.to_string())?;
            }
            compiled_args.insert(0, slot.into());
            Some((slot, return_type))
        } else {
            None
        };
        let call_site = self
            .builder
            .build_call(function, &compiled_args, "ffi_calltmp")
            .map_err(|e| e.to_string())?;
        call_site.set_call_convention(Self::ffi_calling_convention(sig.calling_convention));
        let mut returned = if let Some((slot, return_type)) = indirect_return {
            Some(
                self.builder
                    .build_load(return_type, slot, "ffi_indirect_result_value")
                    .map_err(|error| error.to_string())?,
            )
        } else {
            call_site.try_as_basic_value().basic()
        };
        if sig.error_abi == FfiErrorAbi::Direct
            && sig
                .aggregate_return_layout
                .as_ref()
                .is_some_and(|layout| !layout.register_classes.is_empty())
        {
            returned = returned
                .map(|value| self.unpack_ffi_register_return(value, sig))
                .transpose()?;
        }
        if sig.ret == HirType::Void {
            if sig.error_abi == FfiErrorAbi::Direct {
                return Ok(None);
            }
            let result = returned
                .ok_or_else(|| format!("`{}` did not return its error result", sig.symbol))?
                .into_struct_value();
            let error = self
                .builder
                .build_extract_value(result, 0, "ffi_result_error")
                .map_err(|e| e.to_string())?
                .into_pointer_value();
            let error =
                self.apply_ffi_string_ownership(error, &sig.error_ownership, "ffi_error")?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|e| e.to_string())?;
            self.clear_pending_native_text()?;
            self.mark_pending_native_text(error)?;
            self.branch_on_pending_exception()?;
            return Ok(None);
        }
        let returned =
            returned.ok_or_else(|| format!("`{}` does not return a value", sig.symbol))?;
        if sig.error_abi == FfiErrorAbi::Direct {
            if sig.ret == HirType::Str && sig.return_string_abi == FfiStringAbi::NullTerminated {
                return self
                    .apply_ffi_string_ownership(
                        returned.into_pointer_value(),
                        &sig.return_ownership,
                        "ffi_return",
                    )
                    .map(BasicValueEnum::from)
                    .map(Some);
            }
            return self
                .marshal_ffi_return(
                    returned,
                    &sig.ret,
                    sig.return_string_abi,
                    &sig.return_ownership,
                    sig.aggregate_return_abi,
                    sig.aggregate_return_layout.as_ref(),
                )
                .map(Some);
        }

        let result = returned.into_struct_value();
        let (value_index, error_index) = self.ffi_result_field_indices(sig)?;
        let value = self
            .builder
            .build_extract_value(result, value_index, "ffi_result_value")
            .map_err(|e| e.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, error_index, "ffi_result_error")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        let error = self.apply_ffi_string_ownership(error, &sig.error_ownership, "ffi_error")?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|e| e.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        if sig.ret == HirType::Str && sig.return_string_abi == FfiStringAbi::NullTerminated {
            return self
                .apply_ffi_string_ownership(
                    value.into_pointer_value(),
                    &sig.return_ownership,
                    "ffi_return",
                )
                .map(BasicValueEnum::from)
                .map(Some);
        }
        self.marshal_ffi_return(
            value,
            &sig.ret,
            sig.return_string_abi,
            &sig.return_ownership,
            sig.aggregate_return_abi,
            sig.aggregate_return_layout.as_ref(),
        )
        .map(Some)
    }

}
