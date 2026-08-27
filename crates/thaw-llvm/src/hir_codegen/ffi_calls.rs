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
        let strlen = self.module.get_function("strlen").unwrap();
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
        Ok(result.into())
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
        let mut value = self
            .builder
            .build_extract_value(native, 1, "ffi_tagged_return_payload")
            .map_err(|error| error.to_string())?;
        value = match payload {
            HirType::Str if *ownership != FfiOwnership::Borrowed => self
                .apply_ffi_string_ownership(
                    value.into_pointer_value(),
                    ownership,
                    "ffi_tagged_return_string",
                )?
                .into(),
            HirType::Array(_)
            | HirType::Object(_)
            | HirType::Optional(_)
            | HirType::Nullable(_)
            | HirType::Nullish(_)
                if value.is_struct_value() => self.marshal_ffi_return(
                value,
                payload,
                FfiStringAbi::NullTerminated,
                ownership,
                aggregate_abi,
                None,
            )?,
            _ => value,
        };
        if matches!(ty, HirType::Nullish(_)) {
            return self.build_nullish_tagged_value(value, payload, tag);
        }
        let present = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                tag,
                tag.get_type().const_zero(),
                "ffi_tagged_return_present",
            )
            .map_err(|error| error.to_string())?;
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
                ARRAY_HEADER_BYTES + element_bytes * index as u64,
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
        Ok(tuple.into())
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
                self.unpack_ffi_bool_array(value.into_struct_value())
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
                Ok(result.into())
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
                                .const_int(object_storage_bytes(fields), false)
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
                        HirType::Object(_) | HirType::Array(_) if field.is_struct_value() => self
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
                                &[i64_type.const_int(object_field_offset(fields, index), false)],
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
                let (data, length) = self.pack_ffi_bool_array(
                    value.into_pointer_value(),
                    self.context.i8_type(),
                    1,
                )?;
                output.push(data.into());
                output.push(length.into());
            }
            HirType::Array(element)
                if matches!(element.as_ref(), HirType::F64 | HirType::Str | HirType::JsValue) =>
            {
                let base = value.into_pointer_value();
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
                let base = value.into_pointer_value();
                let i64_type = self.context.i64_type();
                let element_bytes = elements
                    .iter()
                    .map(array_element_storage_bytes)
                    .max()
                    .unwrap_or(ARRAY_ELEM_BYTES);
                for (index, element) in elements.iter().enumerate() {
                    let offset = i64_type.const_int(
                        ARRAY_HEADER_BYTES + element_bytes * index as u64,
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
                    let pointer = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i8_type(),
                                base,
                                &[i64_type.const_int(object_field_offset(fields, index), false)],
                                "ffi_object_field_pointer",
                            )
                            .map_err(|error| error.to_string())?
                    };
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
                let base = value.into_pointer_value();
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
                let base = value.into_pointer_value();
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
                    let field_pointer = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i8_type(),
                                base,
                                &[self
                                    .context
                                    .i64_type()
                                    .const_int(object_field_offset(fields, index), false)],
                                "ffi_aggregate_vararg_field",
                            )
                            .map_err(|error| error.to_string())?
                    };
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
        let function = self
            .module
            .get_function(&sig.symbol)
            .expect("extern function was declared in compile_program's pre-pass");

        let mut compiled_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len());
        for (index, (param_ty, arg)) in sig.params.iter().zip(args).enumerate() {
            let value = self.compile_expr(arg)?;
            match param_ty {
                HirType::Str
                    if sig.param_string_abis.get(index) == Some(&FfiStringAbi::PointerLength) =>
                {
                    let pointer = value.into_pointer_value();
                    let strlen = self.module.get_function("strlen").unwrap();
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
            for arg in &args[sig.params.len()..] {
                let value = self.compile_expr(arg)?;
                match variadic {
                    HirType::F64 => {
                        let value = value.into_float_value();
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
