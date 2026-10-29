impl<'ctx> HirCompiler<'ctx> {
    fn compile_array_insert_values(
        &mut self,
        values: &[HirExpr],
        element: &HirType,
        width: u64,
        label: &str,
    ) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), String> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        if values.is_empty() {
            return Ok((ptr_type.const_null(), ptr_type.const_null()));
        }
        let byte = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let bytes = self.builder.build_alloca(
            byte.array_type((width as u32) * values.len() as u32),
            &format!("{label}_values"),
        ).map_err(|error| error.to_string())?;
        let states = self.builder.build_alloca(
            byte.array_type(values.len() as u32),
            &format!("{label}_states"),
        ).map_err(|error| error.to_string())?;
        for (index, value) in values.iter().enumerate() {
            let compiled = self.compile_expr(value)?;
            let (payload, state) = if self.expr_hir_type(value)
                == Some(HirType::Optional(Box::new(element.clone())))
            {
                let tagged = compiled.into_struct_value();
                let present = self.builder.build_extract_value(tagged, 0, "insert_present")
                    .map_err(|error| error.to_string())?.into_int_value();
                let payload = self.builder.build_extract_value(tagged, 1, "insert_payload")
                    .map_err(|error| error.to_string())?;
                let state = self.builder.build_select(
                    present, byte.const_int(1, false), byte.const_int(2, false), "insert_state",
                ).map_err(|error| error.to_string())?.into_int_value();
                (payload, state)
            } else {
                (compiled, byte.const_int(1, false))
            };
            let value_ptr = unsafe {
                self.builder.build_in_bounds_gep(
                    byte, bytes, &[i64_type.const_int(width * index as u64, false)], "insert_slot",
                ).map_err(|error| error.to_string())?
            };
            self.builder.build_store(value_ptr, payload).map_err(|error| error.to_string())?;
            let state_ptr = unsafe {
                self.builder.build_in_bounds_gep(
                    byte, states, &[i64_type.const_int(index as u64, false)], "insert_state_slot",
                ).map_err(|error| error.to_string())?
            };
            self.builder.build_store(state_ptr, state).map_err(|error| error.to_string())?;
        }
        Ok((bytes, states))
    }

    fn compile_array_named_call(
        &mut self,
        name: &str,
        args: &[HirExpr],
    ) -> Option<Result<BasicValueEnum<'ctx>, String>> {
        let typed_array_call = name.starts_with("__thaw_number_array_")
            || name.starts_with("__thaw_string_array_")
            || name.starts_with("__thaw_bool_array_")
            || name.starts_with("__thaw_object_array_")
            || name.starts_with("__thaw_any_array_")
            || name.starts_with("__thaw_pointer_array_");
        let generic_array_call = matches!(
            name,
            "__thaw_tagged_array_join"
                | "__thaw_array_reverse"
                | "__thaw_array_copy_within"
                | "__thaw_array_fill"
                | "__thaw_array_slice"
                | "__thaw_array_rest"
                | "__thaw_bytes_slice"
                | "__thaw_bytes_subarray"
                | "__thaw_bytes_get"
                | "__thaw_bytes_set"
                | "__thaw_bytes_get_safe"
                | "__thaw_bytes_set_safe"
                | "__thaw_bytes_to_array"
                | "__thaw_bytes_widen_list"
                | "__thaw_bytes_narrow_list"
                | "__thaw_bytes_as_u8"
                | "__thaw_bytes_clone"
                | "__thaw_bytes_buffer"
                | "__thaw_bytes_fill"
                | "__thaw_bytes_reorder"
                | "__thaw_bytes_from_buffer_like"
                | "__thaw_bytes_store_array"
                | "__thaw_bytes_store_length"
                | "__thaw_bytes_byte_offset"
                | "__thaw_array_to_reversed"
                | "__thaw_array_push"
                | "__thaw_array_unshift"
                | "__thaw_array_pop"
                | "__thaw_array_shift"
                | "__thaw_array_pop_optional"
                | "__thaw_array_shift_optional"
                | "__thaw_array_splice"
                | "__thaw_bytes_to_string"
                | "__thaw_bytes_from_string"
                | "__thaw_bytes_from_array"
                | "__thaw_bytes_concat"
                | "__thaw_bytes_equals"
                | "__thaw_bytes_read"
                | "__thaw_bytes_write"
                | "__thaw_bytes_read_i64"
                | "__thaw_bytes_write_i64"
                | "__thaw_bytes_copy"
                | "__thaw_bytes_index_of"
                | "__thaw_bytes_byte_length"
                | "__thaw_bytes_alloc"
                | "__thaw_arraybuffer_alloc"
                | "__thaw_dataview_new"
                | "__thaw_arraybuffer_transfer"
                | "__thaw_dataview_get"
                | "__thaw_dataview_set"
                | "__thaw_bytes_kind"
                | "__thaw_bytes_alloc_fill"
                | "__thaw_bytes_set_from_string"
        );
        if !typed_array_call && !generic_array_call {
            return None;
        }
        if let Err(error) = self.guard_array_mutation(name, args) {
            return Some(Err(error));
        }
        Some(self.compile_array_call(name, args))
    }

    /// Throws a `TypeError` when a mutating array method targets an array that
    /// `Object.freeze`/`seal`/`preventExtensions` locked (the object-state
    /// table is keyed by the array handle). Only programs that call those
    /// functions pay for the check, and only for a receiver that can be
    /// evaluated twice without observable effects.
    pub(super) fn guard_array_mutation(
        &mut self,
        name: &str,
        args: &[HirExpr],
    ) -> Result<(), String> {
        if !self.tracks_object_integrity {
            return Ok(());
        }
        let suffix = name.rsplit("_array_").next().unwrap_or(name);
        // 0: not extensible, 1: sealed, 2: frozen (the table's query codes).
        let (query, message) = match suffix {
            "push" | "unshift" | "splice" => (0, "Cannot add property, object is not extensible"),
            "pop" | "shift" | "pop_optional" | "shift_optional" => {
                (1, "Cannot delete property of [object Array]")
            }
            "reverse" | "copy_within" | "fill" | "sort" => {
                (2, "Cannot assign to read only property of object '[object Array]'")
            }
            _ => return Ok(()),
        };
        let Some(receiver) = args.first() else {
            return Ok(());
        };
        if !matches!(self.expr_hir_type(receiver), Some(HirType::Array(_))) {
            return Ok(());
        }
        self.compile_guard_array_state(receiver, query, message)
    }

    pub(super) fn compile_guard_array_state(
        &mut self,
        receiver: &HirExpr,
        query: u64,
        message: &str,
    ) -> Result<(), String> {
        fn repeatable(expr: &HirExpr) -> bool {
            match expr {
                HirExpr::Var(_) => true,
                HirExpr::PropAccess(object, _, _) => repeatable(object),
                _ => false,
            }
        }
        if !repeatable(receiver) {
            return Ok(());
        }
        let handle = self.compile_expr(receiver)?;
        self.compile_guard_array_handle(handle.into_pointer_value(), query, message)
    }

    pub(super) fn compile_guard_array_handle(
        &mut self,
        handle: inkwell::values::PointerValue<'ctx>,
        query: u64,
        message: &str,
    ) -> Result<(), String> {
        let state = self
            .builder
            .build_call(
                self.module.get_function("thaw_object_state").unwrap(),
                &[handle.into(), self.context.i8_type().const_int(query, false).into()],
                "array_integrity_state",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_object_state returned no value")?
            .into_int_value();
        // Query 0 answers "is extensible"; the others answer "is locked".
        let blocked = if query == 0 {
            self.builder.build_not(state, "array_not_extensible")
                .map_err(|error| error.to_string())?
        } else {
            state
        };
        let function = self.current_function();
        let throw = self.context.append_basic_block(function, "array_integrity_blocked");
        let allowed = self.context.append_basic_block(function, "array_integrity_allowed");
        self.builder
            .build_conditional_branch(blocked, throw, allowed)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(throw);
        self.compile_throw_type_error(message)?;
        self.builder.position_at_end(allowed);
        Ok(())
    }

    /// The `ByteView` record behind a Buffer value (the handle's data word).
    fn compile_bytes_record(&mut self, buffer: &HirExpr) -> Result<PointerValue<'ctx>, String> {
        let handle = self.compile_expr(buffer)?.into_pointer_value();
        self.compile_array_data(handle)
    }

    /// `0 <= index < len` for a view record (false for NaN).
    fn compile_bytes_index_in_range(
        &mut self,
        record: PointerValue<'ctx>,
        index: inkwell::values::FloatValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let f64_type = self.context.f64_type();
        let length = self
            .builder
            .build_load(self.context.i64_type(), record, "bytes_len")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let length = self
            .builder
            .build_unsigned_int_to_float(length, f64_type, "bytes_len_f64")
            .map_err(|error| error.to_string())?;
        let non_negative = self
            .builder
            .build_float_compare(inkwell::FloatPredicate::OGE, index, f64_type.const_zero(), "bytes_index_non_negative")
            .map_err(|error| error.to_string())?;
        let below = self
            .builder
            .build_float_compare(inkwell::FloatPredicate::OLT, index, length, "bytes_index_below_len")
            .map_err(|error| error.to_string())?;
        self.builder.build_and(non_negative, below, "bytes_index_in_range").map_err(|error| error.to_string())
    }

    /// Address of byte `index` of a view record `{ len, tag, base, store }`.
    /// The caller guarantees `0 <= index < len` (thaw-hir range-checks first).
    fn compile_bytes_slot(
        &mut self,
        record: PointerValue<'ctx>,
        index: inkwell::values::FloatValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let base_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    record,
                    &[self.context.i64_type().const_int(16, false)],
                    "bytes_base_slot",
                )
                .map_err(|error| error.to_string())?
        };
        let base = self
            .builder
            .build_load(ptr_type, base_slot, "bytes_base")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let index = self
            .builder
            .build_float_to_unsigned_int(index, self.context.i64_type(), "bytes_index")
            .map_err(|error| error.to_string())?;
        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), base, &[index], "bytes_slot")
                .map_err(|error| error.to_string())
        }
    }

    fn compile_array_call(
        &mut self,
        name: &str,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match name {
            "__thaw_number_array_to_string"
            | "__thaw_string_array_to_string"
            | "__thaw_bool_array_to_string"
            | "__thaw_object_array_to_string" => {
                let [array] = args else {
                    return Err("array string conversion expects one operand".into());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let data = self.compile_array_data(handle)?;
                let presence = self.compile_array_presence(handle)?;
                let runtime = name.trim_start_matches("__thaw_");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&format!("thaw_{runtime}")).unwrap(),
                        &[data.into(), presence.into()],
                        "array_to_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array string conversion returned no value".into());
            }
            "__thaw_bytes_to_string" => {
                let [buffer, encoding] = args else {
                    return Err("bytes toString expects a buffer and an encoding".into());
                };
                let handle = self.compile_expr(buffer)?.into_pointer_value();
                let data = self.compile_array_data(handle)?;
                let encoding = self.compile_expr(encoding)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_bytes_to_string").unwrap(),
                        &[data.into(), encoding.into()],
                        "bytes_to_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("bytes toString returned no value".to_string());
            }
            "__thaw_bytes_from_string" => {
                let [text, encoding] = args else {
                    return Err("Buffer.from expects a string and an encoding".into());
                };
                let text = self.compile_expr(text)?;
                let encoding = self.compile_expr(encoding)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_bytes_from_string").unwrap(),
                        &[text.into(), encoding.into()],
                        "bytes_from_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Buffer.from returned no value".to_string())?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_bytes_from_array" => {
                let [source] = args else {
                    return Err("`__thaw_bytes_from_array` expects one operand".into());
                };
                let handle = self.compile_expr(source)?.into_pointer_value();
                let data = self.compile_array_data(handle)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_bytes_from_array").unwrap(),
                        &[data.into()],
                        "bytes_from_array",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("`__thaw_bytes_from_array` returned no value".to_string())?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_bytes_concat" => {
                let [list, total] = args else {
                    return Err("Buffer.concat expects a list and a totalLength".into());
                };
                let handle = self.compile_expr(list)?.into_pointer_value();
                let data = self.compile_array_data(handle)?;
                let total = self.compile_expr(total)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_bytes_concat").unwrap(),
                        &[data.into(), total.into()],
                        "bytes_concat",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Buffer.concat returned no value".to_string())?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_bytes_equals" => {
                let [left, right] = args else {
                    return Err("Buffer equals expects two operands".into());
                };
                let left = self.compile_expr(left)?.into_pointer_value();
                let left = self.compile_array_data(left)?;
                let right = self.compile_expr(right)?.into_pointer_value();
                let right = self.compile_array_data(right)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_bytes_equals").unwrap(),
                        &[left.into(), right.into()],
                        "bytes_equals",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Buffer equals returned no value".to_string())?;
                // The runtime returns an i8; thaw `Bool` is i1.
                return self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        result.into_int_value(),
                        self.context.i8_type().const_zero(),
                        "bytes_equals_bool",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_bytes_copy" | "__thaw_bytes_index_of" => {
                let (runtime, label, tail) = if name == "__thaw_bytes_index_of" {
                    ("thaw_bytes_index_of", "bytes_index_of", 2)
                } else {
                    ("thaw_bytes_copy", "bytes_copy", 3)
                };
                let [first, second, rest @ ..] = args else {
                    return Err(format!("`{name}` expects two buffers and offsets"));
                };
                if rest.len() != tail {
                    return Err(format!("`{name}` got the wrong number of operands"));
                }
                let first = self.compile_expr(first)?.into_pointer_value();
                let first = self.compile_array_data(first)?;
                let second = self.compile_expr(second)?.into_pointer_value();
                let second = self.compile_array_data(second)?;
                let mut call_args = vec![first.into(), second.into()];
                for argument in rest {
                    call_args.push(self.compile_expr(argument)?.into());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function(runtime).unwrap(),
                        &call_args,
                        label,
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("`{name}` returned no value"));
            }
            "__thaw_bytes_set_from_string" => {
                let [buffer, text, encoding] = args else {
                    return Err(
                        "bytes setFrom expects a buffer, text, and an encoding".into(),
                    );
                };
                let handle = self.compile_expr(buffer)?.into_pointer_value();
                let data = self.compile_array_data(handle)?;
                let text = self.compile_expr(text)?;
                let encoding = self.compile_expr(encoding)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_bytes_set_from_string")
                            .unwrap(),
                        &[data.into(), text.into(), encoding.into()],
                        "bytes_set_from_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("bytes setFrom returned no value".into());
            }
            "__thaw_bytes_read" | "__thaw_bytes_write" | "__thaw_bytes_read_i64"
            | "__thaw_bytes_write_i64" => {
                let (runtime, label) = match name {
                    "__thaw_bytes_write" => ("thaw_bytes_write", "bytes_write"),
                    "__thaw_bytes_read_i64" => ("thaw_bytes_read_i64", "bytes_read_i64"),
                    "__thaw_bytes_write_i64" => ("thaw_bytes_write_i64", "bytes_write_i64"),
                    _ => ("thaw_bytes_read", "bytes_read"),
                };
                let [receiver, rest @ ..] = args else {
                    return Err(format!("`{name}` expects a receiver"));
                };
                let handle = self.compile_expr(receiver)?.into_pointer_value();
                let mut call_args = vec![self.compile_array_data(handle)?.into()];
                for argument in rest {
                    call_args.push(self.compile_expr(argument)?.into());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function(runtime).unwrap(),
                        &call_args,
                        label,
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("`{name}` returned no value"));
            }
            "__thaw_bytes_byte_offset" | "__thaw_bytes_store_length" => {
                let [receiver] = args else {
                    return Err(format!("`{name}` expects a receiver"));
                };
                let record = self.compile_bytes_record(receiver)?;
                let runtime = if name == "__thaw_bytes_byte_offset" {
                    "thaw_bytes_byte_offset"
                } else {
                    "thaw_bytes_store_length"
                };
                return self
                    .builder
                    .build_call(self.module.get_function(runtime).unwrap(), &[record.into()], "bytes_view_call")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("`{name}` returned no value"));
            }
            "__thaw_bytes_reorder" => {
                let [receiver, order] = args else {
                    return Err("`__thaw_bytes_reorder` expects a buffer and an order".into());
                };
                let record = self.compile_bytes_record(receiver)?;
                let order = self.compile_expr(order)?;
                self.builder
                    .build_call(
                        self.module.get_function("thaw_bytes_reorder").unwrap(),
                        &[record.into(), order.into()],
                        "bytes_reorder",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(self.context.f64_type().const_zero().into());
            }
            "__thaw_bytes_fill" => {
                let [receiver, pattern, start, end] = args else {
                    return Err("`__thaw_bytes_fill` expects a buffer, a pattern and a range".into());
                };
                let record = self.compile_bytes_record(receiver)?;
                let pattern = self.compile_bytes_record(pattern)?;
                let start = self.compile_expr(start)?;
                let end = self.compile_expr(end)?;
                self.builder
                    .build_call(
                        self.module.get_function("thaw_bytes_fill").unwrap(),
                        &[record.into(), pattern.into(), start.into(), end.into()],
                        "bytes_fill",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(self.context.f64_type().const_zero().into());
            }
            "__thaw_bytes_from_buffer_like" => {
                let [source, offset, length, kind] = args else {
                    return Err("`__thaw_bytes_from_buffer_like` expects four operands".into());
                };
                let record = self.compile_bytes_record(source)?;
                let offset = self.compile_expr(offset)?;
                let length = self.compile_expr(length)?;
                let kind = self.compile_expr(kind)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_bytes_from_buffer_like").unwrap(),
                        &[record.into(), offset.into(), length.into(), kind.into()],
                        "bytes_from_buffer_like",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("`__thaw_bytes_from_buffer_like` returned no value")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_bytes_slice" | "__thaw_bytes_subarray" => {
                let [receiver, start, end] = args else {
                    return Err(format!("`{name}` expects a receiver, a start and an end"));
                };
                let record = self.compile_bytes_record(receiver)?;
                let start = self.compile_expr(start)?;
                let end = self.compile_expr(end)?;
                let runtime = if name == "__thaw_bytes_slice" { "thaw_bytes_slice" } else { "thaw_bytes_subarray" };
                let result = self
                    .builder
                    .build_call(self.module.get_function(runtime).unwrap(), &[record.into(), start.into(), end.into()], "bytes_view")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("`{name}` returned no value"))?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            // One handle in, one handle out: widen / narrow / clone / retag a
            // byte buffer (or a list of them) through its runtime function.
            "__thaw_bytes_to_array" | "__thaw_bytes_widen_list" | "__thaw_bytes_narrow_list"
            | "__thaw_bytes_as_u8" | "__thaw_bytes_clone" | "__thaw_bytes_buffer" => {
                let [receiver] = args else {
                    return Err(format!("`{name}` expects one operand"));
                };
                let record = self.compile_bytes_record(receiver)?;
                let runtime = match name {
                    "__thaw_bytes_widen_list" => "thaw_bytes_widen_list",
                    "__thaw_bytes_narrow_list" => "thaw_bytes_narrow_list",
                    "__thaw_bytes_as_u8" => "thaw_bytes_as_u8",
                    "__thaw_bytes_clone" => "thaw_bytes_clone",
                    "__thaw_bytes_buffer" => "thaw_bytes_buffer",
                    _ => "thaw_bytes_to_array",
                };
                let result = self
                    .builder
                    .build_call(self.module.get_function(runtime).unwrap(), &[record.into()], "bytes_convert")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("`{name}` returned no value"))?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_bytes_store_array" => {
                let [receiver, array] = args else {
                    return Err("`__thaw_bytes_store_array` expects a buffer and an array".into());
                };
                let record = self.compile_bytes_record(receiver)?;
                let array = self.compile_expr(array)?.into_pointer_value();
                let array = self.compile_array_data(array)?;
                self.builder
                    .build_call(self.module.get_function("thaw_bytes_store_array").unwrap(), &[record.into(), array.into()], "bytes_store_array")
                    .map_err(|error| error.to_string())?;
                return Ok(self.context.f64_type().const_zero().into());
            }
            "__thaw_bytes_get" | "__thaw_bytes_get_safe" => {
                let [receiver, index] = args else {
                    return Err(format!("`{name}` expects a buffer and an index"));
                };
                let record = self.compile_bytes_record(receiver)?;
                let index = self.compile_expr(index)?.into_float_value();
                let f64_type = self.context.f64_type();
                let checked_blocks = if name == "__thaw_bytes_get_safe" {
                    let in_range = self.compile_bytes_index_in_range(record, index)?;
                    let function = self.current_function();
                    let read = self.context.append_basic_block(function, "bytes_get_read");
                    let done = self.context.append_basic_block(function, "bytes_get_done");
                    let start = self.builder.get_insert_block().ok_or("no insert block")?;
                    self.builder.build_conditional_branch(in_range, read, done).map_err(|error| error.to_string())?;
                    self.builder.position_at_end(read);
                    Some((start, done))
                } else {
                    None
                };
                let slot = self.compile_bytes_slot(record, index)?;
                let byte = self
                    .builder
                    .build_load(self.context.i8_type(), slot, "bytes_get")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let value = self
                    .builder
                    .build_unsigned_int_to_float(byte, f64_type, "bytes_get_f64")
                    .map_err(|error| error.to_string())?;
                let Some((start, done)) = checked_blocks else {
                    return Ok(value.into());
                };
                let read_end = self.builder.get_insert_block().ok_or("no insert block")?;
                self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
                self.builder.position_at_end(done);
                let phi = self.builder.build_phi(f64_type, "bytes_get_result").map_err(|error| error.to_string())?;
                phi.add_incoming(&[(&value, read_end), (&f64_type.const_float(f64::NAN), start)]);
                return Ok(phi.as_basic_value());
            }
            "__thaw_bytes_set" | "__thaw_bytes_set_safe" => {
                let [receiver, index, value] = args else {
                    return Err("`__thaw_bytes_set` expects a buffer, an index and a value".into());
                };
                let record = self.compile_bytes_record(receiver)?;
                let index = self.compile_expr(index)?.into_float_value();
                let value = self.compile_expr(value)?.into_float_value();
                let f64_type = self.context.f64_type();
                let checked_done = if name == "__thaw_bytes_set_safe" {
                    let in_range = self.compile_bytes_index_in_range(record, index)?;
                    let function = self.current_function();
                    let write = self.context.append_basic_block(function, "bytes_set_write");
                    let done = self.context.append_basic_block(function, "bytes_set_done");
                    self.builder.build_conditional_branch(in_range, write, done).map_err(|error| error.to_string())?;
                    self.builder.position_at_end(write);
                    Some(done)
                } else {
                    None
                };
                let slot = self.compile_bytes_slot(record, index)?;
                // ToUint8: truncate, wrap modulo 256; NaN / infinities / |v| >= 2^63 are 0.
                let finite = self
                    .builder
                    .build_float_compare(
                        inkwell::FloatPredicate::OLT,
                        self.builder
                            .build_call(
                                self.module.get_function("fabs").unwrap_or_else(|| {
                                    self.module.add_function(
                                        "fabs",
                                        f64_type.fn_type(&[f64_type.into()], false),
                                        Some(inkwell::module::Linkage::External),
                                    )
                                }),
                                &[value.into()],
                                "bytes_set_abs",
                            )
                            .map_err(|error| error.to_string())?
                            .try_as_basic_value()
                            .basic()
                            .ok_or("fabs returned no value")?
                            .into_float_value(),
                        f64_type.const_float(9.0e18),
                        "bytes_set_finite",
                    )
                    .map_err(|error| error.to_string())?;
                let bounded = self
                    .builder
                    .build_select(finite, value, f64_type.const_zero(), "bytes_set_bounded")
                    .map_err(|error| error.to_string())?
                    .into_float_value();
                let wide = self
                    .builder
                    .build_float_to_signed_int(bounded, self.context.i64_type(), "bytes_set_wide")
                    .map_err(|error| error.to_string())?;
                let byte = self
                    .builder
                    .build_int_truncate(wide, self.context.i8_type(), "bytes_set_byte")
                    .map_err(|error| error.to_string())?;
                self.builder.build_store(slot, byte).map_err(|error| error.to_string())?;
                if let Some(done) = checked_done {
                    self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
                    self.builder.position_at_end(done);
                }
                return Ok(value.into());
            }
            "__thaw_bytes_byte_length" => {
                let [text, encoding] = args else {
                    return Err("Buffer.byteLength expects a string and an encoding".into());
                };
                let text = self.compile_expr(text)?;
                let encoding = self.compile_expr(encoding)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_bytes_byte_length").unwrap(),
                        &[text.into(), encoding.into()],
                        "bytes_byte_length",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Buffer.byteLength returned no value".to_string());
            }
            "__thaw_arraybuffer_alloc" => {
                let [size] = args else {
                    return Err("`__thaw_arraybuffer_alloc` expects one operand".into());
                };
                let size = self.compile_expr(size)?;
                let result = self
                    .builder
                    .build_call(self.module.get_function("thaw_arraybuffer_alloc").unwrap(), &[size.into()], "arraybuffer_alloc")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("`__thaw_arraybuffer_alloc` returned no value")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_dataview_new" => {
                let [source, offset, length] = args else {
                    return Err("`__thaw_dataview_new` expects a buffer, an offset and a length".into());
                };
                let record = self.compile_bytes_record(source)?;
                let offset = self.compile_expr(offset)?;
                let length = self.compile_expr(length)?;
                let result = self
                    .builder
                    .build_call(self.module.get_function("thaw_dataview_new").unwrap(), &[record.into(), offset.into(), length.into()], "dataview_new")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("`__thaw_dataview_new` returned no value")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_arraybuffer_transfer" => {
                let [receiver, length] = args else {
                    return Err("`__thaw_arraybuffer_transfer` expects a buffer and a length".into());
                };
                let record = self.compile_bytes_record(receiver)?;
                let length = self.compile_expr(length)?;
                let result = self
                    .builder
                    .build_call(self.module.get_function("thaw_arraybuffer_transfer").unwrap(), &[record.into(), length.into()], "arraybuffer_transfer")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("`__thaw_arraybuffer_transfer` returned no value")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_bytes_kind" => {
                // `(view)` is the inspect kind; `(view, field)` another `thaw_bytes_console_info` field.
                let record = self.compile_bytes_record(args.first().ok_or("`__thaw_bytes_kind` expects an operand")?)?;
                let zero = match args.get(1) {
                    Some(field) => self.compile_expr(field)?.into_float_value(),
                    None => self.context.f64_type().const_zero(),
                };
                return self
                    .builder
                    .build_call(self.module.get_function("thaw_bytes_console_info").unwrap(), &[record.into(), zero.into()], "bytes_kind")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "`__thaw_bytes_kind` returned no value".to_string());
            }
            "__thaw_dataview_get" | "__thaw_dataview_set" => {
                let record = self.compile_bytes_record(args.first().ok_or("DataView access needs a view")?)?;
                let mut call_args = vec![record.into()];
                for arg in &args[1..] {
                    call_args.push(self.compile_expr(arg)?.into());
                }
                let runtime = if name == "__thaw_dataview_get" { "thaw_dataview_get" } else { "thaw_dataview_set" };
                let result = self
                    .builder
                    .build_call(self.module.get_function(runtime).unwrap(), &call_args, "dataview_access")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic();
                return Ok(result.unwrap_or_else(|| self.context.f64_type().const_zero().into()));
            }
            "__thaw_bytes_alloc" => {
                let [size] = args else {
                    return Err("Buffer.alloc expects one operand".into());
                };
                let size = self.compile_expr(size)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_bytes_alloc").unwrap(),
                        &[size.into()],
                        "bytes_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Buffer.alloc returned no value".to_string())?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_bytes_alloc_fill" => {
                let [size, pattern] = args else {
                    return Err("Buffer.alloc with a fill expects two operands".into());
                };
                let size = self.compile_expr(size)?;
                let pattern = self.compile_expr(pattern)?.into_pointer_value();
                let pattern = self.compile_array_data(pattern)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_bytes_alloc_fill").unwrap(),
                        &[size.into(), pattern.into()],
                        "bytes_alloc_fill",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Buffer.alloc returned no value".to_string())?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_number_array_join"
            | "__thaw_string_array_join"
            | "__thaw_bool_array_join"
            | "__thaw_object_array_join"
            | "__thaw_tagged_array_join" => {
                let runtime = name.trim_start_matches("__thaw_");
                let runtime = format!("thaw_{runtime}");
                let [array, separator] = args else {
                    return Err("array join expects an array and separator".to_string());
                };
                let tagged = if name == "__thaw_tagged_array_join" {
                    let Some(HirType::Array(element)) = self.expr_hir_type(array) else {
                        return Err("tagged join requires an array".into());
                    };
                    let (kind, tag, tags) = if let HirType::Union(members) = element.as_ref() {
                        let tags = members.iter().map(|member| match member {
                            HirType::F64 => Ok('n'),
                            HirType::Str => Ok('s'),
                            HirType::Bool => Ok('b'),
                            HirType::Object(_) => Ok('o'),
                            HirType::Undefined | HirType::Null => Ok('u'),
                            _ => Err("array join does not support this union member".to_string()),
                        }).collect::<Result<String, _>>()?;
                        if members.len() > u8::MAX as usize {
                            return Err("array join has too many union members".into());
                        }
                        (5, 0, Some(tags))
                    } else {
                        let (payload, tag) = match element.as_ref() {
                        HirType::Optional(payload) | HirType::Nullable(payload) => (payload.as_ref(), 1),
                        HirType::Nullish(payload) => (payload.as_ref(), 0),
                        HirType::Undefined => (&HirType::Undefined, 0),
                        HirType::Null => (&HirType::Null, 0),
                        _ => return Err("tagged join requires tagged elements".into()),
                        };
                        let kind = match payload {
                        HirType::F64 => 0,
                        HirType::Str => 1,
                        HirType::Bool => 2,
                        HirType::Object(_) => 3,
                        HirType::Undefined | HirType::Null => 4,
                        _ => return Err("tagged join does not support this payload type".into()),
                        };
                        (kind, tag, None)
                    };
                    Some((kind, tag, tags))
                } else {
                    None
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let array = self.compile_array_data(handle)?;
                let presence = self.compile_array_presence(handle)?;
                let separator = self.compile_expr(separator)?;
                let mut arguments = vec![array.into(), presence.into(), separator.into()];
                if let Some((kind, tag, tags)) = tagged {
                    arguments.push(self.context.i8_type().const_int(kind, false).into());
                    arguments.push(self.context.i8_type().const_int(tag, false).into());
                    let tags = if let Some(tags) = tags {
                        self.builder.build_global_string_ptr(&tags, "array_join_union_tags")
                            .map_err(|error| error.to_string())?.as_pointer_value()
                    } else {
                        self.context.ptr_type(AddressSpace::default()).const_null()
                    };
                    arguments.push(tags.into());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &arguments,
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
                    .const_int(array_element_storage_bytes(&element)?, false);
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
                let presence = self.compile_array_presence(handle)?;
                self.builder
                    .build_call(
                        self.module
                            .get_function("thaw_array_presence_reverse")
                            .unwrap(),
                        &[presence.into()],
                        "array_presence_reverse",
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
                let target = self.compile_expr(&args[1])?.into_float_value();
                let start = self.compile_expr(&args[2])?.into_float_value();
                let end = self.compile_expr(&args[3])?.into_float_value();
                let arguments = vec![
                    buffer.into(),
                    self.context
                        .i64_type()
                        .const_int(array_element_storage_bytes(&element)?, false)
                        .into(),
                    target.into(),
                    start.into(),
                    end.into(),
                ];
                // Mutates in place and returns the same pointer -- keep
                // using the original handle as this expression's value.
                self.builder
                    .build_call(
                        self.module.get_function("thaw_array_copy_within").unwrap(),
                        &arguments,
                        "array_copy_within",
                    )
                    .map_err(|error| error.to_string())?;
                let presence = self.compile_array_presence(handle)?;
                self.builder
                    .build_call(
                        self.module
                            .get_function("thaw_array_presence_copy_within")
                            .unwrap(),
                        &[presence.into(), target.into(), start.into(), end.into()],
                        "array_presence_copy_within",
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
                let mut value = self.compile_expr(&args[1])?;
                if name == "__thaw_bool_array_fill" {
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
                let start = self.compile_expr(&args[2])?.into_float_value();
                let end = self.compile_expr(&args[3])?.into_float_value();
                let arguments = [buffer.into(), value.into(), start.into(), end.into()];
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
                let presence = self.compile_array_presence(handle)?;
                let length = self.builder.build_load(self.context.i64_type(), buffer, "fill_length")
                    .map_err(|error| error.to_string())?.into_int_value();
                self.builder
                    .build_call(
                        self.module.get_function("thaw_array_presence_fill").unwrap(),
                        &[presence.into(), length.into(), start.into(), end.into(), self.context.i8_type().const_int(1, false).into()],
                        "array_presence_fill",
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
                let (value, state) = if self.expr_hir_type(&args[1])
                    == Some(HirType::Optional(element.clone()))
                {
                    let optional = value.into_struct_value();
                    let present = self.builder.build_extract_value(optional, 0, "fill_present")
                        .map_err(|error| error.to_string())?.into_int_value();
                    let value = self.builder.build_extract_value(optional, 1, "fill_payload")
                        .map_err(|error| error.to_string())?;
                    let state = self.builder.build_select(
                        present, self.context.i8_type().const_int(1, false),
                        self.context.i8_type().const_int(2, false), "fill_state",
                    ).map_err(|error| error.to_string())?.into_int_value();
                    (value, state)
                } else {
                    (value, self.context.i8_type().const_int(1, false))
                };
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
                let start = self.compile_expr(&args[2])?.into_float_value();
                let end = self.compile_expr(&args[3])?.into_float_value();
                let arguments = [
                    array.into(),
                    value_bytes.into(),
                    self.context
                        .i64_type()
                        .const_int(array_element_storage_bytes(&element)?, false)
                        .into(),
                    start.into(),
                    end.into(),
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
                let presence = self.compile_array_presence(handle)?;
                let length = self.builder.build_load(self.context.i64_type(), array, "fill_length")
                    .map_err(|error| error.to_string())?.into_int_value();
                let presence = self.builder
                    .build_call(
                        self.module.get_function("thaw_array_presence_fill").unwrap(),
                        &[presence.into(), length.into(), start.into(), end.into(), state.into()],
                        "array_presence_fill",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value().basic().ok_or("array presence fill returned no value")?
                    .into_pointer_value();
                self.compile_array_set_presence(handle, presence)?;
                return Ok(handle.into());
            }
            "__thaw_array_slice" | "__thaw_array_rest" => {
                if args.len() != 3 {
                    return Err("array slice expects three operands".to_string());
                }
                let Some(HirType::Array(element)) = self.expr_hir_type(&args[0]) else {
                    return Err("array slice requires a homogeneous array".to_string());
                };
                let handle = self.compile_expr(&args[0])?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let length = self
                    .builder
                    .build_load(self.context.i64_type(), buffer, "array_slice_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let start = self.compile_expr(&args[1])?.into_float_value();
                let end = self.compile_expr(&args[2])?.into_float_value();
                let arguments = vec![
                    buffer.into(),
                    self.context
                        .i64_type()
                        .const_int(array_element_storage_bytes(&element)?, false)
                        .into(),
                    start.into(),
                    end.into(),
                ];
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
                let presence = self.compile_array_presence(handle)?;
                let sliced_presence = self
                    .builder
                    .build_call(
                        self.module
                            .get_function(if name == "__thaw_array_rest" {
                                "thaw_array_presence_rest"
                            } else {
                                "thaw_array_presence_slice"
                            })
                            .unwrap(),
                        &[presence.into(), length.into(), start.into(), end.into()],
                        "array_slice_presence",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array presence slice returned no value".to_string())?
                    .into_pointer_value();
                return Ok(self
                    .compile_array_wrap_with_presence(result, sliced_presence)?
                    .into());
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
                let length = self
                    .builder
                    .build_load(self.context.i64_type(), buffer, "array_to_reversed_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let width = self
                    .context
                    .i64_type()
                    .const_int(array_element_storage_bytes(&element)?, false);
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
                let presence = self.compile_array_presence(handle)?;
                let copied_presence = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_array_presence_slice")
                            .unwrap(),
                        &[
                            presence.into(),
                            length.into(),
                            self.context.f64_type().const_zero().into(),
                            self.context.f64_type().const_float(f64::INFINITY).into(),
                        ],
                        "array_to_reversed_presence",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array presence copy returned no value")?
                    .into_pointer_value();
                self.builder
                    .build_call(
                        self.module
                            .get_function("thaw_array_presence_reverse")
                            .unwrap(),
                        &[copied_presence.into()],
                        "array_to_reversed_presence_reverse",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function("thaw_array_presence_densify").unwrap(),
                        &[copied_presence.into()],
                        "array_to_reversed_presence_densify",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(self
                    .compile_array_wrap_with_presence(result, copied_presence)?
                    .into());
            }
            "__thaw_array_push" | "__thaw_array_unshift" => {
                let [receiver, values @ ..] = args else {
                    return Err("array push/unshift expects a receiver".to_string());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(receiver) else {
                    return Err("array push/unshift requires a homogeneous array".to_string());
                };
                let i64_type = self.context.i64_type();
                let width = array_element_storage_bytes(&element)?;
                let handle = self.compile_expr(receiver)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let old_length = self
                    .builder
                    .build_load(i64_type, buffer, "array_extend_old_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let (values_ptr, states_ptr) = self.compile_array_insert_values(
                    values, &element, width, "array_extend",
                )?;
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
                let presence = self.compile_array_presence(handle)?;
                let new_presence = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_presence_extend").unwrap(),
                        &[
                            presence.into(),
                            old_length.into(),
                            i64_type.const_int(values.len() as u64, false).into(),
                            self.context.i8_type().const_int(u64::from(name == "__thaw_array_unshift"), false).into(),
                            states_ptr.into(),
                        ],
                        "array_extend_presence",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array presence extend returned no value")?
                    .into_pointer_value();
                self.compile_array_set_presence(handle, new_presence)?;
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
            "__thaw_array_pop"
            | "__thaw_array_shift"
            | "__thaw_array_pop_optional"
            | "__thaw_array_shift_optional" => {
                let [receiver] = args else {
                    return Err("array pop/shift expects one operand".to_string());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(receiver) else {
                    return Err("array pop/shift requires a homogeneous array".to_string());
                };
                let i64_type = self.context.i64_type();
                let width = array_element_storage_bytes(&element)?;
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
                let presence = self.compile_array_presence(handle)?;
                let is_shift = matches!(name, "__thaw_array_shift" | "__thaw_array_shift_optional");
                let removed_index = if is_shift {
                    i64_type.const_zero()
                } else {
                    self.builder
                        .build_int_sub(
                            old_length,
                            i64_type.const_int(1, false),
                            "array_remove_index",
                        )
                        .map_err(|error| error.to_string())?
                };
                let index_state = self.compile_array_index_state(handle, removed_index)?;
                let index_has_value = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        index_state,
                        self.context.i8_type().const_int(1, false),
                        "array_remove_has_payload",
                    )
                    .map_err(|error| error.to_string())?;
                let not_empty = self
                    .builder
                    .build_not(is_empty, "array_remove_not_empty")
                    .map_err(|error| error.to_string())?;
                let has_removed_value = self
                    .builder
                    .build_and(
                        not_empty,
                        index_has_value,
                        "array_remove_has_value",
                    )
                    .map_err(|error| error.to_string())?;
                let element_llvm_type = self.basic_type(&element)?;
                let out_slot = self
                    .builder
                    .build_alloca(element_llvm_type, "array_remove_value")
                    .map_err(|error| error.to_string())?;
                let runtime = if is_shift {
                    "thaw_array_shift"
                } else {
                    "thaw_array_pop"
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
                let new_presence = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_presence_remove").unwrap(),
                        &[
                            presence.into(),
                            old_length.into(),
                            self.context.i8_type().const_int(u64::from(is_shift), false).into(),
                        ],
                        "array_remove_presence",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array presence remove returned no value")?
                    .into_pointer_value();
                self.compile_array_set_presence(handle, new_presence)?;
                if matches!(name, "__thaw_array_pop" | "__thaw_array_shift") {
                    return self
                        .builder
                        .build_load(element_llvm_type, out_slot, "array_removed_value")
                        .map_err(|error| error.to_string());
                }
                let function = self.current_function();
                let absent_block = self.context.append_basic_block(function, "array_remove_absent");
                let present_block = self
                    .context
                    .append_basic_block(function, "array_remove_present");
                let merge_block = self.context.append_basic_block(function, "array_remove_merge");
                self.builder
                    .build_conditional_branch(has_removed_value, present_block, absent_block)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(absent_block);
                let absent_value = match element.as_ref() {
                    HirType::Optional(payload) => self.compile_optional_none(payload)?,
                    HirType::Nullish(payload) => {
                        self.compile_expr(&HirExpr::NullishUndefined(payload.as_ref().clone()))?
                    }
                    HirType::Nullable(payload) => {
                        self.compile_expr(&HirExpr::NullishUndefined(payload.as_ref().clone()))?
                    }
                    HirType::Undefined => self.compile_expr(&HirExpr::Lit(HirLit::Undefined))?,
                    HirType::Union(members) => {
                        let mut members = members.clone();
                        if !members.contains(&HirType::Undefined) {
                            members.push(HirType::Undefined);
                        }
                        let index = members.iter().position(|ty| ty == &HirType::Undefined).unwrap();
                        self.compile_expr(&HirExpr::UnionInject(
                            Box::new(HirExpr::Lit(HirLit::Undefined)), index, members,
                        ))?
                    }
                    _ => {
                        let zero_value = self.compile_zero_value(&element)?;
                        self.build_optional_value(zero_value, &element, false)?
                    }
                };
                let absent_block = self.builder.get_insert_block().unwrap();
                self.builder
                    .build_unconditional_branch(merge_block)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(present_block);
                let removed_value = self
                    .builder
                    .build_load(element_llvm_type, out_slot, "array_removed_value")
                    .map_err(|error| error.to_string())?;
                let present_value = if let HirType::Nullable(payload) = element.as_ref() {
                    let nullable = removed_value.into_struct_value();
                    let has_value = self.builder.build_extract_value(nullable, 0, "removed_nullable_present")
                        .map_err(|error| error.to_string())?.into_int_value();
                    let value = self.builder.build_extract_value(nullable, 1, "removed_nullable_value")
                        .map_err(|error| error.to_string())?;
                    let tag = self.builder.build_select(
                        has_value,
                        self.context.i8_type().const_zero(),
                        self.context.i8_type().const_int(1, false),
                        "removed_nullable_tag",
                    ).map_err(|error| error.to_string())?.into_int_value();
                    self.build_nullish_tagged_value(value, payload, tag)?
                } else if matches!(element.as_ref(), HirType::Optional(_) | HirType::Nullish(_) | HirType::Undefined | HirType::Union(_)) {
                    removed_value
                } else {
                    self.build_optional_value(removed_value, &element, true)?
                };
                let present_block = self.builder.get_insert_block().unwrap();
                self.builder
                    .build_unconditional_branch(merge_block)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(merge_block);
                let phi = self
                    .builder
                    .build_phi(absent_value.get_type(), "array_remove_result")
                    .map_err(|error| error.to_string())?;
                phi.add_incoming(&[
                    (&absent_value, absent_block),
                    (&present_value, present_block),
                ]);
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
                let width = array_element_storage_bytes(&element)?;
                let handle = self.compile_expr(&args[0])?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let old_len = self
                    .builder
                    .build_load(i64_type, buffer, "array_splice_old_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let start = self.compile_expr(&args[1])?;
                let delete_count = self.compile_expr(&args[2])?;
                let items = &args[3..];
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let (values_ptr, states_ptr) = self.compile_array_insert_values(
                    items, &element, width, "array_splice",
                )?;
                let out_removed = self
                    .builder
                    .build_alloca(ptr_type, "array_splice_removed")
                    .map_err(|error| error.to_string())?;
                let out_removed_presence = self
                    .builder
                    .build_alloca(ptr_type, "array_splice_removed_presence")
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
                let presence = self.compile_array_presence(handle)?;
                let new_presence = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_array_presence_splice")
                            .unwrap(),
                        &[
                            presence.into(),
                            old_len.into(),
                            start.into(),
                            delete_count.into(),
                            i64_type.const_int(items.len() as u64, false).into(),
                            states_ptr.into(),
                            out_removed_presence.into(),
                        ],
                        "array_splice_presence",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array presence splice returned no value".to_string())?
                    .into_pointer_value();
                self.compile_array_set_presence(handle, new_presence)?;
                let removed = self
                    .builder
                    .build_load(ptr_type, out_removed, "array_splice_removed_buffer")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                let removed_presence = self
                    .builder
                    .build_load(
                        ptr_type,
                        out_removed_presence,
                        "array_splice_removed_presence_value",
                    )
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                return Ok(self
                    .compile_array_wrap_with_presence(removed, removed_presence)?
                    .into());
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
                let old_len = self
                    .builder
                    .build_load(self.context.i64_type(), buffer, "array_sort_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let presence = self.compile_array_presence(handle)?;
                let present_len = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_array_presence_compact")
                            .unwrap(),
                        &[
                            buffer.into(),
                            presence.into(),
                            self.context.i64_type().const_int(8, false).into(),
                        ],
                        "array_sort_present_length",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array presence compaction returned no value".to_string())?
                    .into_int_value();
                self.builder
                    .build_store(buffer, present_len)
                    .map_err(|error| error.to_string())?;
                // Mutates in place and returns the same pointer -- keep
                // using the original handle as this expression's value.
                self.builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[buffer.into()],
                        "array_sort",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(buffer, old_len)
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
                let runtime = format!(
                    "thaw_{}",
                    name.trim_start_matches("__thaw_")
                        .trim_end_matches("_to_sorted")
                        .to_string()
                        + "_sort"
                );
                let handle = self.compile_expr(array)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let old_len = self
                    .builder
                    .build_load(self.context.i64_type(), buffer, "array_to_sorted_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_slice").unwrap(),
                        &[
                            buffer.into(),
                            self.context.i64_type().const_int(8, false).into(),
                            self.context.f64_type().const_zero().into(),
                            self.context.f64_type().const_float(f64::INFINITY).into(),
                        ],
                        "array_to_sorted_copy",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array toSorted returned no value".to_string())?
                    .into_pointer_value();
                let presence = self.compile_array_presence(handle)?;
                let copied_presence = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_array_presence_slice")
                            .unwrap(),
                        &[
                            presence.into(),
                            old_len.into(),
                            self.context.f64_type().const_zero().into(),
                            self.context.f64_type().const_float(f64::INFINITY).into(),
                        ],
                        "array_to_sorted_presence",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array presence slice returned no value".to_string())?
                    .into_pointer_value();
                let present_len = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_array_presence_compact")
                            .unwrap(),
                        &[
                            result.into(),
                            copied_presence.into(),
                            self.context.i64_type().const_int(8, false).into(),
                        ],
                        "array_to_sorted_present_length",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array presence compaction returned no value".to_string())?
                    .into_int_value();
                self.builder
                    .build_store(result, present_len)
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[result.into()],
                        "array_to_sorted",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(result, old_len)
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function("thaw_array_presence_densify").unwrap(),
                        &[copied_presence.into()],
                        "array_to_sorted_presence_densify",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(self
                    .compile_array_wrap_with_presence(result, copied_presence)?
                    .into());
            }
            "__thaw_any_array_flat" => {
                let [array, depth] = args else {
                    return Err("array flat expects two operands".to_string());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let presence = self.compile_array_presence(handle)?;
                let depth = self.compile_expr(depth)?.into_float_value();
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_any_array_flat").unwrap(),
                        &[buffer.into(), presence.into(), depth.into()],
                        "array_flat",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_any_array_flat returned no value".to_string())?
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
            | "__thaw_any_array_index_of"
            | "__thaw_any_array_includes"
            | "__thaw_number_array_last_index_of"
            | "__thaw_string_array_last_index_of"
            | "__thaw_bool_array_last_index_of"
            | "__thaw_object_array_last_index_of"
            | "__thaw_any_array_last_index_of" => {
                let runtime = format!("thaw_{}", name.trim_start_matches("__thaw_"));
                if args.len() != 3 {
                    return Err("array search expects three operands".to_string());
                }
                let mut arguments = Vec::with_capacity(4);
                for (index, argument) in args.iter().enumerate() {
                    let mut value = self.compile_expr(argument)?;
                    if index == 0 {
                        let handle = value.into_pointer_value();
                        value = self.compile_array_data(handle)?.into();
                        arguments.push(value.into());
                        arguments.push(self.compile_array_presence(handle)?.into());
                        continue;
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
                let result = if name.starts_with("__thaw_any_array_") {
                    self.compile_check_json_host_error(result, None)?
                } else {
                    result
                };
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
