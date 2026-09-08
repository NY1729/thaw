impl<'ctx> HirCompiler<'ctx> {
    fn compile_array_named_call(
        &mut self,
        name: &str,
        args: &[HirExpr],
    ) -> Option<Result<BasicValueEnum<'ctx>, String>> {
        let typed_array_call = name.starts_with("__thaw_number_array_")
            || name.starts_with("__thaw_string_array_")
            || name.starts_with("__thaw_bool_array_")
            || name.starts_with("__thaw_object_array_")
            || name.starts_with("__thaw_pointer_array_");
        let generic_array_call = matches!(
            name,
            "__thaw_array_reverse"
                | "__thaw_array_copy_within"
                | "__thaw_array_fill"
                | "__thaw_array_slice"
                | "__thaw_array_to_reversed"
                | "__thaw_array_push"
                | "__thaw_array_unshift"
                | "__thaw_array_pop"
                | "__thaw_array_shift"
                | "__thaw_array_splice"
        );
        if !typed_array_call && !generic_array_call {
            return None;
        }
        Some(self.compile_array_call(name, args))
    }

    fn compile_array_call(
        &mut self,
        name: &str,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match name {
            "__thaw_number_array_to_string" => {
                return self.compile_single_array_arg_call(
                    "thaw_number_array_to_string",
                    args,
                    "String(number[])",
                )
            }
            "__thaw_string_array_to_string" => {
                return self.compile_single_array_arg_call(
                    "thaw_string_array_to_string",
                    args,
                    "String(string[])",
                )
            }
            "__thaw_bool_array_to_string" => {
                return self.compile_single_array_arg_call(
                    "thaw_bool_array_to_string",
                    args,
                    "String(boolean[])",
                )
            }
            "__thaw_object_array_to_string" => {
                return self.compile_single_array_arg_call(
                    "thaw_object_array_to_string",
                    args,
                    "String(object[])",
                )
            }
            "__thaw_number_array_join"
            | "__thaw_string_array_join"
            | "__thaw_bool_array_join"
            | "__thaw_object_array_join" => {
                let runtime = name.trim_start_matches("__thaw_");
                let runtime = format!("thaw_{runtime}");
                let [array, separator] = args else {
                    return Err("array join expects an array and separator".to_string());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let array = self.compile_array_data(handle)?;
                let separator = self.compile_expr(separator)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[array.into(), separator.into()],
                        "array_join",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array join returned no value".to_string());
            }
            "__thaw_array_reverse" => {
                let [array] = args else {
                    return Err("array reverse expects one operand".to_string());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(array) else {
                    return Err("array reverse requires a homogeneous array".to_string());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let width = self
                    .context
                    .i64_type()
                    .const_int(array_element_storage_bytes(&element), false);
                // Mutates the buffer's elements in place and returns the
                // same pointer -- the expression's own value is still the
                // original handle (no reallocation, so no new one).
                self.builder
                    .build_call(
                        self.module.get_function("thaw_array_reverse").unwrap(),
                        &[buffer.into(), width.into()],
                        "array_reverse",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(handle.into());
            }
            "__thaw_array_copy_within" => {
                if args.len() != 4 {
                    return Err("array copyWithin expects four operands".to_string());
                }
                let Some(HirType::Array(element)) = self.expr_hir_type(&args[0]) else {
                    return Err("array copyWithin requires a homogeneous array".to_string());
                };
                let handle = self.compile_expr(&args[0])?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let mut arguments = Vec::with_capacity(5);
                arguments.push(buffer.into());
                arguments.push(
                    self.context
                        .i64_type()
                        .const_int(array_element_storage_bytes(&element), false)
                        .into(),
                );
                for argument in &args[1..] {
                    arguments.push(self.compile_expr(argument)?.into());
                }
                // Mutates in place and returns the same pointer -- keep
                // using the original handle as this expression's value.
                self.builder
                    .build_call(
                        self.module.get_function("thaw_array_copy_within").unwrap(),
                        &arguments,
                        "array_copy_within",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(handle.into());
            }
            "__thaw_number_array_fill" | "__thaw_pointer_array_fill" | "__thaw_bool_array_fill" => {
                if args.len() != 4 {
                    return Err("array fill expects four operands".to_string());
                }
                let handle = self.compile_expr(&args[0])?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let mut arguments = Vec::with_capacity(4);
                arguments.push(buffer.into());
                for (index, argument) in args.iter().enumerate().skip(1) {
                    let mut value = self.compile_expr(argument)?;
                    if index == 1 && name == "__thaw_bool_array_fill" {
                        value = self
                            .builder
                            .build_int_z_extend(
                                value.into_int_value(),
                                self.context.i8_type(),
                                "array_fill_bool",
                            )
                            .map_err(|error| error.to_string())?
                            .into();
                    }
                    arguments.push(value.into());
                }
                let runtime = name.trim_start_matches("__thaw_");
                // Mutates in place and returns the same pointer -- keep
                // using the original handle as this expression's value.
                self.builder
                    .build_call(
                        self.module
                            .get_function(&format!("thaw_{runtime}"))
                            .unwrap(),
                        &arguments,
                        "array_fill",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(handle.into());
            }
            "__thaw_array_fill" => {
                if args.len() != 4 {
                    return Err("array fill expects four operands".to_string());
                }
                let Some(HirType::Array(element)) = self.expr_hir_type(&args[0]) else {
                    return Err("array fill requires a homogeneous array".to_string());
                };
                let handle = self.compile_expr(&args[0])?.into_pointer_value();
                let array = self.compile_array_data(handle)?;
                let value = self.compile_expr(&args[1])?;
                let value_slot = self
                    .builder
                    .build_alloca(value.get_type(), "array_fill_value")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(value_slot, value)
                    .map_err(|error| error.to_string())?;
                let value_bytes = self
                    .builder
                    .build_pointer_cast(
                        value_slot,
                        self.context.ptr_type(AddressSpace::default()),
                        "array_fill_value_bytes",
                    )
                    .map_err(|error| error.to_string())?;
                let arguments = [
                    array.into(),
                    value_bytes.into(),
                    self.context
                        .i64_type()
                        .const_int(array_element_storage_bytes(&element), false)
                        .into(),
                    self.compile_expr(&args[2])?.into(),
                    self.compile_expr(&args[3])?.into(),
                ];
                // Mutates in place and returns the same pointer -- keep
                // using the original handle as this expression's value.
                self.builder
                    .build_call(
                        self.module.get_function("thaw_array_fill").unwrap(),
                        &arguments,
                        "array_fill",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(handle.into());
            }
            "__thaw_array_slice" => {
                if args.len() != 3 {
                    return Err("array slice expects three operands".to_string());
                }
                let Some(HirType::Array(element)) = self.expr_hir_type(&args[0]) else {
                    return Err("array slice requires a homogeneous array".to_string());
                };
                let handle = self.compile_expr(&args[0])?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let mut arguments = Vec::with_capacity(4);
                arguments.push(buffer.into());
                arguments.push(
                    self.context
                        .i64_type()
                        .const_int(array_element_storage_bytes(&element), false)
                        .into(),
                );
                for argument in &args[1..] {
                    arguments.push(self.compile_expr(argument)?.into());
                }
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_slice").unwrap(),
                        &arguments,
                        "array_slice",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array slice returned no value".to_string())?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_array_to_reversed" => {
                let [array] = args else {
                    return Err("array toReversed expects one operand".to_string());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(array) else {
                    return Err("array toReversed requires a homogeneous array".to_string());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let width = self
                    .context
                    .i64_type()
                    .const_int(array_element_storage_bytes(&element), false);
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_to_reversed").unwrap(),
                        &[buffer.into(), width.into()],
                        "array_to_reversed",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array toReversed returned no value".to_string())?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_array_push" | "__thaw_array_unshift" => {
                let [receiver, values @ ..] = args else {
                    return Err("array push/unshift expects a receiver".to_string());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(receiver) else {
                    return Err("array push/unshift requires a homogeneous array".to_string());
                };
                let i64_type = self.context.i64_type();
                let width = array_element_storage_bytes(&element);
                let handle = self.compile_expr(receiver)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                // Build a scratch buffer holding each pushed/unshifted
                // value's bytes contiguously, matching the raw array
                // element layout, so a single runtime call can append/
                // prepend all of them at once.
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let values_ptr = if values.is_empty() {
                    ptr_type.const_null()
                } else {
                    let byte_array_type = self
                        .context
                        .i8_type()
                        .array_type((width as u32) * values.len() as u32);
                    let slot = self
                        .builder
                        .build_alloca(byte_array_type, "array_extend_values")
                        .map_err(|error| error.to_string())?;
                    for (index, value) in values.iter().enumerate() {
                        let compiled = self.compile_expr(value)?;
                        let offset = i64_type.const_int(width * index as u64, false);
                        let elem_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.i8_type(),
                                    slot,
                                    &[offset],
                                    "array_extend_slot",
                                )
                                .map_err(|error| error.to_string())?
                        };
                        self.builder
                            .build_store(elem_ptr, compiled)
                            .map_err(|error| error.to_string())?;
                    }
                    slot
                };
                let runtime = if name == "__thaw_array_push" {
                    "thaw_array_push_values"
                } else {
                    "thaw_array_unshift_values"
                };
                let new_buffer = self
                    .builder
                    .build_call(
                        self.module.get_function(runtime).unwrap(),
                        &[
                            buffer.into(),
                            i64_type.const_int(width, false).into(),
                            values_ptr.into(),
                            i64_type.const_int(values.len() as u64, false).into(),
                        ],
                        "array_extend",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array push/unshift returned no value".to_string())?
                    .into_pointer_value();
                // Mutate the receiver's existing handle in place -- every
                // alias sharing it observes the grown buffer.
                self.builder
                    .build_store(handle, new_buffer)
                    .map_err(|error| error.to_string())?;
                let new_length = self
                    .builder
                    .build_load(i64_type, new_buffer, "array_extend_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                return self
                    .builder
                    .build_signed_int_to_float(
                        new_length,
                        self.context.f64_type(),
                        "array_extend_length_f64",
                    )
                    .map_err(|error| error.to_string())
                    .map(Into::into);
            }
            "__thaw_array_pop" | "__thaw_array_shift" => {
                let [receiver] = args else {
                    return Err("array pop/shift expects one operand".to_string());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(receiver) else {
                    return Err("array pop/shift requires a homogeneous array".to_string());
                };
                let i64_type = self.context.i64_type();
                let width = array_element_storage_bytes(&element);
                let handle = self.compile_expr(receiver)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let old_length = self
                    .builder
                    .build_load(i64_type, buffer, "array_remove_old_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let is_empty = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        old_length,
                        i64_type.const_zero(),
                        "array_remove_is_empty",
                    )
                    .map_err(|error| error.to_string())?;
                let element_llvm_type = self.basic_type(&element)?;
                let out_slot = self
                    .builder
                    .build_alloca(element_llvm_type, "array_remove_value")
                    .map_err(|error| error.to_string())?;
                let runtime = if name == "__thaw_array_pop" {
                    "thaw_array_pop"
                } else {
                    "thaw_array_shift"
                };
                let new_buffer = self
                    .builder
                    .build_call(
                        self.module.get_function(runtime).unwrap(),
                        &[
                            buffer.into(),
                            i64_type.const_int(width, false).into(),
                            out_slot.into(),
                        ],
                        "array_remove",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array pop/shift returned no value".to_string())?
                    .into_pointer_value();
                self.builder
                    .build_store(handle, new_buffer)
                    .map_err(|error| error.to_string())?;
                // An empty receiver has nothing to remove -- the runtime
                // zero-fills `out_slot` in that case, which is only a valid
                // representation for scalar/pointer element types, so branch
                // to the element type's own zero value (a real empty array/
                // object, not a null pointer) instead of just reading it back.
                let function = self.current_function();
                let empty_block = self.context.append_basic_block(function, "array_remove_empty");
                let present_block = self
                    .context
                    .append_basic_block(function, "array_remove_present");
                let merge_block = self.context.append_basic_block(function, "array_remove_merge");
                self.builder
                    .build_conditional_branch(is_empty, empty_block, present_block)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(empty_block);
                let zero_value = self.compile_zero_value(&element)?;
                let empty_block = self.builder.get_insert_block().unwrap();
                self.builder
                    .build_unconditional_branch(merge_block)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(present_block);
                let removed_value = self
                    .builder
                    .build_load(element_llvm_type, out_slot, "array_removed_value")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_unconditional_branch(merge_block)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(merge_block);
                let phi = self
                    .builder
                    .build_phi(element_llvm_type, "array_remove_result")
                    .map_err(|error| error.to_string())?;
                phi.add_incoming(&[(&zero_value, empty_block), (&removed_value, present_block)]);
                return Ok(phi.as_basic_value());
            }
            "__thaw_array_splice" => {
                if args.len() < 3 {
                    return Err("array splice expects at least three operands".to_string());
                }
                let Some(HirType::Array(element)) = self.expr_hir_type(&args[0]) else {
                    return Err("array splice requires a homogeneous array".to_string());
                };
                let i64_type = self.context.i64_type();
                let width = array_element_storage_bytes(&element);
                let handle = self.compile_expr(&args[0])?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let start = self.compile_expr(&args[1])?;
                let delete_count = self.compile_expr(&args[2])?;
                let items = &args[3..];
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let values_ptr = if items.is_empty() {
                    ptr_type.const_null()
                } else {
                    let byte_array_type = self
                        .context
                        .i8_type()
                        .array_type((width as u32) * items.len() as u32);
                    let slot = self
                        .builder
                        .build_alloca(byte_array_type, "array_splice_values")
                        .map_err(|error| error.to_string())?;
                    for (index, item) in items.iter().enumerate() {
                        let compiled = self.compile_expr(item)?;
                        let offset = i64_type.const_int(width * index as u64, false);
                        let elem_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.i8_type(),
                                    slot,
                                    &[offset],
                                    "array_splice_slot",
                                )
                                .map_err(|error| error.to_string())?
                        };
                        self.builder
                            .build_store(elem_ptr, compiled)
                            .map_err(|error| error.to_string())?;
                    }
                    slot
                };
                let out_removed = self
                    .builder
                    .build_alloca(ptr_type, "array_splice_removed")
                    .map_err(|error| error.to_string())?;
                let new_buffer = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_splice").unwrap(),
                        &[
                            buffer.into(),
                            i64_type.const_int(width, false).into(),
                            start.into(),
                            delete_count.into(),
                            values_ptr.into(),
                            i64_type.const_int(items.len() as u64, false).into(),
                            out_removed.into(),
                        ],
                        "array_splice",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array splice returned no value".to_string())?
                    .into_pointer_value();
                self.builder
                    .build_store(handle, new_buffer)
                    .map_err(|error| error.to_string())?;
                let removed = self
                    .builder
                    .build_load(ptr_type, out_removed, "array_splice_removed_buffer")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(removed)?.into());
            }
            "__thaw_number_array_sort"
            | "__thaw_string_array_sort"
            | "__thaw_bool_array_sort"
            | "__thaw_object_array_sort" => {
                let [array] = args else {
                    return Err("array sort expects one operand".to_string());
                };
                let runtime = format!("thaw_{}", name.trim_start_matches("__thaw_"));
                let handle = self.compile_expr(array)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                // Mutates in place and returns the same pointer -- keep
                // using the original handle as this expression's value.
                self.builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[buffer.into()],
                        "array_sort",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(handle.into());
            }
            "__thaw_number_array_to_sorted"
            | "__thaw_string_array_to_sorted"
            | "__thaw_bool_array_to_sorted"
            | "__thaw_object_array_to_sorted" => {
                let [array] = args else {
                    return Err("array toSorted expects one operand".to_string());
                };
                let runtime = format!("thaw_{}", name.trim_start_matches("__thaw_"));
                let handle = self.compile_expr(array)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[buffer.into()],
                        "array_to_sorted",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array toSorted returned no value".to_string())?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_number_array_index_of"
            | "__thaw_number_array_includes"
            | "__thaw_string_array_index_of"
            | "__thaw_string_array_includes"
            | "__thaw_bool_array_index_of"
            | "__thaw_bool_array_includes"
            | "__thaw_object_array_index_of"
            | "__thaw_object_array_includes"
            | "__thaw_number_array_last_index_of"
            | "__thaw_string_array_last_index_of"
            | "__thaw_bool_array_last_index_of"
            | "__thaw_object_array_last_index_of" => {
                let runtime = format!("thaw_{}", name.trim_start_matches("__thaw_"));
                if args.len() != 3 {
                    return Err("array search expects three operands".to_string());
                }
                let mut arguments = Vec::with_capacity(3);
                for (index, argument) in args.iter().enumerate() {
                    let mut value = self.compile_expr(argument)?;
                    if index == 0 {
                        value = self.compile_array_data(value.into_pointer_value())?.into();
                    }
                    if index == 1 && name.starts_with("__thaw_bool_array_") {
                        value = self
                            .builder
                            .build_int_z_extend(
                                value.into_int_value(),
                                self.context.i8_type(),
                                "array_bool_needle",
                            )
                            .map_err(|error| error.to_string())?
                            .into();
                    }
                    arguments.push(value.into());
                }
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &arguments,
                        "array_search",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array search returned no value".to_string())?;
                if name.ends_with("_includes") {
                    return self
                        .builder
                        .build_int_compare(
                            IntPredicate::NE,
                            result.into_int_value(),
                            self.context.i8_type().const_zero(),
                            "array_includes_bool",
                        )
                        .map(Into::into)
                        .map_err(|error| error.to_string());
                }
                return Ok(result);
            }
            _ => {}
        }
        unreachable!("array call name was checked before dispatch")
    }
}
