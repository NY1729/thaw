impl<'ctx> HirCompiler<'ctx> {
    fn compile_array_lit(&mut self, elems: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let element_bytes = elems
            .iter()
            .filter_map(|element| self.expr_hir_type(element))
            .map(|element| array_element_storage_bytes(&element))
            .max()
            .unwrap_or(ARRAY_ELEM_BYTES);
        let elem_vals = elems
            .iter()
            .map(|e| self.compile_expr(e))
            .collect::<Result<Vec<_>, _>>()?;

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

        Ok(base_ptr.into())
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
        Ok(result.into())
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
            let array = self.compile_expr(part)?.into_pointer_value();
            let length = self
                .builder
                .build_load(i64_type, array, "spread_length")
                .map_err(|error| error.to_string())?
                .into_int_value();
            total = self
                .builder
                .build_int_add(total, length, "spread_total")
                .map_err(|error| error.to_string())?;
            arrays.push((array, length));
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

        let mut destination_offset = i64_type.const_int(ARRAY_HEADER_BYTES, false);
        for (array, length) in arrays {
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
            destination_offset = self
                .builder
                .build_int_add(destination_offset, bytes, "next_spread_destination")
                .map_err(|error| error.to_string())?;
        }
        Ok(result.into())
    }

    /// Computes the address of `array[index]` (past the length header).
    fn compile_element_ptr(
        &mut self,
        array: &HirExpr,
        index: &HirExpr,
    ) -> Result<PointerValue<'ctx>, String> {
        let arr_ptr = self.compile_expr(array)?.into_pointer_value();
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
