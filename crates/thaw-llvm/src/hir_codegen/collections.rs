impl<'ctx> HirCompiler<'ctx> {
    /// An `HirType::Array`/`Tuple` value is a pointer to a one-word "handle"
    /// cell holding the *current* raw `[length][elem...]` buffer pointer,
    /// not the buffer itself -- this indirection is what lets `.push()`/
    /// `.pop()`/`.shift()`/`.unshift()`/`.splice()` grow or shrink an array
    /// by replacing the buffer the handle points to, while every alias of
    /// the same array (another variable, a field, a captured closure
    /// value) shares the same handle pointer and so observes the new
    /// buffer on its next read, matching JavaScript's array reference
    /// semantics. Every array/tuple-typed `HirExpr` compiles to a handle;
    /// `compile_array_data` unwraps one to the buffer a specific operation
    /// actually needs to read/write, and `compile_array_wrap` allocates a
    /// fresh handle around a freshly built buffer.
    fn compile_array_wrap_with_presence(
        &mut self,
        buffer: PointerValue<'ctx>,
        presence: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let handle = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[i64_type.const_int(16, false).into(), i64_type.const_int(8, false).into()],
                "array_handle",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return an array handle")?
            .into_pointer_value();
        self.builder
            .build_store(handle, buffer)
            .map_err(|error| error.to_string())?;
        let presence_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    handle,
                    &[i64_type.const_int(8, false)],
                    "array_presence_slot",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(presence_slot, presence)
            .map_err(|error| error.to_string())?;
        Ok(handle)
    }

    fn compile_array_wrap(
        &mut self,
        buffer: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        self.compile_array_wrap_with_presence(
            buffer,
            self.context.ptr_type(AddressSpace::default()).const_null(),
        )
    }

    /// Loads the current raw `[length][elem...]` buffer pointer out of an
    /// array/tuple handle. See `compile_array_wrap`.
    fn compile_array_data(
        &mut self,
        handle: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        self.builder
            .build_load(ptr_type, handle, "array_data")
            .map_err(|error| error.to_string())
            .map(BasicValueEnum::into_pointer_value)
    }

    fn compile_array_has_index(
        &mut self,
        handle: PointerValue<'ctx>,
        index: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    handle,
                    &[i64_type.const_int(8, false)],
                    "array_presence_slot",
                )
                .map_err(|error| error.to_string())?
        };
        let presence = self
            .builder
            .build_load(ptr_type, slot, "array_presence")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let function = self.current_function();
        let dense = self.context.append_basic_block(function, "array_dense");
        let sparse = self.context.append_basic_block(function, "array_sparse");
        let sparse_present = self.context.append_basic_block(function, "array_sparse_present");
        let beyond_mask = self.context.append_basic_block(function, "array_beyond_presence_mask");
        let done = self.context.append_basic_block(function, "array_presence_done");
        let is_dense = self
            .builder
            .build_is_null(presence, "array_is_dense")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(is_dense, dense, sparse)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(dense);
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(sparse);
        let mask_length = self
            .builder
            .build_load(i64_type, presence, "array_presence_length")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let in_mask = self
            .builder
            .build_int_compare(IntPredicate::ULT, index, mask_length, "array_index_in_presence_mask")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(in_mask, sparse_present, beyond_mask)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(sparse_present);
        let element = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    presence,
                    &[index],
                    "array_presence_element",
                )
                .map_err(|error| error.to_string())?
        };
        let element = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    element,
                    &[i64_type.const_int(8, false)],
                    "array_presence_payload",
                )
                .map_err(|error| error.to_string())?
        };
        let present = self
            .builder
            .build_load(self.context.i8_type(), element, "array_element_present")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let present = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                present,
                self.context.i8_type().const_zero(),
                "array_has_index",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(beyond_mask);
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        let phi = self
            .builder
            .build_phi(self.context.bool_type(), "array_index_present")
            .map_err(|error| error.to_string())?;
        let present_by_default = self.context.bool_type().const_int(1, false);
        phi.add_incoming(&[
            (&present_by_default, dense),
            (&present, sparse_present),
            (&present_by_default, beyond_mask),
        ]);
        Ok(phi.as_basic_value().into_int_value())
    }

    /// Like `compile_array_wrap`, but for a runtime call that signals
    /// failure with a null buffer pointer (regex `exec`/`split`/`match`/
    /// `matchAll`, checked afterward via `__thaw_array_is_null`) -- wrapping
    /// unconditionally would turn that null into a handle pointing at a
    /// cell holding null, which is never itself null, breaking that check.
    /// Only wraps when `buffer` is non-null; a null buffer passes through
    /// as a null "handle" so the null check still means what it always did.
    fn compile_array_wrap_nullable(
        &mut self,
        buffer: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        let handle = self.compile_array_wrap(buffer)?;
        let is_null = self
            .builder
            .build_is_null(buffer, "array_wrap_is_null")
            .map_err(|error| error.to_string())?;
        let null_ptr = self.context.ptr_type(AddressSpace::default()).const_null();
        self.builder
            .build_select(is_null, null_ptr, handle, "array_wrap_nullable")
            .map_err(|error| error.to_string())
            .map(|value| value.into_pointer_value())
    }

    fn compile_array_lit(&mut self, elems: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let mut element_bytes = elems
            .iter()
            .filter_map(|element| self.expr_hir_type(element))
            .map(|element| array_element_storage_bytes(&element))
            .max()
            .unwrap_or(ARRAY_ELEM_BYTES);
        let elem_vals = elems
            .iter()
            .map(|e| self.compile_expr(e))
            .collect::<Result<Vec<_>, _>>()?;
        if elem_vals.iter().any(|value| value.is_struct_value()) {
            element_bytes = element_bytes.max(ASYNC_SLOT_BYTES);
        }

        let size = ARRAY_HEADER_BYTES + element_bytes * elem_vals.len() as u64;
        let i64_type = self.context.i64_type();
        let size_val = i64_type.const_int(size, false);
        let align_val = i64_type.const_int(element_bytes.min(8), false);

        let alloc_fn = self.module.get_function("thaw_arena_alloc").unwrap();
        let call = self
            .builder
            .build_call(alloc_fn, &[size_val.into(), align_val.into()], "arr_alloc")
            .map_err(|e| e.to_string())?;
        let base_ptr = call
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return a value")?
            .into_pointer_value();

        self.builder
            .build_store(base_ptr, i64_type.const_int(elem_vals.len() as u64, false))
            .map_err(|e| e.to_string())?;

        for (i, val) in elem_vals.into_iter().enumerate() {
            let offset = i64_type.const_int(ARRAY_HEADER_BYTES + element_bytes * i as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), base_ptr, &[offset], "elem_ptr")
                    .map_err(|e| e.to_string())?
            };
            self.builder
                .build_store(elem_ptr, val)
                .map_err(|e| e.to_string())?;
        }

        let holes = elems
            .iter()
            .map(|element| matches!(element, HirExpr::Lit(HirLit::ArrayHole)))
            .collect::<Vec<_>>();
        if !holes.iter().any(|hole| *hole) {
            return Ok(self.compile_array_wrap(base_ptr)?.into());
        }
        let presence = self
            .builder
            .build_call(
                alloc_fn,
                &[
                    i64_type.const_int(elems.len() as u64 + 8, false).into(),
                    i64_type.const_int(1, false).into(),
                ],
                "array_presence",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return an array presence mask")?
            .into_pointer_value();
        self.builder
            .build_store(presence, i64_type.const_int(elems.len() as u64, false))
            .map_err(|error| error.to_string())?;
        for (index, hole) in holes.into_iter().enumerate() {
            let slot = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        presence,
                        &[i64_type.const_int(index as u64 + 8, false)],
                        "array_presence_element",
                    )
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(
                    slot,
                    self.context.i8_type().const_int((!hole) as u64, false),
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(self
            .compile_array_wrap_with_presence(base_ptr, presence)?
            .into())
    }

    fn compile_array_alloc(
        &mut self,
        length: &HirExpr,
        element: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let length = self.compile_expr(length)?.into_float_value();
        let length = self
            .builder
            .build_float_to_signed_int(length, i64_type, "array_alloc_length")
            .map_err(|error| error.to_string())?;
        let payload_size = self
            .builder
            .build_int_mul(
                length,
                i64_type.const_int(array_element_storage_bytes(element), false),
                "array_alloc_payload_size",
            )
            .map_err(|error| error.to_string())?;
        let allocation_size = self
            .builder
            .build_int_add(
                payload_size,
                i64_type.const_int(ARRAY_HEADER_BYTES, false),
                "array_alloc_size",
            )
            .map_err(|error| error.to_string())?;
        let result = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    allocation_size.into(),
                    i64_type
                        .const_int(array_element_storage_bytes(element).min(8), false)
                        .into(),
                ],
                "array_alloc",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return an array")?
            .into_pointer_value();
        self.builder
            .build_store(result, length)
            .map_err(|error| error.to_string())?;
        Ok(self.compile_array_wrap(result)?.into())
    }

    /// Evaluates each array part once from left to right and copies their
    /// uniform eight-byte element slots into one arena-owned result array.
    fn compile_array_concat(
        &mut self,
        parts: &[HirExpr],
        element: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let mut arrays = Vec::with_capacity(parts.len());
        let mut total = i64_type.const_zero();
        for part in parts {
            let handle = self.compile_expr(part)?.into_pointer_value();
            let array = self.compile_array_data(handle)?;
            let length = self
                .builder
                .build_load(i64_type, array, "spread_length")
                .map_err(|error| error.to_string())?
                .into_int_value();
            total = self
                .builder
                .build_int_add(total, length, "spread_total")
                .map_err(|error| error.to_string())?;
            arrays.push((handle, array, length));
        }

        let element_width = array_element_storage_bytes(element);
        let element_bytes = i64_type.const_int(element_width, false);
        let payload_size = self
            .builder
            .build_int_mul(total, element_bytes, "spread_payload_size")
            .map_err(|error| error.to_string())?;
        let allocation_size = self
            .builder
            .build_int_add(
                payload_size,
                i64_type.const_int(ARRAY_HEADER_BYTES, false),
                "spread_allocation_size",
            )
            .map_err(|error| error.to_string())?;
        let result = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    allocation_size.into(),
                    i64_type.const_int(element_width.min(8), false).into(),
                ],
                "spread_array_alloc",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return a spread array")?
            .into_pointer_value();
        self.builder
            .build_store(result, total)
            .map_err(|error| error.to_string())?;

        // Array spread must retain holes from every source. Keep one mask for
        // the result; dense inputs simply contribute `true` entries.
        let presence = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    self.builder
                        .build_int_add(total, i64_type.const_int(8, false), "spread_presence_size")
                        .map_err(|error| error.to_string())?
                        .into(),
                    i64_type.const_int(1, false).into(),
                ],
                "spread_presence",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return a spread presence mask")?
            .into_pointer_value();
        self.builder
            .build_store(presence, total)
            .map_err(|error| error.to_string())?;

        let mut destination_offset = i64_type.const_int(ARRAY_HEADER_BYTES, false);
        let mut destination_index = i64_type.const_zero();
        for (handle, array, length) in arrays {
            let bytes = self
                .builder
                .build_int_mul(length, element_bytes, "spread_copy_size")
                .map_err(|error| error.to_string())?;
            let source = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        array,
                        &[i64_type.const_int(ARRAY_HEADER_BYTES, false)],
                        "spread_source",
                    )
                    .map_err(|error| error.to_string())?
            };
            let destination = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        result,
                        &[destination_offset],
                        "spread_destination",
                    )
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_call(
                    self.module.get_function("memcpy").unwrap(),
                    &[destination.into(), source.into(), bytes.into()],
                    "copy_spread_part",
                )
                .map_err(|error| error.to_string())?;
            self.compile_array_presence_copy(handle, length, presence, destination_index)?;
            destination_offset = self
                .builder
                .build_int_add(destination_offset, bytes, "next_spread_destination")
                .map_err(|error| error.to_string())?;
            destination_index = self
                .builder
                .build_int_add(destination_index, length, "next_spread_presence_offset")
                .map_err(|error| error.to_string())?;
        }
        Ok(self.compile_array_wrap_with_presence(result, presence)?.into())
    }

    fn compile_array_presence_copy(
        &mut self,
        source: PointerValue<'ctx>,
        length: IntValue<'ctx>,
        destination: PointerValue<'ctx>,
        destination_offset: IntValue<'ctx>,
    ) -> Result<(), String> {
        let i64_type = self.context.i64_type();
        let function = self.current_function();
        let entry = self.builder.get_insert_block().ok_or("array spread has no block")?;
        let condition = self.context.append_basic_block(function, "spread_presence_next");
        let body = self.context.append_basic_block(function, "spread_presence_element");
        let done = self.context.append_basic_block(function, "spread_presence_done");
        self.builder.build_unconditional_branch(condition).map_err(|e| e.to_string())?;
        self.builder.position_at_end(condition);
        let index = self.builder.build_phi(i64_type, "spread_presence_index").map_err(|e| e.to_string())?;
        index.add_incoming(&[(&i64_type.const_zero(), entry)]);
        let current = index.as_basic_value().into_int_value();
        let more = self.builder.build_int_compare(IntPredicate::ULT, current, length, "spread_presence_more").map_err(|e| e.to_string())?;
        self.builder.build_conditional_branch(more, body, done).map_err(|e| e.to_string())?;
        self.builder.position_at_end(body);
        let present = self.compile_array_has_index(source, current)?;
        let target_index = self.builder.build_int_add(destination_offset, current, "spread_presence_target").map_err(|e| e.to_string())?;
        let target = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), destination, &[self.builder.build_int_add(target_index, i64_type.const_int(8, false), "spread_presence_payload").map_err(|e| e.to_string())?], "spread_presence_slot").map_err(|e| e.to_string())? };
        let present = self.builder.build_int_z_extend(present, self.context.i8_type(), "spread_presence_byte").map_err(|e| e.to_string())?;
        self.builder.build_store(target, present).map_err(|e| e.to_string())?;
        let next = self.builder.build_int_add(current, i64_type.const_int(1, false), "spread_presence_increment").map_err(|e| e.to_string())?;
        let body_end = self.builder.get_insert_block().ok_or("array spread lost its body block")?;
        self.builder.build_unconditional_branch(condition).map_err(|e| e.to_string())?;
        index.add_incoming(&[(&next, body_end)]);
        self.builder.position_at_end(done);
        Ok(())
    }

    /// Computes the address of `array[index]` (past the length header).
    fn compile_element_ptr(
        &mut self,
        array: &HirExpr,
        index: &HirExpr,
    ) -> Result<PointerValue<'ctx>, String> {
        let handle = self.compile_expr(array)?.into_pointer_value();
        let arr_ptr = self.compile_array_data(handle)?;
        let idx_val = self.compile_expr(index)?.into_float_value();

        let i64_type = self.context.i64_type();
        let idx_int = self
            .builder
            .build_float_to_signed_int(idx_val, i64_type, "idx")
            .map_err(|e| e.to_string())?;
        let element_bytes = match self.expr_hir_type(array) {
            Some(HirType::Array(element)) => array_element_storage_bytes(&element),
            Some(HirType::Tuple(elements)) => elements
                .iter()
                .map(array_element_storage_bytes)
                .max()
                .unwrap_or(ARRAY_ELEM_BYTES),
            _ => ARRAY_ELEM_BYTES,
        };
        let elem_size = i64_type.const_int(element_bytes, false);
        let byte_offset = self
            .builder
            .build_int_mul(idx_int, elem_size, "byteoff")
            .map_err(|e| e.to_string())?;
        let byte_offset = self
            .builder
            .build_int_add(
                byte_offset,
                i64_type.const_int(ARRAY_HEADER_BYTES, false),
                "byteoff_hdr",
            )
            .map_err(|e| e.to_string())?;

        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), arr_ptr, &[byte_offset], "elem_ptr")
                .map_err(|e| e.to_string())
        }
    }

    /// Allocates native fields from the arena in the order `fields` lists
    /// them (thaw-hir's lowering already reordered
    /// the literal to match its declared type, so this order is always the
    /// declared one, not whatever order the user happened to write). No
    /// length header: field count/order is static, part of the type, so
    /// unlike arrays there's nothing to record at runtime.
    fn compile_object_lit(
        &mut self,
        fields: &[(String, HirExpr)],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let field_vals = fields
            .iter()
            .map(|(_, expr)| self.compile_expr(expr))
            .collect::<Result<Vec<_>, _>>()?;

        let field_sizes = field_vals
            .iter()
            .map(|value| {
                if value.is_struct_value() {
                    ASYNC_SLOT_BYTES
                } else {
                    OBJECT_FIELD_BYTES
                }
            })
            .collect::<Vec<_>>();
        let size = field_sizes.iter().sum::<u64>();
        let i64_type = self.context.i64_type();
        let size_val = i64_type.const_int(size.max(1), false);
        let align_val = i64_type.const_int(OBJECT_FIELD_BYTES, false);

        let alloc_fn = self.module.get_function("thaw_arena_alloc").unwrap();
        let call = self
            .builder
            .build_call(alloc_fn, &[size_val.into(), align_val.into()], "obj_alloc")
            .map_err(|e| e.to_string())?;
        let base_ptr = call
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return a value")?
            .into_pointer_value();

        let mut byte_offset = 0;
        for (i, val) in field_vals.into_iter().enumerate() {
            let offset = i64_type.const_int(byte_offset, false);
            let field_ptr = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), base_ptr, &[offset], "field_ptr")
                    .map_err(|e| e.to_string())?
            };
            self.builder
                .build_store(field_ptr, val)
                .map_err(|e| e.to_string())?;
            byte_offset += field_sizes[i];
        }

        Ok(base_ptr.into())
    }

    fn compile_object_alloc(
        &mut self,
        object_type: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirType::Object(fields) = object_type else {
            return Err(format!(
                "object allocation requires an object type, got {object_type:?}"
            ));
        };
        let size = fields
            .iter()
            .map(|(_, field_type)| object_field_storage_bytes(field_type))
            .sum::<u64>()
            .max(1);
        let i64_type = self.context.i64_type();
        let allocation = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    i64_type.const_int(size, false).into(),
                    i64_type.const_int(OBJECT_FIELD_BYTES, false).into(),
                ],
                "object_alloc",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return an object allocation")?
            .into_pointer_value();
        let mut byte_offset = 0u64;
        for (_, field_type) in fields {
            let field_pointer = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        allocation,
                        &[i64_type.const_int(byte_offset, false)],
                        "object_zero_field",
                    )
                    .map_err(|error| error.to_string())?
            };
            let zero = self.compile_zero_value(field_type)?;
            self.builder
                .build_store(field_pointer, zero)
                .map_err(|error| error.to_string())?;
            byte_offset += object_field_storage_bytes(field_type);
        }
        Ok(allocation.into())
    }

    /// Looks up `field`'s declared type within `object_ty`, so a read
    /// knows whether to load an `f64`, a pointer (nested object/array/
    /// string/json), etc. -- fields are no longer assumed to all be `f64`
    /// now that nested objects are supported.
    fn field_type(&self, object_ty: &HirType, field: &str) -> Result<HirType, String> {
        let HirType::Object(fields) = object_ty else {
            return Err(format!(
                "`.{field}` used on a non-object type {object_ty:?}"
            ));
        };
        fields
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, ty)| ty.clone())
            .ok_or_else(|| format!("object has no field `{field}`"))
    }

    /// Computes the address of `object.field`, from `object_ty`'s
    /// (statically known, per `HirExpr::PropAccess`'s payload) field order.
    fn compile_field_ptr(
        &mut self,
        object: &HirExpr,
        object_ty: &HirType,
        field: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let HirType::Object(fields) = object_ty else {
            return Err(format!(
                "`.{field}` used on a non-object type {object_ty:?}"
            ));
        };
        let index = fields
            .iter()
            .position(|(name, _)| name == field)
            .ok_or_else(|| format!("object has no field `{field}`"))?;

        let obj_ptr = self.compile_expr(object)?.into_pointer_value();
        let offset = self
            .context
            .i64_type()
            .const_int(object_field_offset(fields, index), false);

        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), obj_ptr, &[offset], "field_ptr")
                .map_err(|e| e.to_string())
        }
    }

}
